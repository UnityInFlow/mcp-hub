/// Daemon lifecycle integration tests.
///
/// All tests are Unix-only (`#[cfg(unix)]`) because daemon mode requires
/// `fork(2)`. Each test uses a unique socket and PID file path via a temp
/// directory, injected via `MCP_HUB_SOCKET` and `MCP_HUB_PID` environment
/// variables, to avoid interference with other tests or a running production
/// daemon.
#[cfg(unix)]
mod daemon_tests {
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::time::Duration;

    use assert_cmd::Command;
    use tempfile::{NamedTempFile, TempDir};

    // ─────────────────────────────────────────────────────────────────────────
    // Helpers
    // ─────────────────────────────────────────────────────────────────────────

    fn mcp_hub() -> Command {
        Command::cargo_bin("mcp-hub").expect("mcp-hub binary not found")
    }

    /// Write a minimal mcp-hub TOML config with a single `sleep`-based server.
    fn write_minimal_config(dir: &TempDir) -> NamedTempFile {
        let mut f = NamedTempFile::new_in(dir.path()).expect("tempfile");
        writeln!(
            f,
            r#"
[servers.test-server]
command = "sleep"
args = ["9999"]
transport = "stdio"
"#
        )
        .expect("write config");
        f
    }

    /// Read the PID from a PID file. Returns None if the file does not exist or
    /// cannot be parsed.
    fn read_pid(path: &PathBuf) -> Option<u32> {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
    }

    /// Wait up to `max_wait` for a condition to become true, polling every 100 ms.
    fn wait_for<F: Fn() -> bool>(condition: F, max_wait: Duration) -> bool {
        let deadline = std::time::Instant::now() + max_wait;
        while std::time::Instant::now() < deadline {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    /// Isolated socket + PID paths for a single test daemon instance.
    ///
    /// `MCP_HUB_SOCKET` and `MCP_HUB_PID` override the default path resolution
    /// inside daemon.rs so each test gets its own socket without conflicting
    /// with other tests or a production daemon.
    struct TestDaemonPaths {
        sock: PathBuf,
        pid: PathBuf,
        _dir: TempDir, // keep the tempdir alive
    }

    impl TestDaemonPaths {
        fn new() -> Self {
            let dir = TempDir::new().expect("tempdir");
            let sock = dir.path().join("mcp-hub.sock");
            let pid = dir.path().join("mcp-hub.pid");
            Self {
                sock,
                pid,
                _dir: dir,
            }
        }

        /// Environment entries that override socket/PID paths inside mcp-hub.
        fn env_overrides(&self) -> [(&'static str, String); 2] {
            [
                ("MCP_HUB_SOCKET", self.sock.to_string_lossy().into()),
                ("MCP_HUB_PID", self.pid.to_string_lossy().into()),
            ]
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 1: daemon creates socket and PID files
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn daemon_creates_socket_and_pid() {
        let paths = TestDaemonPaths::new();
        let cfg_dir = TempDir::new().expect("cfg tempdir");
        let config = write_minimal_config(&cfg_dir);

        // Start the daemon.
        let output = mcp_hub()
            .args(["start", "--daemon", "-c", config.path().to_str().unwrap()])
            .envs(paths.env_overrides())
            .output()
            .expect("failed to run mcp-hub");

        assert!(
            output.status.success(),
            "mcp-hub start --daemon should exit 0; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        // The daemon forks — give it a moment to write its PID file and socket.
        let sock_exists = wait_for(|| paths.sock.exists(), Duration::from_secs(5));
        let pid_exists = wait_for(|| paths.pid.exists(), Duration::from_secs(5));

        // Stop the daemon before asserting so we don't leave orphans.
        let _ = mcp_hub().arg("stop").envs(paths.env_overrides()).output();

        assert!(sock_exists, "Socket file should exist after daemon starts");
        assert!(pid_exists, "PID file should exist after daemon starts");

        if let Some(pid) = read_pid(&paths.pid) {
            assert!(pid > 1, "PID file should contain a valid PID > 1");
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 2: duplicate daemon prevention
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn duplicate_daemon_prevention() {
        let paths = TestDaemonPaths::new();
        let cfg_dir = TempDir::new().expect("cfg tempdir");
        let config = write_minimal_config(&cfg_dir);

        // Start the first daemon.
        mcp_hub()
            .args(["start", "--daemon", "-c", config.path().to_str().unwrap()])
            .envs(paths.env_overrides())
            .output()
            .expect("first daemon start");

        // Wait for the socket to appear.
        wait_for(|| paths.sock.exists(), Duration::from_secs(5));

        // Try starting a second daemon — must fail with exit code 1.
        let second = mcp_hub()
            .args(["start", "--daemon", "-c", config.path().to_str().unwrap()])
            .envs(paths.env_overrides())
            .output()
            .expect("second daemon start attempt");

        // Stop the first daemon.
        let _ = mcp_hub().arg("stop").envs(paths.env_overrides()).output();

        assert!(
            !second.status.success(),
            "Second daemon start should fail (exit code != 0)"
        );
        let stderr = String::from_utf8_lossy(&second.stderr);
        assert!(
            stderr.to_lowercase().contains("already running")
                || stderr.contains("daemon is already running"),
            "stderr should mention 'already running', got: {stderr}"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 3: `mcp-hub status` returns server info from daemon
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn status_from_daemon() {
        let paths = TestDaemonPaths::new();
        let cfg_dir = TempDir::new().expect("cfg tempdir");
        let config = write_minimal_config(&cfg_dir);

        mcp_hub()
            .args(["start", "--daemon", "-c", config.path().to_str().unwrap()])
            .envs(paths.env_overrides())
            .output()
            .expect("start daemon");

        // Wait for socket.
        wait_for(|| paths.sock.exists(), Duration::from_secs(5));

        // A brief additional wait for the daemon to be fully ready to accept
        // control socket connections.
        std::thread::sleep(Duration::from_millis(500));

        let status_output = mcp_hub()
            .arg("status")
            .envs(paths.env_overrides())
            .output()
            .expect("mcp-hub status");

        // Stop daemon.
        let _ = mcp_hub().arg("stop").envs(paths.env_overrides()).output();

        assert!(
            status_output.status.success(),
            "mcp-hub status should exit 0; stderr: {}",
            String::from_utf8_lossy(&status_output.stderr)
        );
        let stdout = String::from_utf8_lossy(&status_output.stdout);
        assert!(
            stdout.contains("test-server"),
            "Status output should include the server name 'test-server', got: {stdout}"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 4: `mcp-hub stop` shuts down daemon
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn stop_shuts_down_daemon() {
        let paths = TestDaemonPaths::new();
        let cfg_dir = TempDir::new().expect("cfg tempdir");
        let config = write_minimal_config(&cfg_dir);

        mcp_hub()
            .args(["start", "--daemon", "-c", config.path().to_str().unwrap()])
            .envs(paths.env_overrides())
            .output()
            .expect("start daemon");

        wait_for(|| paths.sock.exists(), Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(300));

        let stop_output = mcp_hub()
            .arg("stop")
            .envs(paths.env_overrides())
            .output()
            .expect("mcp-hub stop");

        assert!(
            stop_output.status.success(),
            "mcp-hub stop should exit 0; stderr: {}",
            String::from_utf8_lossy(&stop_output.stderr)
        );

        // Socket file should disappear after daemon shuts down.
        let sock_gone = wait_for(|| !paths.sock.exists(), Duration::from_secs(8));
        assert!(
            sock_gone,
            "Socket file should be removed after daemon stops"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 5: `mcp-hub restart <name>` via daemon
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn restart_via_daemon() {
        let paths = TestDaemonPaths::new();
        let cfg_dir = TempDir::new().expect("cfg tempdir");
        let config = write_minimal_config(&cfg_dir);

        mcp_hub()
            .args(["start", "--daemon", "-c", config.path().to_str().unwrap()])
            .envs(paths.env_overrides())
            .output()
            .expect("start daemon");

        wait_for(|| paths.sock.exists(), Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(300));

        let restart_output = mcp_hub()
            .args(["restart", "test-server"])
            .envs(paths.env_overrides())
            .output()
            .expect("mcp-hub restart");

        // Stop daemon.
        let _ = mcp_hub().arg("stop").envs(paths.env_overrides()).output();

        assert!(
            restart_output.status.success(),
            "mcp-hub restart should exit 0; stderr: {}",
            String::from_utf8_lossy(&restart_output.stderr)
        );
        let stdout = String::from_utf8_lossy(&restart_output.stdout);
        assert!(
            stdout.contains("Restart signal sent"),
            "Output should mention 'Restart signal sent', got: {stdout}"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 6: stale socket cleanup after crash
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn stale_socket_cleanup() {
        let paths = TestDaemonPaths::new();
        let cfg_dir = TempDir::new().expect("cfg tempdir");
        let config = write_minimal_config(&cfg_dir);

        // Start the first daemon instance.
        mcp_hub()
            .args(["start", "--daemon", "-c", config.path().to_str().unwrap()])
            .envs(paths.env_overrides())
            .output()
            .expect("first daemon start");

        wait_for(|| paths.sock.exists(), Duration::from_secs(5));
        wait_for(|| paths.pid.exists(), Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(300));

        // Force-kill the daemon process to simulate a crash.
        if let Some(pid) = read_pid(&paths.pid) {
            let _ = std::process::Command::new("kill")
                .args(["-9", &pid.to_string()])
                .output();
        }

        // Wait for the process to die and the socket to go stale.
        std::thread::sleep(Duration::from_millis(800));

        // Start a new daemon — it should clean up the stale socket and start successfully.
        let second_start = mcp_hub()
            .args(["start", "--daemon", "-c", config.path().to_str().unwrap()])
            .envs(paths.env_overrides())
            .output()
            .expect("second daemon start after crash");

        let second_sock_appeared = wait_for(
            || {
                // The new daemon should write a new socket.
                // We verify it's connectable, not just that the file exists.
                use std::os::unix::net::UnixStream;
                paths.sock.exists() && UnixStream::connect(&paths.sock).is_ok()
            },
            Duration::from_secs(8),
        );

        // Stop the second daemon.
        let _ = mcp_hub().arg("stop").envs(paths.env_overrides()).output();

        assert!(
            second_start.status.success(),
            "Second daemon start after crash should succeed; stderr: {}",
            String::from_utf8_lossy(&second_start.stderr)
        );
        assert!(
            second_sock_appeared,
            "New daemon socket should be connectable after stale socket cleanup"
        );
    }
}
