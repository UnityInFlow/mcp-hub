use mcp_hub::logs::{format_log_line, LogAggregator, LogBuffer, LogLine};
use std::time::{Duration, SystemTime};

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn make_line(server: &str, message: &str) -> LogLine {
    LogLine {
        server: server.to_string(),
        timestamp: SystemTime::now(),
        message: message.to_string(),
    }
}

fn make_line_with_offset(server: &str, message: &str, offset_secs: u64) -> LogLine {
    LogLine {
        server: server.to_string(),
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + offset_secs),
        message: message.to_string(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Ring buffer capacity enforcement
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn push_within_capacity() {
    let buf = LogBuffer::new(10);
    for i in 0..5 {
        buf.push(make_line("srv", &format!("msg {i}"))).await;
    }
    assert_eq!(buf.len().await, 5);
}

#[tokio::test]
async fn push_evicts_oldest_at_capacity() {
    let capacity = 5;
    let buf = LogBuffer::new(capacity);

    // Push capacity + 1 lines. The first line ("msg 0") should be evicted.
    for i in 0..=(capacity as u32) {
        buf.push(make_line("srv", &format!("msg {i}"))).await;
    }

    assert_eq!(buf.len().await, capacity);

    let snap = buf.snapshot().await;
    let messages: Vec<&str> = snap.iter().map(|l| l.message.as_str()).collect();
    assert!(
        !messages.contains(&"msg 0"),
        "oldest line 'msg 0' should have been evicted; got: {messages:?}"
    );
    assert!(
        messages.contains(&"msg 5"),
        "newest line 'msg 5' should be present"
    );
}

#[tokio::test]
async fn push_many_over_capacity() {
    let capacity = 4;
    let buf = LogBuffer::new(capacity);

    // Push 2× capacity lines.
    for i in 0..(capacity * 2) as u32 {
        buf.push(make_line("srv", &format!("msg {i}"))).await;
    }

    assert_eq!(buf.len().await, capacity);

    let snap = buf.snapshot().await;
    // Only the last `capacity` lines should remain: msg 4, msg 5, msg 6, msg 7.
    let messages: Vec<&str> = snap.iter().map(|l| l.message.as_str()).collect();
    assert_eq!(messages, vec!["msg 4", "msg 5", "msg 6", "msg 7"]);
}

// ─────────────────────────────────────────────────────────────────────────────
// Snapshot ordering
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn snapshot_preserves_fifo_order() {
    let buf = LogBuffer::new(10);
    for letter in ["A", "B", "C"] {
        buf.push(make_line("srv", letter)).await;
    }

    let snap = buf.snapshot().await;
    let messages: Vec<&str> = snap.iter().map(|l| l.message.as_str()).collect();
    assert_eq!(messages, vec!["A", "B", "C"]);
}

#[tokio::test]
async fn snapshot_last_returns_tail() {
    let buf = LogBuffer::new(20);
    for i in 0..10u32 {
        buf.push(make_line("srv", &format!("msg {i}"))).await;
    }

    let tail = buf.snapshot_last(3).await;
    let messages: Vec<&str> = tail.iter().map(|l| l.message.as_str()).collect();
    assert_eq!(messages, vec!["msg 7", "msg 8", "msg 9"]);
}

// ─────────────────────────────────────────────────────────────────────────────
// LogAggregator multi-server
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn aggregator_push_to_correct_server() {
    let names = vec!["a".to_string(), "b".to_string()];
    let agg = LogAggregator::new(&names, 10);

    agg.push("a", "hello from a".to_string()).await;
    agg.push("b", "hello from b".to_string()).await;
    agg.push("a", "second from a".to_string()).await;

    let buf_a = agg.get_buffer("a").expect("buffer 'a' should exist");
    let buf_b = agg.get_buffer("b").expect("buffer 'b' should exist");

    let snap_a = buf_a.snapshot().await;
    let snap_b = buf_b.snapshot().await;

    assert_eq!(snap_a.len(), 2, "buffer 'a' should have 2 lines");
    assert_eq!(snap_b.len(), 1, "buffer 'b' should have 1 line");

    assert!(
        snap_a.iter().all(|l| l.server == "a"),
        "all lines in 'a' buffer should have server='a'"
    );
    assert!(
        snap_b.iter().all(|l| l.server == "b"),
        "all lines in 'b' buffer should have server='b'"
    );
}

#[tokio::test]
async fn aggregator_snapshot_all_merges_and_sorts() {
    let names = vec!["alpha".to_string(), "beta".to_string()];
    let agg = LogAggregator::new(&names, 20);

    // Push lines with explicit timestamps to ensure deterministic sort order.
    // beta at t=1, alpha at t=2, beta at t=3
    let buf_alpha = agg.get_buffer("alpha").unwrap().clone();
    let buf_beta = agg.get_buffer("beta").unwrap().clone();

    buf_beta
        .push(LogLine {
            server: "beta".to_string(),
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
            message: "beta first".to_string(),
        })
        .await;

    buf_alpha
        .push(LogLine {
            server: "alpha".to_string(),
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(2),
            message: "alpha second".to_string(),
        })
        .await;

    buf_beta
        .push(LogLine {
            server: "beta".to_string(),
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(3),
            message: "beta third".to_string(),
        })
        .await;

    let all = agg.snapshot_all().await;
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].message, "beta first");
    assert_eq!(all[1].message, "alpha second");
    assert_eq!(all[2].message, "beta third");
}

#[tokio::test]
async fn aggregator_subscribe_receives_all() {
    let names = vec!["srv".to_string()];
    let agg = LogAggregator::new(&names, 10);

    let mut rx = agg.subscribe();

    agg.push("srv", "line one".to_string()).await;
    agg.push("srv", "line two".to_string()).await;

    let first = rx.try_recv().expect("should receive first line");
    let second = rx.try_recv().expect("should receive second line");

    assert_eq!(first.message, "line one");
    assert_eq!(second.message, "line two");
}

// ─────────────────────────────────────────────────────────────────────────────
// format_log_line
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn format_log_line_no_color() {
    let line = make_line_with_offset("my-server", "hello world", 0);
    let formatted = format_log_line(&line, false);

    assert!(
        formatted.contains("my-server"),
        "output should contain server name; got: {formatted}"
    );
    assert!(
        formatted.contains('|'),
        "output should contain pipe separator; got: {formatted}"
    );
    assert!(
        formatted.contains("hello world"),
        "output should contain message; got: {formatted}"
    );
    // Should contain a timestamp with 'T' and 'Z'.
    assert!(
        formatted.contains('T') && formatted.contains('Z'),
        "output should contain ISO timestamp; got: {formatted}"
    );
}

#[test]
fn format_log_line_with_color() {
    let line = make_line_with_offset("my-server", "hello world", 0);
    let colored = format_log_line(&line, true);
    let plain = format_log_line(&line, false);

    // ANSI escape codes make the colored string longer.
    assert!(
        colored.len() > plain.len(),
        "colored output should be longer than plain (ANSI codes); colored={}, plain={}",
        colored.len(),
        plain.len()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Edge cases
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn empty_buffer_snapshot() {
    let buf = LogBuffer::new(10);
    let snap = buf.snapshot().await;
    assert!(snap.is_empty(), "snapshot of empty buffer should be empty");
}

#[tokio::test]
async fn snapshot_last_more_than_available() {
    let buf = LogBuffer::new(10);
    buf.push(make_line("s", "a")).await;
    buf.push(make_line("s", "b")).await;
    buf.push(make_line("s", "c")).await;

    let tail = buf.snapshot_last(100).await;
    assert_eq!(
        tail.len(),
        3,
        "snapshot_last(100) should return all 3 available lines"
    );
}
