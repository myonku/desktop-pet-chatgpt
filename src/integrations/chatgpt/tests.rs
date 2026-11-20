use super::*;

const INITIALIZED: &str = r#"{"id":0,"result":{"userAgent":"test-server"}}"#;

fn query(response: &str) -> Result<AccountPlan, QueryError> {
    exchange_account(
        &mut Vec::new(),
        &mut format!("{INITIALIZED}\n{response}").as_bytes(),
    )
}

#[test]
fn reads_only_the_plan_after_handshake() {
    let server = concat!(
        r#"{"method":"account/updated","params":{"planType":"free"}}"#,
        "\n",
        r#"{"id":0,"result":{"userAgent":"test-server"}}"#,
        "\n",
        r#"{"id":1,"result":{"account":{"type":"chatgpt","planType":"plus","email":"ignored@example.test","workspaceName":"ignored"},"requiresOpenaiAuth":true}}"#,
        "\n",
    );
    let mut requests = Vec::new();
    let plan = exchange_account(&mut requests, &mut server.as_bytes()).unwrap();
    assert_eq!(plan, AccountPlan::Available { plan: "plus" });
    let requests: Vec<Value> = requests
        .split_inclusive(|&byte| byte == b'\n')
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0]["method"], "initialize");
    assert_eq!(requests[1], json!({"method":"initialized"}));
    assert_eq!(
        requests[2],
        json!({"id":1,"method":"account/read","params":{"refreshToken":false}}),
    );
    assert_eq!(json!(plan), json!({"status":"available","plan":"plus"}));
}

#[test]
fn reads_the_named_codex_bucket_and_preserves_window_units() {
    let mut requests = Vec::new();
    let response = json!({
        "id": 2,
        "result": {
            "rateLimits": { "limitId": "other", "primary": { "usedPercent": 99 } },
            "rateLimitsByLimitId": {
                "codex": { "limitId": null, "primary": {
                    "usedPercent": 0, "windowDurationMins": 300, "resetsAt": 1800000000
                }, "secondary": { "usedPercent": 60, "windowDurationMins": 10080 } },
                "other": { "primary": { "usedPercent": 99 } }
            }
        }
    });
    let usage = exchange_usage(&mut requests, &mut format!("{response}\n").as_bytes()).unwrap();
    assert_eq!(
        usage,
        CodexUsage::Available {
            primary: Some(UsageWindow {
                used_percent: 0,
                window_duration_mins: Some(300),
                resets_at: Some(1_800_000_000),
            }),
            secondary: Some(UsageWindow {
                used_percent: 60,
                window_duration_mins: Some(10_080),
                resets_at: None,
            }),
        }
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&requests).unwrap(),
        json!({"id":2,"method":"account/rateLimits/read"}),
    );
}

#[test]
fn requires_an_identified_codex_bucket_and_handles_missing_windows() {
    let usage = |result| {
        let response = json!({"id":2,"result":result});
        exchange_usage(&mut Vec::new(), &mut format!("{response}\n").as_bytes())
    };
    for legacy in [
        json!({"primary":{"usedPercent":30}}),
        json!({"limitId":"other","primary":{"usedPercent":30}}),
        json!({"limitId":"codex"}),
    ] {
        assert_eq!(
            usage(json!({"rateLimits":legacy})),
            Ok(CodexUsage::Unavailable)
        );
    }
    assert_eq!(
        usage(json!({"rateLimits":{"limitId":"codex","secondary":{"usedPercent":30}}})),
        Ok(CodexUsage::Available {
            primary: None,
            secondary: Some(UsageWindow {
                used_percent: 30,
                window_duration_mins: None,
                resets_at: None,
            }),
        }),
    );
    assert_eq!(
        usage(json!({"rateLimits":{},"rateLimitsByLimitId":{"codex":{"limitId":"other"}}})),
        Err(QueryError::InvalidResponse),
    );
    assert_eq!(
        usage(json!({"rateLimits":{"limitId":"codex","primary":{"usedPercent":-1}}})),
        Err(QueryError::InvalidResponse),
    );
}

#[test]
fn keeps_the_plan_when_the_quota_request_times_out() {
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        r#"
            [void][Console]::ReadLine()
            [Console]::WriteLine('{"id":0,"result":{"userAgent":"test-server"}}')
            [void][Console]::ReadLine()
            [void][Console]::ReadLine()
            [Console]::WriteLine('{"id":1,"result":{"account":{"type":"chatgpt","planType":"plus"},"requiresOpenaiAuth":true}}')
            [void][Console]::ReadLine()
            Start-Sleep -Seconds 30
        "#,
    ]);
    let snapshot = run_query(
        &mut command,
        Duration::from_secs(3),
        Duration::from_millis(500),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(snapshot.plan, AccountPlan::Available { plan: "plus" });
    assert_eq!(
        snapshot.usage,
        Some(CodexUsage::Failed {
            error: QueryError::TimedOut
        })
    );
}

#[test]
fn distinguishes_auth_other_accounts_and_unknown_plans() {
    for (account, expected) in [
        (Value::Null, AccountPlan::NeedsAuth),
        (
            json!({"type":"apiKey","planType":"plus"}),
            AccountPlan::UnsupportedAccount,
        ),
        (json!({"type":"chatgpt"}), AccountPlan::UnknownPlan),
        (
            json!({"type":"chatgpt","planType":"unknown"}),
            AccountPlan::UnknownPlan,
        ),
        (
            json!({"type":"chatgpt","planType":"unrecognized"}),
            AccountPlan::UnknownPlan,
        ),
    ] {
        let response = json!({"id":1,"result":{"account":account,"requiresOpenaiAuth":true}});
        assert_eq!(query(&format!("{response}\n")), Ok(expected));
    }
}

#[test]
fn refuses_missing_mismatched_and_oversized_replies() {
    assert_eq!(query(""), Err(QueryError::Disconnected));
    for response in [
        r#"{"id":2,"result":{"account":null,"requiresOpenaiAuth":true}}"#,
        r#"{"id":1,"result":{}}"#,
        r#"{"id":1,"method":"account/chatgptAuthTokens/refresh"}"#,
        "broken json",
    ] {
        assert_eq!(
            query(&format!("{response}\n")),
            Err(QueryError::InvalidResponse)
        );
    }
    assert_eq!(
        query(&" ".repeat(MAX_LINE_BYTES + 1)),
        Err(QueryError::InvalidResponse)
    );
    assert_eq!(
        query("{\"id\":1,\"error\":{\"message\":\"discarded\"}}\n"),
        Err(QueryError::ServerError),
    );
}

#[test]
fn terminates_a_nonresponsive_child() {
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Start-Sleep -Seconds 30",
    ]);
    let started = Instant::now();
    assert_eq!(
        run_query(
            &mut command,
            Duration::from_millis(250),
            Duration::ZERO,
            &AtomicBool::new(false)
        )
        .unwrap_err(),
        QueryError::TimedOut,
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}
