use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use windows::Win32::{
    Foundation::{FILETIME, SYSTEMTIME},
    System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTimeEx},
};
use winit::event_loop::EventLoopProxy;

use crate::{
    integrations::chatgpt::{
        AccountPlan, AccountSnapshot, CodexUsage, QueryError, read_account_cancellable,
    },
    platform::windows::chatgpt::{Source, discover},
};

use super::AppEvent;

const DISCOVERY_INTERVAL: Duration = Duration::from_secs(15);
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);
const SNAPSHOT_TTL: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum ClientId {
    ChatGpt,
    #[cfg(any(test, feature = "window-diagnostics"))]
    Fixture,
}

impl ClientId {
    pub fn name(self) -> &'static str {
        match self {
            Self::ChatGpt => "ChatGPT",
            #[cfg(any(test, feature = "window-diagnostics"))]
            Self::Fixture => "合成客户端",
        }
    }

    pub fn source_label(self) -> &'static str {
        match self {
            Self::ChatGpt => "来源：Codex App Server 所用 ChatGPT 账户",
            #[cfg(any(test, feature = "window-diagnostics"))]
            Self::Fixture => "来源：本地测试夹具",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SourceRoute {
    pub client: ClientId,
    epoch: u64,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct ClientView {
    pub route: SourceRoute,
    pub enabled: bool,
    pub connected: bool,
    pub desktop: Option<String>,
    pub version: Option<String>,
    pub status: String,
    pub plan: Option<String>,
    pub usage: Option<CodexUsage>,
    pub observed_at: Option<SystemTime>,
    pub stale: bool,
    pub refreshing: bool,
    pub can_refresh: bool,
    pub can_clear: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Request {
    route: SourceRoute,
    id: u64,
}

pub(crate) struct Event {
    request: Request,
    observation: Option<Observation>,
    refresh: Option<RefreshResult>,
}

struct RefreshResult {
    source: ObservedSource,
    result: Result<AccountSnapshot, QueryError>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ObservedSource {
    ChatGpt(Source),
    #[cfg(any(test, feature = "window-diagnostics"))]
    Fixture(u64),
}

struct Observation {
    version: Option<String>,
    installed: bool,
    source: Option<ObservedSource>,
}

#[derive(Clone)]
enum Adapter {
    ChatGpt,
    #[cfg(any(test, feature = "window-diagnostics"))]
    Fixture(std::path::PathBuf),
}

impl Adapter {
    fn discover(&self) -> Option<Observation> {
        match self {
            Self::ChatGpt => discover().ok().map(|desktop| Observation {
                version: desktop.version,
                installed: desktop.installed,
                source: desktop.source.map(ObservedSource::ChatGpt),
            }),
            #[cfg(any(test, feature = "window-diagnostics"))]
            Self::Fixture(path) => {
                let source = std::fs::read_to_string(path)
                    .ok()
                    .and_then(|value| value.trim().parse().ok());
                Some(Observation {
                    version: source.map(|_| "fixture".into()),
                    installed: true,
                    source: source.map(ObservedSource::Fixture),
                })
            }
        }
    }

    fn interval(&self) -> Duration {
        match self {
            Self::ChatGpt => DISCOVERY_INTERVAL,
            #[cfg(any(test, feature = "window-diagnostics"))]
            Self::Fixture(_) => Duration::from_millis(500),
        }
    }
}

impl ObservedSource {
    fn read(&self, cancel: &AtomicBool) -> Result<AccountSnapshot, QueryError> {
        match self {
            Self::ChatGpt(source) => read_account_cancellable(&source.cli, cancel),
            #[cfg(any(test, feature = "window-diagnostics"))]
            Self::Fixture(_) => {
                // Complete even after cancellation to exercise late response rejection.
                thread::sleep(Duration::from_millis(1200));
                Ok(AccountSnapshot {
                    source: "local fixture",
                    plan: AccountPlan::Available { plan: "fixture" },
                    observed_at: SystemTime::now(),
                    usage: None,
                })
            }
        }
    }
}

struct Cached {
    plan: AccountPlan,
    usage: Option<CodexUsage>,
    observed_at: SystemTime,
    fresh_until: Option<Instant>,
}

struct Active {
    request: Request,
    cancel: Arc<AtomicBool>,
    refreshing: bool,
}

pub(super) struct Client {
    adapter: Adapter,
    route: SourceRoute,
    enabled: bool,
    request_id: u64,
    next_scan: Option<Instant>,
    next_refresh: Option<Instant>,
    observation: Option<Observation>,
    scan_failed: bool,
    active: Option<Active>,
    snapshot: Option<Cached>,
    error: Option<QueryError>,
}

pub(super) struct Clients {
    entries: Vec<Client>,
}

impl Clients {
    pub fn new(chatgpt: bool) -> Self {
        let entries = vec![Client::new(ClientId::ChatGpt, Adapter::ChatGpt, chatgpt)];
        #[cfg(feature = "window-diagnostics")]
        let mut entries = entries;
        #[cfg(feature = "window-diagnostics")]
        if let Some(path) = std::env::var_os("DESKTOP_PET_TEST_CLIENT_SOURCE") {
            entries.push(Client::new(
                ClientId::Fixture,
                Adapter::Fixture(path.into()),
                true,
            ));
        }
        Self { entries }
    }

    pub fn set_chatgpt_enabled(&mut self, enabled: bool) {
        self.entries[0].set_enabled(enabled);
    }

    pub fn contains(&self, id: ClientId) -> bool {
        self.entries.iter().any(|client| client.route.client == id)
    }

    pub fn get_mut(&mut self, route: SourceRoute) -> Option<&mut Client> {
        self.entries.iter_mut().find(|client| client.route == route)
    }

    pub fn complete(&mut self, event: Event, now: Instant) {
        if let Some(client) = self
            .entries
            .iter_mut()
            .find(|client| client.route.client == event.request.route.client)
        {
            client.complete(event, now);
        }
    }

    pub fn tick(
        &mut self,
        now: Instant,
        overview_open: bool,
        proxy: &EventLoopProxy<AppEvent>,
    ) -> (Option<Instant>, bool) {
        let mut deadline = None;
        let mut changed = false;
        for client in &mut self.entries {
            let (next, updated) = client.tick(now, overview_open, proxy);
            deadline = deadline.into_iter().chain(next).min();
            changed |= updated;
        }
        (deadline, changed)
    }

    pub fn views(&self, now: Instant) -> Vec<ClientView> {
        self.entries.iter().map(|client| client.view(now)).collect()
    }
}

impl Client {
    fn new(id: ClientId, adapter: Adapter, enabled: bool) -> Self {
        Self {
            adapter,
            route: SourceRoute {
                client: id,
                epoch: 0,
            },
            enabled,
            request_id: 0,
            next_scan: enabled.then(Instant::now),
            next_refresh: None,
            observation: None,
            scan_failed: false,
            active: None,
            snapshot: None,
            error: None,
        }
    }

    fn source(&self) -> Option<&ObservedSource> {
        self.observation
            .as_ref()
            .and_then(|observation| observation.source.as_ref())
    }

    fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
            self.observation = None;
            self.scan_failed = false;
            self.clear();
            self.next_scan = enabled.then(Instant::now);
            self.next_refresh = None;
        }
    }

    pub fn clear(&mut self) {
        self.route.epoch += 1;
        if let Some(active) = &self.active {
            active.cancel.store(true, Ordering::Relaxed);
        }
        self.snapshot = None;
        self.error = None;
    }

    fn scan_due(&self, now: Instant) -> bool {
        self.active.is_none() && self.next_scan.is_some_and(|at| now >= at)
    }

    fn refresh_deadline(&self, now: Instant, overview_open: bool) -> Option<Instant> {
        (overview_open && self.active.is_none() && self.source().is_some())
            .then(|| self.next_refresh.unwrap_or(now))
    }

    fn tick(
        &mut self,
        now: Instant,
        overview_open: bool,
        proxy: &EventLoopProxy<AppEvent>,
    ) -> (Option<Instant>, bool) {
        let mut started = false;
        if self
            .refresh_deadline(now, overview_open)
            .is_some_and(|at| at <= now)
        {
            self.refresh(now, proxy);
            started = true;
        } else if self.scan_due(now) {
            self.start(now, None, proxy);
            started = true;
        }
        let scan = self.next_scan.filter(|_| self.active.is_none());
        let mut expired = false;
        let expiry = self.snapshot.as_mut().and_then(|snapshot| {
            if snapshot.fresh_until.is_some_and(|at| at <= now) {
                expired = true;
                snapshot.fresh_until = None;
            }
            snapshot.fresh_until
        });
        (
            scan.into_iter()
                .chain(self.refresh_deadline(now, overview_open))
                .chain(expiry)
                .min(),
            started || expired,
        )
    }

    pub fn refresh(&mut self, now: Instant, proxy: &EventLoopProxy<AppEvent>) {
        if self.active.is_none()
            && let Some(source) = self.source().cloned()
        {
            self.start(now, Some(source), proxy);
        }
    }

    fn begin(&mut self, now: Instant, source: Option<&ObservedSource>) -> Request {
        self.request_id += 1;
        self.next_scan = Some(now + self.adapter.interval());
        if source.is_some() {
            self.next_refresh = Some(now + REFRESH_INTERVAL);
        }
        let request = Request {
            route: self.route,
            id: self.request_id,
        };
        self.active = Some(Active {
            request,
            cancel: Arc::new(AtomicBool::new(false)),
            refreshing: source.is_some(),
        });
        request
    }

    fn start(
        &mut self,
        now: Instant,
        source: Option<ObservedSource>,
        proxy: &EventLoopProxy<AppEvent>,
    ) {
        let request = self.begin(now, source.as_ref());
        let cancel = Arc::clone(&self.active.as_ref().unwrap().cancel);
        let refreshing = source.is_some();
        let adapter = self.adapter.clone();
        let proxy = proxy.clone();
        let worker = thread::Builder::new()
            .name("client-integration".into())
            .spawn(move || {
                let mut observation = adapter.discover();
                let refresh = source.and_then(|source| {
                    if observation.as_ref().and_then(|value| value.source.as_ref()) != Some(&source)
                    {
                        return None;
                    }
                    let result = source.read(&cancel);
                    observation = adapter.discover();
                    Some(RefreshResult { source, result })
                });
                let _ = proxy.send_event(AppEvent::Client(Event {
                    request,
                    observation,
                    refresh,
                }));
            });
        if worker.is_err() {
            self.active = None;
            if refreshing {
                self.error = Some(QueryError::LaunchFailed);
            } else {
                self.apply_observation(None);
            }
        }
    }

    fn complete(&mut self, event: Event, now: Instant) {
        if !self
            .active
            .as_ref()
            .is_some_and(|active| active.request == event.request)
        {
            return;
        }
        self.active = None;
        if self.route != event.request.route {
            return;
        }
        self.apply_observation(event.observation);
        if let Some(RefreshResult { source, result }) = event.refresh
            && self.route == event.request.route
            && self.source() == Some(&source)
        {
            match result {
                Ok(snapshot) => {
                    self.snapshot = Some(Cached {
                        plan: snapshot.plan,
                        usage: snapshot.usage,
                        observed_at: snapshot.observed_at,
                        fresh_until: Some(now + SNAPSHOT_TTL),
                    });
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
    }

    fn apply_observation(&mut self, observation: Option<Observation>) {
        let next_source = observation.as_ref().and_then(|value| value.source.as_ref());
        if self.source() != next_source {
            self.clear();
            self.next_refresh = None;
        }
        self.scan_failed = observation.is_none();
        self.observation = observation;
    }

    fn view(&self, now: Instant) -> ClientView {
        let enabled = self.enabled;
        let connected = self.source().is_some();
        let desktop = match &self.observation {
            Some(value) if !value.installed => Some("未检测到受支持的安装".into()),
            Some(value) => Some(value.version.as_ref().map_or_else(
                || "已安装 · 当前会话中未运行".into(),
                |version| format!("正在运行 · {version}"),
            )),
            None if self.scan_failed => Some("客户端检测失败".into()),
            _ => None,
        };
        let active = self
            .active
            .as_ref()
            .filter(|active| active.request.route == self.route);
        let refreshing = active.is_some_and(|active| active.refreshing);
        let snapshot = self.snapshot.as_ref();
        let status = if !enabled {
            "连接未启用"
        } else if refreshing {
            "正在刷新"
        } else if let Some(error) = self.error {
            match error {
                QueryError::TimedOut => "刷新超时，请重试",
                QueryError::InvalidResponse => "查询响应不受支持",
                _ => "查询暂不可用，请重试",
            }
        } else if !connected {
            if active.is_some() {
                "正在检测本机客户端"
            } else {
                "等待桌面客户端"
            }
        } else {
            match snapshot.map(|snapshot| &snapshot.plan) {
                None => "已连接，尚未读取账户信息",
                Some(AccountPlan::NeedsAuth) => "需认证，请在客户端登录后重试",
                Some(AccountPlan::UnsupportedAccount) => "当前账户类型不受支持",
                Some(AccountPlan::UnknownPlan) => "账户已连接，套餐未知",
                Some(AccountPlan::Available { .. }) => "账户已连接",
            }
        };
        ClientView {
            route: self.route,
            enabled,
            connected,
            desktop,
            version: self
                .observation
                .as_ref()
                .and_then(|value| value.version.clone()),
            status: status.into(),
            plan: snapshot.and_then(|snapshot| match snapshot.plan {
                AccountPlan::Available { plan } => Some(plan.to_owned()),
                _ => None,
            }),
            usage: snapshot.and_then(|snapshot| snapshot.usage.clone()),
            observed_at: snapshot.map(|snapshot| snapshot.observed_at),
            stale: snapshot.is_some_and(|snapshot| {
                (matches!(snapshot.plan, AccountPlan::Available { .. })
                    || matches!(snapshot.usage, Some(CodexUsage::Available { .. })))
                    && (self.error.is_some() || snapshot.fresh_until.is_none_or(|at| now >= at))
            }),
            refreshing,
            can_refresh: connected && self.active.is_none(),
            can_clear: snapshot.is_some() || refreshing || self.error.is_some(),
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(active) = &self.active {
            active.cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests;

fn utc_system_time(time: SystemTime) -> Option<SYSTEMTIME> {
    // FILETIME uses 100 ns intervals since 1601. The snapshot is local acquisition time.
    let ticks = u64::try_from(time.duration_since(UNIX_EPOCH).ok()?.as_nanos() / 100).ok()?;
    let ticks = ticks.checked_add(116_444_736_000_000_000)?;
    let file_time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    unsafe { FileTimeToSystemTime(&file_time, &mut utc).ok()? };
    Some(utc)
}

pub(super) fn utc_label(time: SystemTime) -> Option<String> {
    let utc = utc_system_time(time)?;
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        utc.wYear, utc.wMonth, utc.wDay, utc.wHour, utc.wMinute, utc.wSecond
    ))
}

pub(super) fn local_reset_label(epoch_seconds: i64) -> Option<String> {
    let time = UNIX_EPOCH.checked_add(Duration::from_secs(epoch_seconds.try_into().ok()?))?;
    let local = local_system_time(time)?;
    Some(format!(
        "{:02}-{:02} {:02}:{:02}",
        local.wMonth, local.wDay, local.wHour, local.wMinute
    ))
}

pub(super) fn local_clock_label(time: SystemTime) -> Option<String> {
    let local = local_system_time(time)?;
    Some(format!(
        "{:02}:{:02}:{:02}",
        local.wHour, local.wMinute, local.wSecond
    ))
}

fn local_system_time(time: SystemTime) -> Option<SYSTEMTIME> {
    let utc = utc_system_time(time)?;
    let mut local = SYSTEMTIME::default();
    unsafe { SystemTimeToTzSpecificLocalTimeEx(None, &utc, &mut local).ok()? };
    Some(local)
}
