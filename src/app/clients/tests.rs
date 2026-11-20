use super::*;

fn source(generation: u64) -> ObservedSource {
    ObservedSource::ChatGpt(Source::new("codex.exe".into(), 1, generation))
}

fn observed(source: ObservedSource) -> Option<Observation> {
    Some(Observation {
        installed: true,
        version: Some("test".into()),
        source: Some(source),
    })
}

fn request(
    client: &mut Client,
    source: ObservedSource,
    result: Result<AccountPlan, QueryError>,
) -> Event {
    let usage = matches!(source, ObservedSource::ChatGpt(_)).then_some(CodexUsage::Available {
        primary: Some(crate::integrations::chatgpt::UsageWindow {
            used_percent: 25,
            window_duration_mins: Some(300),
            resets_at: None,
        }),
        secondary: None,
    });
    Event {
        request: client.begin(Instant::now(), Some(&source)),
        observation: observed(source.clone()),
        refresh: Some(RefreshResult {
            source,
            result: result.map(|plan| AccountSnapshot {
                source: "test",
                observed_at: UNIX_EPOCH,
                usage: if matches!(
                    plan,
                    AccountPlan::Available { .. } | AccountPlan::UnknownPlan
                ) {
                    usage
                } else {
                    None
                },
                plan,
            }),
        }),
    }
}

#[test]
fn expiry_and_errors_keep_only_the_current_sources_last_value() {
    let mut client = Client::new(ClientId::ChatGpt, Adapter::ChatGpt, true);
    let desktop = source(1);
    client.apply_observation(observed(desktop.clone()));
    let now = Instant::now();
    let event = request(
        &mut client,
        desktop.clone(),
        Ok(AccountPlan::Available { plan: "plus" }),
    );
    client.complete(event, now);
    assert_eq!(client.view(now).plan.as_deref(), Some("plus"));
    assert!(matches!(
        client.view(now).usage,
        Some(CodexUsage::Available { .. })
    ));
    assert!(!client.view(now).stale);
    assert!(client.view(now + SNAPSHOT_TTL).stale);

    let event = request(&mut client, desktop.clone(), Err(QueryError::TimedOut));
    client.complete(event, now);
    let view = client.view(now);
    assert!(view.stale);
    assert_eq!(view.plan.as_deref(), Some("plus"));
    assert!(matches!(view.usage, Some(CodexUsage::Available { .. })));

    let event = request(&mut client, desktop.clone(), Ok(AccountPlan::UnknownPlan));
    client.complete(event, now);
    assert!(client.view(now).plan.is_none());
    assert!(client.view(now).status.contains("套餐未知"));

    let event = request(&mut client, desktop, Ok(AccountPlan::NeedsAuth));
    client.complete(event, now);
    let view = client.view(now);
    assert!(view.status.contains("需认证"));
    assert!(view.plan.is_none());
    assert!(view.usage.is_none());
}

#[test]
fn overview_refreshes_on_connection_and_at_the_next_interval() {
    let mut client = Client::new(ClientId::ChatGpt, Adapter::ChatGpt, false);
    assert!(!client.scan_due(Instant::now()));
    client.set_enabled(true);
    let now = Instant::now();
    assert!(client.scan_due(now));
    assert_eq!(client.refresh_deadline(now, true), None);
    let probe = client.begin(now, None);
    let desktop = source(1);
    client.complete(
        Event {
            request: probe,
            observation: observed(desktop.clone()),
            refresh: None,
        },
        now,
    );
    assert_eq!(client.refresh_deadline(now, false), None);
    assert_eq!(client.refresh_deadline(now, true), Some(now));

    let query = client.begin(now, Some(&desktop));
    assert_eq!(client.refresh_deadline(now, true), None);
    client.complete(
        Event {
            request: query,
            observation: observed(desktop.clone()),
            refresh: Some(RefreshResult {
                source: desktop,
                result: Err(QueryError::TimedOut),
            }),
        },
        now,
    );
    assert_eq!(
        client.refresh_deadline(now, true),
        Some(now + REFRESH_INTERVAL)
    );

    client.apply_observation(None);
    assert_eq!(client.refresh_deadline(now, true), None);
    assert!(client.scan_due(now + DISCOVERY_INTERVAL));
    client.apply_observation(observed(source(2)));
    assert_eq!(client.refresh_deadline(now, true), Some(now));
    client.set_enabled(false);
    assert!(!client.scan_due(now + DISCOVERY_INTERVAL));
    assert_eq!(client.refresh_deadline(now, true), None);
}

#[test]
fn refresh_and_late_results_remain_scoped_to_their_source() {
    let mut clients = Clients {
        entries: vec![
            Client::new(ClientId::ChatGpt, Adapter::ChatGpt, true),
            Client::new(ClientId::Fixture, Adapter::Fixture("unused".into()), true),
        ],
    };
    let desktop = source(1);
    let fixture = ObservedSource::Fixture(2);
    clients.entries[0].apply_observation(observed(desktop.clone()));
    clients.entries[1].apply_observation(observed(fixture.clone()));
    let old_route = clients.entries[0].route;
    let now = Instant::now();
    let chat = request(
        &mut clients.entries[0],
        desktop.clone(),
        Ok(AccountPlan::Available { plan: "plus" }),
    );
    let fake = request(
        &mut clients.entries[1],
        fixture,
        Ok(AccountPlan::Available { plan: "fixture" }),
    );
    clients.get_mut(old_route).unwrap().clear();
    assert!(clients.get_mut(old_route).is_none());
    clients.complete(chat, now);
    assert!(clients.views(now)[0].plan.is_none());
    assert!(clients.views(now)[0].usage.is_none());
    assert!(!clients.views(now)[1].can_refresh);
    clients.complete(fake, now);
    assert_eq!(clients.views(now)[1].plan.as_deref(), Some("fixture"));
    assert!(clients.views(now)[1].usage.is_none());

    let old = request(
        &mut clients.entries[0],
        desktop.clone(),
        Ok(AccountPlan::Available { plan: "pro" }),
    );
    let mut restarted = request(
        &mut clients.entries[0],
        desktop,
        Ok(AccountPlan::Available { plan: "plus" }),
    );
    clients.complete(old, now);
    assert!(!clients.views(now)[0].can_refresh);
    restarted.observation = observed(source(3));
    clients.complete(restarted, now);
    assert!(clients.views(now)[0].plan.is_none());
    assert!(clients.views(now)[0].usage.is_none());
    assert!(clients.views(now)[0].can_refresh);

    clients.set_chatgpt_enabled(false);
    assert!(!clients.views(now)[0].enabled);
    assert_eq!(clients.views(now)[1].plan.as_deref(), Some("fixture"));
}
