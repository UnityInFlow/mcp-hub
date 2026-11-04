use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use mcp_hub::config::ServerConfig;
use mcp_hub::logs::LogAggregator;
use mcp_hub::supervisor::{
    compute_backoff_delay, shutdown_process, spawn_server, start_single_server,
};
use mcp_hub::types::{BackoffConfig, ProcessState};
use tokio_util::sync::CancellationToken;

// ─────────────────────────────────────────────────────────────────────────────
// Helper: build a minimal ServerConfig for testing
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

fn no_jitter_config() -> BackoffConfig {
    BackoffConfig {
        jitter_factor: 0.0,
        ..BackoffConfig::default()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Backoff delay tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_backoff_delay_increases() {
    let cfg = no_jitter_config();

    let d0 = compute_backoff_delay(0, &cfg).as_secs_f64();
    let d1 = compute_backoff_delay(1, &cfg).as_secs_f64();
    let d2 = compute_backoff_delay(2, &cfg).as_secs_f64();
    let d3 = compute_backoff_delay(3, &cfg).as_secs_f64();

    // With jitter_factor = 0.0, jitter multiplier is exactly 1.0.
    // Formula: base_delay_secs * 2^attempt
    assert!(
        (d0 - 1.0).abs() < 1e-9,
        "attempt 0 should be 1.0s, got {d0}"
    );
    assert!(
        (d1 - 2.0).abs() < 1e-9,
        "attempt 1 should be 2.0s, got {d1}"
    );
    assert!(
        (d2 - 4.0).abs() < 1e-9,
        "attempt 2 should be 4.0s, got {d2}"
    );
    assert!(
        (d3 - 8.0).abs() < 1e-9,
        "attempt 3 should be 8.0s, got {d3}"
    );

    assert!(d0 < d3, "delay should increase with attempt count");
}

#[test]
fn test_backoff_cap_at_60s() {
    let cfg = no_jitter_config();

    // At attempt 20, 2^20 * 1.0 = 1_048_576 — well above the 60s cap.
    let d20 = compute_backoff_delay(20, &cfg).as_secs_f64();
    assert!(
        d20 <= 60.0,
        "delay for attempt 20 should be capped at 60s, got {d20}"
    );

    // u32::MAX should also be safely capped.
    let d_max = compute_backoff_delay(100, &cfg).as_secs_f64();
    assert!(
        d_max <= 60.0,
        "delay for attempt 100 should be capped at 60s, got {d_max}"
    );
}

#[test]
fn test_backoff_jitter_in_range() {
    let cfg = BackoffConfig {
        jitter_factor: 0.3,
        ..BackoffConfig::default()
    };

    // At attempt 2, base = 1.0 * 2^2 = 4.0s.
    // With ±30% jitter: result ∈ [4.0 * 0.7, 4.0 * 1.3] = [2.8, 5.2].
    let low = 4.0 * 0.7;
    let high = 4.0 * 1.3;

    let samples: Vec<f64> = (0..100)
        .map(|_| compute_backoff_delay(2, &cfg).as_secs_f64())
        .collect();

    for &s in &samples {
        assert!(
            s >= low && s <= high,
            "jitter sample {s} out of range [{low}, {high}]"
        );
    }

    // Verify that jitter actually varies (not all identical).
    let first = samples[0];
    let all_identical = samples.iter().all(|&s| (s - first).abs() < 1e-12);
    assert!(
        !all_identical,
        "jitter should produce varying values, not all {first}"
    );
}

#[test]
fn test_backoff_attempt_overflow_capped() {
    let cfg = no_jitter_config();

    // u32::MAX without capping would cause 2u32.pow(u32::MAX) to panic.
    // The implementation caps at attempt.min(10) = 10, so 2^10 * 1.0 = 1024 → capped at 60.
    let result = compute_backoff_delay(u32::MAX, &cfg);
    assert!(
        result.as_secs_f64() <= 60.0,
        "u32::MAX attempt should not panic and should be capped at 60s, got {}s",
        result.as_secs_f64()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Process spawning tests
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_spawn_server_with_echo() {
    let config = make_server_config("bash", &["-c", "echo hello >&2 && sleep 300"]);
    let mut spawned = spawn_server("test-echo", &config, &HashMap::new(), None)
        .expect("spawn_server should succeed for a valid command");

    assert!(
        spawned.pid > 0,
        "spawned process should have a positive PID"
    );

    // Clean up — kill the sleep process.
    spawned.child.kill().await.expect("kill should succeed");
    spawned
        .child
        .wait()
        .await
        .expect("wait should succeed after kill");
}

#[tokio::test]
async fn test_spawn_nonexistent_command() {
    let config = make_server_config("/nonexistent/binary/path", &[]);
    let result = spawn_server("test-missing", &config, &HashMap::new(), None);

    assert!(
        result.is_err(),
        "spawn_server should fail for a nonexistent command"
    );

    let err = match result {
        Err(e) => e.to_string(),
        Ok(_) => panic!("expected an error but got Ok"),
    };
    assert!(
        err.contains("Failed to spawn") || err.contains("nonexistent"),
        "error message should describe the failure, got: {err}"
    );
}

#[tokio::test]
async fn test_shutdown_process_terminates_child() {
    // Spawn a long-running process.
    let config = make_server_config("sleep", &["300"]);
    let spawned = spawn_server("test-shutdown", &config, &HashMap::new(), None)
        .expect("spawn_server should succeed for sleep");

    let pid = spawned.pid;
    assert!(pid > 0, "spawned process should have a positive PID");

    // Graceful shutdown via shutdown_process.
    shutdown_process(spawned.child, pid)
        .await
        .expect("shutdown_process should return Ok");

    // Verify the process is no longer running (on Unix).
    #[cfg(unix)]
    {
        use nix::sys::signal::kill;
        use nix::unistd::Pid;

        // Sending signal 0 probes liveness without delivering a real signal.
        // An ESRCH error means the process does not exist — i.e., it was terminated.
        let result = kill(Pid::from_raw(pid as i32), None);
        assert!(
            result.is_err(),
            "process with PID {pid} should no longer be alive after shutdown_process"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-server max_retries / restart_delay overrides
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_max_retries_override_goes_fatal() {
    let mut config = make_server_config("sh", &["-c", "exit 1"]);
    config.max_retries = Some(1);
    config.restart_delay = Some(0);

    let shutdown = CancellationToken::new();
    let log_agg = Arc::new(LogAggregator::new(&["crasher".to_string()], 100));
    let mut handle = start_single_server("crasher", &config, &shutdown, &log_agg).await;

    // With the default of 10 attempts this would still be restarting after 10 s.
    let fatal = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = handle.state_rx.borrow_and_update().clone();
            if matches!(snapshot.process_state, ProcessState::Fatal) {
                return snapshot;
            }
            if handle.state_rx.changed().await.is_err() {
                return handle.state_rx.borrow().clone();
            }
        }
    })
    .await
    .expect("server with max_retries = 1 should reach Fatal within 10s");

    assert!(
        matches!(fatal.process_state, ProcessState::Fatal),
        "expected Fatal, got {}",
        fatal.process_state
    );
    assert_eq!(fatal.restart_count, 1, "Fatal after exactly 1 failure");

    shutdown.cancel();
    let _ = handle.task.await;
}

// ─────────────────────────────────────────────────────────────────────────────
// Process-group kill tests (issue #23, unix only)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(unix)]
mod process_group {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use mcp_hub::logs::LogAggregator;
    use mcp_hub::supervisor::{
        shutdown_process, spawn_server, start_single_server, SupervisorCommand,
    };
    use mcp_hub::types::ProcessState;
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    use tokio_util::sync::CancellationToken;

    use super::make_server_config;

    /// True if `pid` is a live (non-zombie) process.
    ///
    /// Zombies still answer `kill(pid, 0)`; in containers PID 1 may not reap
    /// orphans, so a killed process can linger as a zombie. Treat state `Z` as dead.
    pub(super) fn is_alive(pid: i32) -> bool {
        if kill(Pid::from_raw(pid), None).is_err() {
            return false;
        }
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            // Format: "pid (comm) S ..." — comm may contain spaces/parens, so
            // find the last ')' and read the state char after it.
            Ok(stat) => match stat
                .rfind(')')
                .and_then(|i| stat[i + 1..].trim().chars().next())
            {
                Some('Z') | Some('X') => false,
                Some(_) | None => true,
            },
            Err(_) => false,
        }
    }

    /// Poll until `pid` is gone, up to `limit`. Returns true if it died.
    async fn wait_until_dead(pid: i32, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            if !is_alive(pid) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Poll until `path` holds a parseable first-line PID.
    async fn read_pid_file(path: &Path) -> i32 {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(s) = std::fs::read_to_string(path) {
                if let Some(pid) = s.lines().next().and_then(|l| l.trim().parse::<i32>().ok()) {
                    return pid;
                }
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for pid file {}",
                path.display()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Best-effort cleanup so a failing test does not leak `sleep 600`.
    fn force_kill(pid: i32) {
        let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
    }

    #[tokio::test]
    async fn test_crash_kills_orphaned_process_group_members() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pid_file = dir.path().join("inner.pid");
        // Append so a respawn cannot overwrite the first inner PID.
        let script = format!("sleep 600 & echo $! >> {}; exit 1", pid_file.display());
        let config = make_server_config("bash", &["-c", &script]);

        let shutdown = CancellationToken::new();
        let log_agg = Arc::new(LogAggregator::new(&["crash".to_string()], 16));
        let mut handle = start_single_server("crash", &config, &shutdown, &log_agg).await;

        let inner_pid = tokio::time::timeout(Duration::from_secs(5), read_pid_file(&pid_file))
            .await
            .expect("inner pid file should appear");

        // Wait for the supervisor to observe the crash.
        tokio::time::timeout(
            Duration::from_secs(5),
            handle
                .state_rx
                .wait_for(|s| matches!(s.process_state, ProcessState::Backoff { .. })),
        )
        .await
        .expect("supervisor should reach Backoff after the crash")
        .expect("state channel open");

        // The wrapper is gone; its group member must have been killed too.
        let died = wait_until_dead(inner_pid, Duration::from_secs(2)).await;
        force_kill(inner_pid);

        // Stop the supervisor (also cleans up any respawned instance).
        let _ = handle.cmd_tx.send(SupervisorCommand::Shutdown).await;
        let _ = tokio::time::timeout(Duration::from_secs(10), &mut handle.task).await;

        assert!(
            died,
            "inner `sleep` (pid {inner_pid}) survived the wrapper crash — process group not killed"
        );
    }

    #[tokio::test]
    async fn test_sigkill_escalation_kills_whole_group() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bash_file = dir.path().join("bash.pid");
        let sleep_file = dir.path().join("sleep.pid");
        // Ignore SIGTERM (inherited by the sleep too) so only SIGKILL can stop them.
        let script = format!(
            "trap '' TERM; echo $$ > {}; sleep 600 & echo $! > {}; wait",
            bash_file.display(),
            sleep_file.display()
        );
        let config = make_server_config("bash", &["-c", &script]);
        let spawned = spawn_server("escalate", &config, &HashMap::new(), None)
            .expect("spawn_server should succeed");
        let pid = spawned.pid;

        let bash_pid = read_pid_file(&bash_file).await;
        let sleep_pid = read_pid_file(&sleep_file).await;
        assert_eq!(bash_pid as u32, pid, "bash should be the direct child");

        // Keep stdout/stdin alive for the duration, as the supervisor would.
        let _stdout = spawned.stdout;
        let _stdin = spawned.stdin;

        let started = Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            shutdown_process(spawned.child, pid),
        )
        .await;
        let elapsed = started.elapsed();

        let sleep_dead = wait_until_dead(sleep_pid, Duration::from_secs(2)).await;
        let bash_dead = wait_until_dead(bash_pid, Duration::from_secs(1)).await;
        force_kill(sleep_pid);
        force_kill(bash_pid);

        assert!(result.is_ok(), "shutdown_process timed out");
        assert!(
            elapsed <= Duration::from_secs(7),
            "stop took too long: {elapsed:?}"
        );
        assert!(bash_dead, "bash (pid {bash_pid}) should be dead");
        assert!(
            sleep_dead,
            "group member `sleep` (pid {sleep_pid}) survived SIGKILL escalation"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Supervisor end-to-end: restart, backoff, Restart command, stop_all_servers
//
// Note: the "stable window resets the failure counter" path is not covered
// here — `BackoffConfig::stable_window_secs` is a fixed 60 s with no config
// override, so a faithful test would need a 60 s wall-clock wait.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(unix)]
mod supervisor_e2e {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use mcp_hub::logs::LogAggregator;
    use mcp_hub::supervisor::{
        start_single_server, stop_all_servers, ServerHandle, SupervisorCommand,
    };
    use mcp_hub::types::{ProcessState, ServerSnapshot};
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    use tokio_util::sync::CancellationToken;

    use super::make_server_config;
    use super::process_group::is_alive;

    async fn start(name: &str, command: &str, args: &[&str], restart_delay: u64) -> ServerHandle {
        let mut config = make_server_config(command, args);
        config.restart_delay = Some(restart_delay);
        let token = CancellationToken::new();
        let log_agg = Arc::new(LogAggregator::new(&[name.to_string()], 16));
        start_single_server(name, &config, &token, &log_agg).await
    }

    /// Wait up to `secs` for a snapshot matching `pred`.
    async fn wait_state(
        handle: &mut ServerHandle,
        secs: u64,
        pred: impl Fn(&ServerSnapshot) -> bool,
    ) -> Option<ServerSnapshot> {
        tokio::time::timeout(Duration::from_secs(secs), handle.state_rx.wait_for(pred))
            .await
            .ok()?
            .ok()
            .map(|s| s.clone())
    }

    fn force_kill(pid: u32) {
        let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
    }

    /// Always stop the supervisor (and its child) before any assertion runs.
    async fn finish(handle: ServerHandle) {
        stop_all_servers(vec![handle]).await;
    }

    #[tokio::test]
    async fn crashed_server_is_restarted_with_new_pid() {
        let mut h = start("flap", "bash", &["-c", "sleep 0.3; exit 1"], 0).await;

        let first = wait_state(&mut h, 10, |s| {
            matches!(s.process_state, ProcessState::Running)
        })
        .await;
        let second = match &first {
            Some(f) => {
                let old = f.pid;
                wait_state(&mut h, 10, |s| {
                    matches!(s.process_state, ProcessState::Running)
                        && s.pid != old
                        && s.restart_count >= 1
                })
                .await
            }
            None => None,
        };
        if let Some(pid) = second.as_ref().and_then(|s| s.pid) {
            force_kill(pid);
        }
        finish(h).await;

        let first = first.expect("server should reach Running");
        let second = second.expect("server should be restarted into Running with a new pid");
        assert_ne!(first.pid, second.pid, "restart must spawn a new process");
        assert_eq!(first.restart_count, 0);
        assert!(second.restart_count >= 1, "restart_count must increment");
    }

    #[tokio::test]
    async fn backoff_delay_grows_between_attempts() {
        // restart_delay = 1 -> delays of roughly 1 s, 2 s, 4 s (±30 % jitter).
        let mut h = start("crasher", "sh", &["-c", "exit 1"], 1).await;

        let mut remaining = Vec::new();
        for attempt in 1..=3u32 {
            let snap = wait_state(&mut h, 15, |s| {
                matches!(s.process_state, ProcessState::Backoff { attempt: a, .. } if a == attempt)
            })
            .await;
            match snap.map(|s| s.process_state) {
                Some(ProcessState::Backoff { until, .. }) => {
                    remaining.push(until.saturating_duration_since(Instant::now()));
                }
                Some(_) | None => break,
            }
        }
        finish(h).await;

        assert_eq!(remaining.len(), 3, "expected Backoff attempts 1..=3");
        // Jitter ranges are [0.7, 1.3] s for attempt 1 and [2.8, 5.2] s for 3.
        assert!(
            remaining[2] > remaining[0],
            "backoff must grow: attempt 1 {:?} vs attempt 3 {:?}",
            remaining[0],
            remaining[2]
        );
    }

    #[tokio::test]
    async fn restart_command_replaces_running_process() {
        let mut h = start("sleeper", "sleep", &["300"], 0).await;

        let first = wait_state(&mut h, 10, |s| {
            matches!(s.process_state, ProcessState::Running)
        })
        .await;
        let old_pid = first.as_ref().and_then(|s| s.pid);
        if old_pid.is_some() {
            let _ = h.cmd_tx.send(SupervisorCommand::Restart).await;
        }
        let second = wait_state(&mut h, 15, |s| {
            matches!(s.process_state, ProcessState::Running) && s.pid != old_pid
        })
        .await;
        let new_pid = second.as_ref().and_then(|s| s.pid);
        let old_alive = old_pid.is_some_and(|p| is_alive(p as i32));
        if let Some(p) = old_pid {
            force_kill(p);
        }
        finish(h).await;
        let new_alive = new_pid.is_some_and(|p| is_alive(p as i32));
        if let Some(p) = new_pid {
            force_kill(p);
        }

        let old_pid = old_pid.expect("server should reach Running");
        let second = second.expect("Restart should bring up a new Running process");
        assert_ne!(second.pid, Some(old_pid), "Restart must give a new pid");
        assert!(
            !old_alive,
            "old process {old_pid} must be gone after Restart"
        );
        assert!(!new_alive, "new process must be stopped by shutdown");
        assert_eq!(second.restart_count, 1, "Restart counts as one restart");
    }

    #[tokio::test]
    async fn stop_all_servers_leaves_no_live_child() {
        let mut a = start("a", "sleep", &["300"], 0).await;
        let mut b = start("b", "sleep", &["301"], 0).await;
        let sa = wait_state(&mut a, 10, |s| {
            matches!(s.process_state, ProcessState::Running)
        })
        .await;
        let sb = wait_state(&mut b, 10, |s| {
            matches!(s.process_state, ProcessState::Running)
        })
        .await;
        let (rx_a, rx_b) = (a.state_rx.clone(), b.state_rx.clone());

        stop_all_servers(vec![a, b]).await;

        let pids: Vec<u32> = [&sa, &sb]
            .iter()
            .filter_map(|s| s.as_ref().and_then(|s| s.pid))
            .collect();
        let alive: Vec<u32> = pids
            .iter()
            .copied()
            .filter(|p| is_alive(*p as i32))
            .collect();
        alive.iter().for_each(|p| force_kill(*p));

        assert_eq!(pids.len(), 2, "both servers should have reached Running");
        assert!(
            alive.is_empty(),
            "children still alive after stop: {alive:?}"
        );
        for rx in [rx_a, rx_b] {
            let state = rx.borrow().process_state.clone();
            assert!(
                matches!(state, ProcessState::Stopped),
                "expected Stopped, got {state}"
            );
        }
    }
}
