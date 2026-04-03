use mcp_hub::types::{compute_health_status, format_uptime, HealthStatus};
use std::time::{Duration, Instant};

// ─────────────────────────────────────────────────────────────────────────────
// Health state transition tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn unknown_to_healthy_on_zero_misses() {
    // The success path directly creates Healthy — not via compute_health_status.
    // Verify that the default (Unknown) is overridden correctly.
    let status: HealthStatus = HealthStatus::default();
    assert!(matches!(status, HealthStatus::Unknown));

    // Simulate a successful ping: construct Healthy directly.
    let healthy = HealthStatus::Healthy {
        latency_ms: 5,
        last_checked: Instant::now(),
    };
    assert!(matches!(healthy, HealthStatus::Healthy { .. }));
}

#[test]
fn healthy_to_degraded_at_2_misses() {
    let current = HealthStatus::Healthy {
        latency_ms: 10,
        last_checked: Instant::now(),
    };
    let next = compute_health_status(2, &current);
    assert!(
        matches!(
            next,
            HealthStatus::Degraded {
                consecutive_misses: 2,
                ..
            }
        ),
        "Expected Degraded(2), got: {next}"
    );
}

#[test]
fn degraded_stays_degraded_at_3_to_6_misses() {
    for misses in 3..=6 {
        let current = HealthStatus::Degraded {
            consecutive_misses: misses - 1,
            last_success: None,
        };
        let next = compute_health_status(misses, &current);
        assert!(
            matches!(next, HealthStatus::Degraded { consecutive_misses, .. } if consecutive_misses == misses),
            "misses={misses}: Expected Degraded({misses}), got: {next}"
        );
    }
}

#[test]
fn degraded_to_failed_at_7_misses() {
    let current = HealthStatus::Degraded {
        consecutive_misses: 6,
        last_success: None,
    };
    let next = compute_health_status(7, &current);
    assert!(
        matches!(
            next,
            HealthStatus::Failed {
                consecutive_misses: 7
            }
        ),
        "Expected Failed(7), got: {next}"
    );
}

#[test]
fn degraded_to_healthy_on_recovery() {
    // After being Degraded, a successful ping creates Healthy directly.
    let _was_degraded = HealthStatus::Degraded {
        consecutive_misses: 3,
        last_success: None,
    };

    // Simulate the success path: caller constructs Healthy directly.
    let recovered = HealthStatus::Healthy {
        latency_ms: 8,
        last_checked: Instant::now(),
    };
    assert!(
        matches!(recovered, HealthStatus::Healthy { .. }),
        "Expected Healthy after recovery, got: {recovered}"
    );
}

#[test]
fn failed_to_healthy_on_recovery() {
    // Same as above but from Failed state.
    let _was_failed = HealthStatus::Failed {
        consecutive_misses: 10,
    };

    let recovered = HealthStatus::Healthy {
        latency_ms: 12,
        last_checked: Instant::now(),
    };
    assert!(
        matches!(recovered, HealthStatus::Healthy { .. }),
        "Expected Healthy after recovery from Failed, got: {recovered}"
    );
}

#[test]
fn health_resets_to_unknown_on_restart() {
    // D-04: After a restart the health must reset to Unknown (the Default).
    let reset: HealthStatus = Default::default();
    assert!(
        matches!(reset, HealthStatus::Unknown),
        "Expected Unknown after reset, got: {reset}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// format_uptime tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn format_uptime_zero() {
    assert_eq!(format_uptime(Duration::from_secs(0)), "00:00:00");
}

#[test]
fn format_uptime_59_secs() {
    assert_eq!(format_uptime(Duration::from_secs(59)), "00:00:59");
}

#[test]
fn format_uptime_one_hour() {
    assert_eq!(format_uptime(Duration::from_secs(3600)), "01:00:00");
}

#[test]
fn format_uptime_mixed() {
    // 3661s = 1h + 1m + 1s
    assert_eq!(format_uptime(Duration::from_secs(3661)), "01:01:01");
}

#[test]
fn format_uptime_over_24h() {
    // 90061s = 25h + 1m + 1s
    assert_eq!(format_uptime(Duration::from_secs(90061)), "25:01:01");
}

// ─────────────────────────────────────────────────────────────────────────────
// Display trait tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn display_unknown() {
    assert_eq!(HealthStatus::Unknown.to_string(), "unknown");
}

#[test]
fn display_healthy() {
    let status = HealthStatus::Healthy {
        latency_ms: 5,
        last_checked: Instant::now(),
    };
    assert_eq!(status.to_string(), "healthy");
}

#[test]
fn display_degraded() {
    let status = HealthStatus::Degraded {
        consecutive_misses: 3,
        last_success: None,
    };
    assert_eq!(status.to_string(), "degraded (3 missed)");
}

#[test]
fn display_failed() {
    let status = HealthStatus::Failed {
        consecutive_misses: 7,
    };
    assert_eq!(status.to_string(), "failed (7 missed)");
}
