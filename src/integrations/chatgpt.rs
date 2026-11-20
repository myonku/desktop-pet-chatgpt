//! Read the account and Codex usage associated with a selected local App Server.

use std::{
    io::{BufRead, BufReader, Read, Write},
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use serde::{Deserialize, Serialize, de::IgnoredAny};
use serde_json::{Value, json};
use windows::{
    Win32::{
        Foundation::HANDLE,
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            },
            Threading::CREATE_NO_WINDOW,
        },
    },
    core::PCWSTR,
};

pub const ACCOUNT_SOURCE: &str = "ChatGPT account via Codex App Server";
const ACCOUNT_TIMEOUT: Duration = Duration::from_secs(5);
const USAGE_TIMEOUT: Duration = Duration::from_secs(8);
const EXIT_GRACE: Duration = Duration::from_millis(200);
const MAX_LINE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AccountPlan {
    Available { plan: &'static str },
    NeedsAuth,
    UnsupportedAccount,
    UnknownPlan,
}

#[derive(Debug, Serialize)]
pub struct AccountSnapshot {
    pub source: &'static str,
    pub observed_at: SystemTime,
    #[serde(flatten)]
    pub plan: AccountPlan,
    pub usage: Option<CodexUsage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CodexUsage {
    Available {
        primary: Option<UsageWindow>,
        secondary: Option<UsageWindow>,
    },
    Unavailable,
    Failed {
        error: QueryError,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub used_percent: i32,
    pub window_duration_mins: Option<i64>,
    pub resets_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryError {
    InvalidExecutable,
    LaunchFailed,
    Disconnected,
    TimedOut,
    InvalidResponse,
    ServerError,
    CleanupFailed,
    Cancelled,
}

/// Run one blocking query on an explicitly selected, trusted Windows executable.
/// Call from a worker thread when used by a UI. Each query owns its subprocess.
pub fn read_account(cli: &Path) -> Result<AccountSnapshot, QueryError> {
    read_account_cancellable(cli, &AtomicBool::new(false))
}

pub(crate) fn read_account_cancellable(
    cli: &Path,
    cancelled: &AtomicBool,
) -> Result<AccountSnapshot, QueryError> {
    if !cli.is_absolute()
        || !cli
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return Err(QueryError::InvalidExecutable);
    }
    let mut command = Command::new(cli);
    command.args(["app-server", "--listen", "stdio://"]);
    run_query(&mut command, ACCOUNT_TIMEOUT, USAGE_TIMEOUT, cancelled)
}

fn run_query(
    command: &mut Command,
    account_timeout: Duration,
    usage_timeout: Duration,
    cancelled: &AtomicBool,
) -> Result<AccountSnapshot, QueryError> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(QueryError::Cancelled);
    }
    let mut deadline = Instant::now() + account_timeout;
    let job = server_job().map_err(|_| QueryError::LaunchFailed)?;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()
        .map_err(|_| QueryError::LaunchFailed)?;
    // The job is private to this request. Closing its handle, including on process
    // exit, terminates the owned server and any of its children.
    if unsafe {
        AssignProcessToJobObject(HANDLE(job.as_raw_handle()), HANDLE(child.as_raw_handle()))
    }
    .is_err()
    {
        close_server(&mut child, Instant::now())?;
        return Err(QueryError::LaunchFailed);
    }
    let input = child.stdin.take().expect("piped stdin");
    let output = child.stdout.take().expect("piped stdout");
    let (account_tx, account_rx) = mpsc::sync_channel(1);
    let (usage_tx, usage_rx) = mpsc::sync_channel(1);
    let worker = thread::Builder::new()
        .name("chatgpt-account-io".into())
        .spawn(move || {
            let mut input = input;
            let mut output = BufReader::new(output);
            let account = exchange_account(&mut input, &mut output);
            let read_usage = matches!(
                account,
                Ok(AccountPlan::Available { .. } | AccountPlan::UnknownPlan)
            );
            if account_tx.send(account).is_ok() && read_usage {
                let _ = usage_tx.send(exchange_usage(&mut input, &mut output));
            }
        });
    let result = worker.map_err(|_| QueryError::LaunchFailed).and_then(|_| {
        let plan = receive_reply(&account_rx, deadline, cancelled)?;
        let usage = match plan {
            AccountPlan::Available { .. } | AccountPlan::UnknownPlan => {
                deadline = Instant::now() + usage_timeout;
                Some(match receive_reply(&usage_rx, deadline, cancelled) {
                    Ok(usage) => usage,
                    Err(QueryError::Cancelled) => return Err(QueryError::Cancelled),
                    Err(error) => CodexUsage::Failed { error },
                })
            }
            _ => None,
        };
        Ok(AccountSnapshot {
            source: ACCOUNT_SOURCE,
            observed_at: SystemTime::now(),
            plan,
            usage,
        })
    });
    let exit_by = if matches!(result, Err(QueryError::Cancelled)) {
        Instant::now()
    } else {
        deadline
    };
    close_server(&mut child, exit_by)?;
    result
}

fn receive_reply<T>(
    receiver: &mpsc::Receiver<Result<T, QueryError>>,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<T, QueryError> {
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err(QueryError::Cancelled);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(QueryError::TimedOut);
        }
        match receiver.recv_timeout(remaining.min(Duration::from_millis(50))) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(QueryError::Disconnected),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn server_job() -> windows::core::Result<OwnedHandle> {
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // The job handle is non-inheritable and owned by this process.
    unsafe {
        let job = OwnedHandle::from_raw_handle(CreateJobObjectW(None, PCWSTR::null())?.0);
        SetInformationJobObject(
            HANDLE(job.as_raw_handle()),
            JobObjectExtendedLimitInformation,
            std::ptr::from_ref(&info).cast(),
            std::mem::size_of_val(&info) as u32,
        )?;
        Ok(job)
    }
}

fn close_server(child: &mut Child, deadline: Instant) -> Result<(), QueryError> {
    // The I/O worker closes stdin after the exchange. Allow the server to exit on EOF
    // within the original deadline, then terminate only the process we started.
    let exit_by = deadline.min(Instant::now() + EXIT_GRACE);
    loop {
        if child
            .try_wait()
            .map_err(|_| QueryError::CleanupFailed)?
            .is_some()
        {
            return Ok(());
        }
        let remaining = exit_by.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        thread::sleep(remaining.min(Duration::from_millis(10)));
    }
    if child.kill().is_err()
        && child
            .try_wait()
            .map_err(|_| QueryError::CleanupFailed)?
            .is_none()
    {
        return Err(QueryError::CleanupFailed);
    }
    child.wait().map_err(|_| QueryError::CleanupFailed)?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitializeResult {
    user_agent: String,
}

#[derive(Deserialize)]
struct AccountResult {
    account: Option<Account>,
    // This flag describes the selected provider, not whether there is a ChatGPT account.
    #[serde(rename = "requiresOpenaiAuth")]
    _requires_openai_auth: bool,
}

#[derive(Deserialize)]
struct Account {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "planType")]
    plan: Option<String>,
}

fn exchange_account(
    input: &mut impl Write,
    output: &mut impl BufRead,
) -> Result<AccountPlan, QueryError> {
    write_message(
        input,
        json!({
            "id": 0, "method": "initialize",
            "params": {"clientInfo": {
                "name": "desktop_pet_chatgpt",
                "title": "Desktop Pet ChatGPT",
                "version": env!("CARGO_PKG_VERSION")
            }}
        }),
    )?;
    let initialized: InitializeResult = read_response(output, 0)?;
    if initialized.user_agent.is_empty() {
        return Err(QueryError::InvalidResponse);
    }
    write_message(input, json!({"method": "initialized"}))?;
    write_message(
        input,
        json!({"id": 1, "method": "account/read", "params": {"refreshToken": false}}),
    )?;
    let result: AccountResult = read_response(output, 1)?;
    Ok(match result.account {
        None => AccountPlan::NeedsAuth,
        Some(account) if account.kind != "chatgpt" => AccountPlan::UnsupportedAccount,
        Some(account) => known_plan(account.plan.as_deref())
            .map_or(AccountPlan::UnknownPlan, |plan| AccountPlan::Available {
                plan,
            }),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageResult {
    rate_limits: RateLimitBucket,
    rate_limits_by_limit_id: Option<UsageBuckets>,
}

#[derive(Deserialize)]
struct UsageBuckets {
    codex: Option<RateLimitBucket>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RateLimitBucket {
    limit_id: Option<String>,
    primary: Option<UsageWindow>,
    secondary: Option<UsageWindow>,
}

fn exchange_usage(
    input: &mut impl Write,
    output: &mut impl BufRead,
) -> Result<CodexUsage, QueryError> {
    write_message(input, json!({"id": 2, "method": "account/rateLimits/read"}))?;
    let result: UsageResult = read_response(output, 2)?;
    let bucket = result
        .rate_limits_by_limit_id
        .and_then(|buckets| buckets.codex)
        .or_else(|| {
            (result.rate_limits.limit_id.as_deref() == Some("codex")).then_some(result.rate_limits)
        });
    let Some(bucket) = bucket else {
        return Ok(CodexUsage::Unavailable);
    };
    if bucket.limit_id.as_deref().is_some_and(|id| id != "codex") {
        return Err(QueryError::InvalidResponse);
    }
    let normalize = |mut window: UsageWindow| {
        window.window_duration_mins = window.window_duration_mins.filter(|&minutes| minutes > 0);
        window.resets_at = window.resets_at.filter(|&timestamp| timestamp > 0);
        window
    };
    if bucket.primary.is_none() && bucket.secondary.is_none() {
        return Ok(CodexUsage::Unavailable);
    }
    if [&bucket.primary, &bucket.secondary]
        .into_iter()
        .flatten()
        .any(|window| window.used_percent < 0)
    {
        return Err(QueryError::InvalidResponse);
    }
    Ok(CodexUsage::Available {
        primary: bucket.primary.map(normalize),
        secondary: bucket.secondary.map(normalize),
    })
}

fn write_message(input: &mut impl Write, message: Value) -> Result<(), QueryError> {
    writeln!(input, "{message}").map_err(|_| QueryError::Disconnected)
}

#[derive(Deserialize)]
struct Message<T> {
    id: Option<u64>,
    method: Option<String>,
    result: Option<T>,
    error: Option<IgnoredAny>,
}

fn read_response<T: serde::de::DeserializeOwned>(
    output: &mut impl BufRead,
    id: u64,
) -> Result<T, QueryError> {
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = (&mut *output)
            .take((MAX_LINE_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|_| QueryError::Disconnected)?;
        if read == 0 {
            return Err(QueryError::Disconnected);
        }
        if read > MAX_LINE_BYTES || line.last() != Some(&b'\n') {
            return Err(QueryError::InvalidResponse);
        }
        let message: Message<T> =
            serde_json::from_slice(&line).map_err(|_| QueryError::InvalidResponse)?;
        if message.id.is_none()
            && message.method.is_some()
            && message.result.is_none()
            && message.error.is_none()
        {
            continue;
        }
        if message.id != Some(id) || message.method.is_some() {
            return Err(QueryError::InvalidResponse);
        }
        return match (message.result, message.error) {
            (Some(result), None) => Ok(result),
            (None, Some(_)) => Err(QueryError::ServerError),
            _ => Err(QueryError::InvalidResponse),
        };
    }
}

fn known_plan(plan: Option<&str>) -> Option<&'static str> {
    // Current App Server PlanType values. Keep unknown values out of user-facing data.
    const PLANS: &[&str] = &[
        "free",
        "go",
        "plus",
        "pro",
        "prolite",
        "promax",
        "team",
        "self_serve_business_prolite",
        "self_serve_business_usage_based",
        "business",
        "ent26",
        "enterprise_cbp_automation",
        "enterprise_cbp_usage_based",
        "enterprise",
        "edu",
        "edu_plus",
        "edu_pro",
        "sci",
        "sci_plus",
    ];
    PLANS
        .iter()
        .copied()
        .find(|candidate| Some(*candidate) == plan)
}

#[cfg(test)]
mod tests;
