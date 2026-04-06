/// Integration tests for the config reload logic (apply_config_diff).
///
/// Tests cover all four diff cases:
/// - unchanged: servers with identical config are not restarted
/// - added: new servers in the new config are started
/// - removed: servers absent from the new config are stopped
/// - changed: servers with a different config are stopped and re-started
use std::collections::HashMap;

use mcp_hub::config::{HubConfig, ServerConfig};
use mcp_hub::logs::LogAggregator;
use mcp_hub::supervisor::{apply_config_diff, start_all_servers};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn make_server_config(command: &str, args: &[&str]) -> ServerConfig {
    ServerConfig {
        command: command.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        env: HashMap::new(),
        env_file: None,
        transport: "stdio".to_string(),
        cwd: None,
        health_check_interval: None,
        max_retries: None,
        restart_delay: None,
    }
}

fn make_hub_config(entries: &[(&str, ServerConfig)]) -> HubConfig {
    let mut servers = HashMap::new();
    for (name, cfg) in entries {
        servers.insert((*name).to_string(), cfg.clone());
    }
    HubConfig {
        hub: Default::default(),
        servers,
    }
}

/// Build a long-running process config that does nothing and can be stopped.
fn sleep_server() -> ServerConfig {
    make_server_config("sleep", &["9999"])
}

/// Build a different server config (different args) to trigger a "changed" diff.
fn sleep_server_alt() -> ServerConfig {
    make_server_config("sleep", &["8888"])
}

// ─────────────────────────────────────────────────────────────────────────────
// PartialEq tests (synchronous — no runtime needed)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn server_config_partial_eq() {
    let a = make_server_config("cmd", &["arg1", "arg2"]);
    let b = make_server_config("cmd", &["arg1", "arg2"]);
    assert_eq!(a, b, "Identical ServerConfigs must be equal");

    // Different command.
    let c = make_server_config("other-cmd", &["arg1", "arg2"]);
    assert_ne!(a, c, "Different command must not be equal");

    // Different args.
    let d = make_server_config("cmd", &["arg1"]);
    assert_ne!(a, d, "Different args must not be equal");

    // Different transport.
    let mut e = a.clone();
    e.transport = "http".to_string();
    assert_ne!(a, e, "Different transport must not be equal");

    // Different env.
    let mut f = a.clone();
    f.env.insert("KEY".to_string(), "VALUE".to_string());
    assert_ne!(a, f, "Different env must not be equal");

    // Different cwd.
    let mut g = a.clone();
    g.cwd = Some("/tmp".to_string());
    assert_ne!(a, g, "Different cwd must not be equal");
}

// ─────────────────────────────────────────────────────────────────────────────
// apply_config_diff tests (async — require a Tokio runtime)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn unchanged_config_no_restarts() {
    let shutdown = CancellationToken::new();
    let cfg = make_hub_config(&[("a", sleep_server())]);
    let log_agg = Arc::new(LogAggregator::new(&["a".to_string()], 100));

    let mut handles = start_all_servers(&cfg, shutdown.clone(), Arc::clone(&log_agg)).await;
    assert_eq!(handles.len(), 1, "Should have 1 handle after start");

    // Diff with an identical config — nothing should change.
    let (added, removed, changed) =
        apply_config_diff(&mut handles, &cfg, &cfg, &shutdown, &log_agg).await;

    assert_eq!(added, 0, "Unchanged config: no servers should be added");
    assert_eq!(removed, 0, "Unchanged config: no servers should be removed");
    assert_eq!(changed, 0, "Unchanged config: no servers should be changed");
    assert_eq!(
        handles.len(),
        1,
        "Handle count must remain 1 after no-op diff"
    );

    // Clean up.
    shutdown.cancel();
    mcp_hub::supervisor::stop_all_servers(handles).await;
}

#[tokio::test]
async fn add_new_server() {
    let shutdown = CancellationToken::new();
    let old_cfg = make_hub_config(&[("a", sleep_server())]);
    let new_cfg = make_hub_config(&[("a", sleep_server()), ("b", sleep_server())]);
    let log_agg = Arc::new(LogAggregator::new(&["a".to_string(), "b".to_string()], 100));

    let mut handles = start_all_servers(&old_cfg, shutdown.clone(), Arc::clone(&log_agg)).await;
    assert_eq!(handles.len(), 1);

    let (added, removed, changed) =
        apply_config_diff(&mut handles, &old_cfg, &new_cfg, &shutdown, &log_agg).await;

    assert_eq!(added, 1, "One new server should be added");
    assert_eq!(removed, 0);
    assert_eq!(changed, 0);
    assert_eq!(handles.len(), 2, "Should now have 2 handles");

    let names: Vec<&str> = handles.iter().map(|h| h.name.as_str()).collect();
    assert!(names.contains(&"a"), "Handle for 'a' must still exist");
    assert!(
        names.contains(&"b"),
        "Handle for newly added 'b' must exist"
    );

    shutdown.cancel();
    mcp_hub::supervisor::stop_all_servers(handles).await;
}

#[tokio::test]
async fn remove_server() {
    let shutdown = CancellationToken::new();
    let old_cfg = make_hub_config(&[("a", sleep_server()), ("b", sleep_server())]);
    let new_cfg = make_hub_config(&[("a", sleep_server())]);
    let log_agg = Arc::new(LogAggregator::new(&["a".to_string(), "b".to_string()], 100));

    let mut handles = start_all_servers(&old_cfg, shutdown.clone(), Arc::clone(&log_agg)).await;
    assert_eq!(handles.len(), 2);

    let (added, removed, changed) =
        apply_config_diff(&mut handles, &old_cfg, &new_cfg, &shutdown, &log_agg).await;

    assert_eq!(added, 0);
    assert_eq!(removed, 1, "One server should be removed");
    assert_eq!(changed, 0);
    assert_eq!(handles.len(), 1, "Should now have 1 handle");

    // The remaining handle must be 'a', not the removed 'b'.
    assert_eq!(handles[0].name, "a", "Only 'a' should remain");

    shutdown.cancel();
    mcp_hub::supervisor::stop_all_servers(handles).await;
}

#[tokio::test]
async fn change_server_command() {
    let shutdown = CancellationToken::new();
    let old_cfg = make_hub_config(&[("a", sleep_server())]);
    let new_cfg = make_hub_config(&[("a", sleep_server_alt())]);
    let log_agg = Arc::new(LogAggregator::new(&["a".to_string()], 100));

    let mut handles = start_all_servers(&old_cfg, shutdown.clone(), Arc::clone(&log_agg)).await;
    assert_eq!(handles.len(), 1);

    // Record the original task ID to confirm it was replaced.
    let old_task_id = handles[0].task.id();

    let (added, removed, changed) =
        apply_config_diff(&mut handles, &old_cfg, &new_cfg, &shutdown, &log_agg).await;

    assert_eq!(added, 0);
    assert_eq!(removed, 0);
    assert_eq!(changed, 1, "One server should be changed");
    assert_eq!(handles.len(), 1, "Still 1 handle after change");

    // The task must be a new instance (different ID).
    let new_task_id = handles[0].task.id();
    assert_ne!(
        old_task_id, new_task_id,
        "Changed server must have a new supervisor task"
    );

    shutdown.cancel();
    mcp_hub::supervisor::stop_all_servers(handles).await;
}

#[tokio::test]
async fn mixed_config_diff() {
    // Old: ["a", "b", "c_old"]  New: ["a", "c_new", "d"]
    // "a" → unchanged
    // "b" → removed
    // "c_old"/"c_new" share the name "c" but differ → changed
    // "d" → added
    let shutdown = CancellationToken::new();
    let old_cfg = make_hub_config(&[
        ("a", sleep_server()),
        ("b", sleep_server()),
        ("c", sleep_server()),
    ]);
    let new_cfg = make_hub_config(&[
        ("a", sleep_server()),     // unchanged
        ("c", sleep_server_alt()), // changed
        ("d", sleep_server()),     // new
    ]);
    let log_agg = Arc::new(LogAggregator::new(
        &[
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ],
        100,
    ));

    let mut handles = start_all_servers(&old_cfg, shutdown.clone(), Arc::clone(&log_agg)).await;
    assert_eq!(handles.len(), 3);

    let (added, removed, changed) =
        apply_config_diff(&mut handles, &old_cfg, &new_cfg, &shutdown, &log_agg).await;

    assert_eq!(added, 1, "Expected 1 added (d)");
    assert_eq!(removed, 1, "Expected 1 removed (b)");
    assert_eq!(changed, 1, "Expected 1 changed (c)");
    assert_eq!(handles.len(), 3, "Should still have 3 handles total");

    let names: Vec<&str> = handles.iter().map(|h| h.name.as_str()).collect();
    assert!(names.contains(&"a"), "a must remain");
    assert!(!names.contains(&"b"), "b must be removed");
    assert!(names.contains(&"c"), "c must remain (re-started)");
    assert!(names.contains(&"d"), "d must be added");

    shutdown.cancel();
    mcp_hub::supervisor::stop_all_servers(handles).await;
}
