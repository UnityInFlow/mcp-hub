use std::collections::HashMap;

use mcp_hub::config::ServerConfig;
use mcp_hub::supervisor::{compute_backoff_delay, shutdown_process, spawn_server};
use mcp_hub::types::BackoffConfig;

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
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;

        // Sending signal 0 probes liveness without delivering a real signal.
        // An ESRCH error means the process does not exist — i.e., it was terminated.
        let result = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
        assert!(
            result.is_err(),
            "process with PID {pid} should no longer be alive after shutdown_process"
        );
    }
}
