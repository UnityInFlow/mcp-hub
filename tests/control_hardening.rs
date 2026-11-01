/// Control socket hardening tests (issue #13): owner-only permissions, private
/// state directory, idle-connection timeout, request size cap, and refusal to
/// touch socket/PID files owned by another user.
#[cfg(unix)]
mod hardening {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use mcp_hub::control::{
        run_control_socket_with_limits, send_daemon_command, ControlLimits, DaemonRequest,
        DaemonResponse, DaemonState, DEFAULT_MAX_REQUEST_BYTES,
    };
    use mcp_hub::daemon::{check_existing_daemon, ensure_private_dir};
    use mcp_hub::logs::LogAggregator;
    use tempfile::TempDir;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;
    use tokio_util::sync::CancellationToken;

    fn mode_of(path: &Path) -> u32 {
        std::fs::metadata(path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777
    }

    /// Start the control socket on `sock` and wait until it is bound.
    async fn start(
        sock: &Path,
        limits: ControlLimits,
    ) -> (CancellationToken, tokio::task::JoinHandle<()>) {
        let shutdown = CancellationToken::new();
        let state = Arc::new(DaemonState {
            handles: Arc::new(tokio::sync::Mutex::new(vec![])),
            log_agg: Arc::new(LogAggregator::new(&[], 1000)),
            shutdown: shutdown.clone(),
            color: false,
        });
        let sock_owned = sock.to_path_buf();
        let handle = tokio::spawn(async move {
            run_control_socket_with_limits(&sock_owned, state, limits)
                .await
                .expect("control socket should run");
        });
        for _ in 0..100 {
            if sock.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // Allow the post-bind chmod to complete as well.
        tokio::time::sleep(Duration::from_millis(100)).await;
        (shutdown, handle)
    }

    #[tokio::test]
    async fn control_socket_is_owner_only() {
        let dir = TempDir::new().expect("temp dir");
        let sock = dir.path().join("perm.sock");
        let (shutdown, handle) = start(&sock, ControlLimits::default()).await;

        assert_eq!(mode_of(&sock), 0o600, "socket must be 0600");

        shutdown.cancel();
        handle.await.expect("server task");
    }

    #[test]
    fn ensure_private_dir_creates_0700() {
        let base = TempDir::new().expect("temp dir");
        let dir = base.path().join("a").join("mcp-hub");
        ensure_private_dir(&dir).expect("create private dir");
        assert_eq!(mode_of(&dir), 0o700);
    }

    #[test]
    fn ensure_private_dir_tightens_existing_dir() {
        let base = TempDir::new().expect("temp dir");
        let dir = base.path().join("mcp-hub");
        std::fs::create_dir(&dir).expect("mkdir");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        ensure_private_dir(&dir).expect("tighten");
        assert_eq!(mode_of(&dir), 0o700);
    }

    #[tokio::test]
    async fn idle_client_is_disconnected_after_read_timeout() {
        let dir = TempDir::new().expect("temp dir");
        let sock = dir.path().join("idle.sock");
        let limits = ControlLimits {
            read_timeout: Duration::from_millis(200),
            ..ControlLimits::default()
        };
        let (shutdown, handle) = start(&sock, limits).await;

        let mut stream = UnixStream::connect(&sock).await.expect("connect");
        let mut buf = [0u8; 16];
        let n = tokio::time::timeout(Duration::from_secs(3), stream.read(&mut buf))
            .await
            .expect("server should close the idle connection within the timeout")
            .expect("read");
        assert_eq!(n, 0, "idle connection should be closed (EOF)");

        shutdown.cancel();
        handle.await.expect("server task");
    }

    #[tokio::test]
    async fn oversized_request_line_is_rejected() {
        let dir = TempDir::new().expect("temp dir");
        let sock = dir.path().join("big.sock");
        let (shutdown, handle) = start(&sock, ControlLimits::default()).await;

        let stream = UnixStream::connect(&sock).await.expect("connect");
        let (reader, mut writer) = stream.into_split();

        // Larger than the cap with no newline: the server must give up without
        // waiting for one. It may close before we finish writing; ignore EPIPE.
        let payload = vec![b'a'; DEFAULT_MAX_REQUEST_BYTES * 2];
        let _ = writer.write_all(&payload).await;

        let mut lines = BufReader::new(reader).lines();
        let line = tokio::time::timeout(Duration::from_secs(3), lines.next_line())
            .await
            .expect("server should respond promptly")
            .expect("read")
            .expect("an error response line");
        let resp: DaemonResponse = serde_json::from_str(&line).expect("response JSON");
        assert!(!resp.ok, "oversized request must yield ok=false");
        let msg = resp.error.unwrap_or_default();
        assert!(msg.contains("too large"), "unexpected message: {msg}");

        // The connection is closed afterwards.
        let next = tokio::time::timeout(Duration::from_secs(3), lines.next_line())
            .await
            .expect("connection should be closed")
            .expect("read");
        assert!(next.is_none(), "connection should be closed after error");

        // The daemon survives and still serves valid requests.
        let ok = send_daemon_command(&sock, &DaemonRequest::Status, 5)
            .await
            .expect("daemon still alive");
        assert!(ok.ok);

        shutdown.cancel();
        handle.await.expect("server task");
    }

    #[test]
    fn check_existing_daemon_refuses_foreign_owned_files() {
        // Only possible when running as root: chown a file to another uid.
        if nix::unistd::getuid().as_raw() != 0 {
            eprintln!("skipping: needs root to create a foreign-owned file");
            return;
        }
        let dir = TempDir::new().expect("temp dir");
        let sock = dir.path().join("foreign.sock");
        let pid = dir.path().join("foreign.pid");
        std::fs::write(&sock, b"").expect("write");
        std::os::unix::fs::chown(&sock, Some(54321), None).expect("chown");
        assert_eq!(std::fs::metadata(&sock).expect("meta").uid(), 54321);

        let err = check_existing_daemon(&sock, &pid).expect_err("must refuse");
        assert!(
            err.to_string().contains("owned by another user"),
            "unexpected error: {err}"
        );
        assert!(sock.exists(), "foreign-owned file must not be unlinked");
    }
}
