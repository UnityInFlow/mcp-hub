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
[hub]
web_port = {}

[servers.test-server]
command = "sleep"
args = ["9999"]
transport = "stdio"
"#,
            free_port()
        )
        .expect("write config");
        f
    }

    /// Pick a free TCP port so parallel test daemons never fight over the
    /// default web UI port (a bind failure is fatal for `start --daemon`).
    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind ephemeral port")
            .local_addr()
            .expect("local addr")
            .port()
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

    // ─────────────────────────────────────────────────────────────────────────
    // #18: startup errors must surface in the parent, before/while daemonizing
    // ─────────────────────────────────────────────────────────────────────────

    /// Assert that no PID or socket file was left behind by a failed start.
    fn assert_no_daemon_files(paths: &TestDaemonPaths) {
        assert!(!paths.pid.exists(), "stale PID file left behind");
        assert!(!paths.sock.exists(), "stale socket file left behind");
    }

    #[test]
    fn start_daemon_missing_config_fails_cleanly() {
        let paths = TestDaemonPaths::new();

        let output = mcp_hub()
            .args(["start", "--daemon", "-c", "/nonexistent/mcp-hub.toml"])
            .envs(paths.env_overrides())
            .output()
            .expect("run mcp-hub");

        assert!(
            !output.status.success(),
            "start --daemon with a missing config must exit non-zero"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("Failed to load config") && stderr.contains("nonexistent"),
            "stderr should mention the config error, got: {stderr}"
        );
        assert_no_daemon_files(&paths);
    }

    #[test]
    fn start_daemon_zero_servers_fails_cleanly() {
        let paths = TestDaemonPaths::new();
        let cfg_dir = TempDir::new().expect("cfg tempdir");
        let mut f = NamedTempFile::new_in(cfg_dir.path()).expect("tempfile");
        writeln!(f, "[hub]\nweb_port = {}", free_port()).expect("write config");

        let output = mcp_hub()
            .args(["start", "--daemon", "-c", f.path().to_str().unwrap()])
            .envs(paths.env_overrides())
            .output()
            .expect("run mcp-hub");

        assert!(
            !output.status.success(),
            "start --daemon with zero servers must exit non-zero"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("No servers defined"),
            "stderr should mention the empty server list, got: {stderr}"
        );
        assert_no_daemon_files(&paths);
    }

    #[test]
    fn start_daemon_web_port_in_use_fails_cleanly() {
        let paths = TestDaemonPaths::new();
        let cfg_dir = TempDir::new().expect("cfg tempdir");

        // Occupy a port so the daemon's web UI bind fails after the fork.
        let blocker = std::net::TcpListener::bind("127.0.0.1:0").expect("bind blocker");
        let port = blocker.local_addr().expect("local addr").port();

        let mut f = NamedTempFile::new_in(cfg_dir.path()).expect("tempfile");
        writeln!(
            f,
            "[hub]\nweb_port = {port}\n\n[servers.s]\ncommand = \"sleep\"\nargs = [\"9999\"]"
        )
        .expect("write config");

        let output = mcp_hub()
            .args(["start", "--daemon", "-c", f.path().to_str().unwrap()])
            .envs(paths.env_overrides())
            .output()
            .expect("run mcp-hub");

        assert!(
            !output.status.success(),
            "start --daemon must exit non-zero when the web port is taken"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("Failed to bind web UI"),
            "stderr should relay the daemon's bind error, got: {stderr}"
        );
        // The daemon removes its PID file and socket on its way out.
        assert!(
            wait_for(
                || !paths.pid.exists() && !paths.sock.exists(),
                Duration::from_secs(5)
            ),
            "PID/socket files should be cleaned up after a failed start"
        );
        drop(blocker);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // #19: the daemon logs to a file instead of /dev/null
    // ─────────────────────────────────────────────────────────────────────────

    /// Config with one server that prints a line on stderr and then idles.
    fn write_chatty_config(dir: &TempDir) -> NamedTempFile {
        let mut f = NamedTempFile::new_in(dir.path()).expect("tempfile");
        writeln!(
            f,
            r#"
[hub]
web_port = {}

[servers.chatty]
command = "bash"
args = ["-c", "echo child-stderr-line >&2; sleep 9999"]
transport = "stdio"
"#,
            free_port()
        )
        .expect("write config");
        f
    }

    /// Poll until `path` contains `needle` (the daemon logs asynchronously).
    fn log_contains(path: &PathBuf, needle: &str) -> bool {
        wait_for(
            || {
                std::fs::read_to_string(path)
                    .map(|s| s.contains(needle))
                    .unwrap_or(false)
            },
            Duration::from_secs(5),
        )
    }

    #[test]
    fn daemon_logs_to_file_from_env() {
        use std::os::unix::fs::PermissionsExt as _;

        let paths = TestDaemonPaths::new();
        let cfg_dir = TempDir::new().expect("cfg tempdir");
        let config = write_chatty_config(&cfg_dir);
        let log = paths.sock.parent().expect("parent").join("custom.log");

        let start = mcp_hub()
            .args(["start", "--daemon", "-c", config.path().to_str().unwrap()])
            .envs(paths.env_overrides())
            .env("MCP_HUB_LOG_FILE", &log)
            .output()
            .expect("start daemon");
        let status = mcp_hub()
            .arg("status")
            .envs(paths.env_overrides())
            .output()
            .expect("status");
        let found_listening = log_contains(&log, "Control socket listening");
        let found_child = log_contains(&log, "child-stderr-line");
        let mode = std::fs::metadata(&log).map(|m| m.permissions().mode() & 0o777);

        let _ = mcp_hub().arg("stop").envs(paths.env_overrides()).output();

        assert!(
            start.status.success(),
            "start --daemon should exit 0; stderr: {}",
            String::from_utf8_lossy(&start.stderr)
        );
        let stdout = String::from_utf8_lossy(&start.stdout);
        assert!(
            stdout.contains("Daemon started (pid") && stdout.contains(log.to_str().unwrap()),
            "start should print pid and log path, got: {stdout}"
        );

        assert!(status.status.success(), "status should work");
        assert!(
            found_listening,
            "log should contain the control socket line"
        );
        assert!(found_child, "log should contain the child's stderr line");
        assert_eq!(mode.expect("log file exists"), 0o600, "log must be 0600");
    }

    #[test]
    fn daemon_log_defaults_next_to_socket_and_flag_beats_env() {
        // Default: <socket dir>/daemon.log.
        let paths = TestDaemonPaths::new();
        let cfg_dir = TempDir::new().expect("cfg tempdir");
        let config = write_chatty_config(&cfg_dir);
        let default_log = paths.sock.parent().expect("parent").join("daemon.log");

        let start = mcp_hub()
            .args(["start", "--daemon", "-c", config.path().to_str().unwrap()])
            .envs(paths.env_overrides())
            .output()
            .expect("start daemon");
        let default_ok = start.status.success() && log_contains(&default_log, "Control socket");
        let _ = mcp_hub().arg("stop").envs(paths.env_overrides()).output();
        assert!(default_ok, "default log should be <socket dir>/daemon.log");
        wait_for(|| !paths.sock.exists(), Duration::from_secs(8));

        // --log-file wins over MCP_HUB_LOG_FILE.
        let flag_log = paths.sock.parent().expect("parent").join("flag.log");
        let env_log = paths.sock.parent().expect("parent").join("env.log");
        let start = mcp_hub()
            .args([
                "start",
                "--daemon",
                "-c",
                config.path().to_str().unwrap(),
                "--log-file",
                flag_log.to_str().unwrap(),
            ])
            .envs(paths.env_overrides())
            .env("MCP_HUB_LOG_FILE", &env_log)
            .output()
            .expect("start daemon with flag");
        let flag_ok = start.status.success() && log_contains(&flag_log, "Control socket");
        let _ = mcp_hub().arg("stop").envs(paths.env_overrides()).output();
        assert!(flag_ok, "--log-file should be used");
        assert!(
            !env_log.exists(),
            "env log must be ignored when flag is set"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // #20: `reload` returns the real outcome
    // ─────────────────────────────────────────────────────────────────────────

    /// Config text with the given server names (each a `sleep` server) on `port`.
    fn config_text(port: u16, servers: &[&str]) -> String {
        let mut text = format!("[hub]\nweb_port = {port}\n");
        for name in servers {
            text.push_str(&format!(
                "\n[servers.{name}]\ncommand = \"sleep\"\nargs = [\"9999\"]\ntransport = \"stdio\"\n"
            ));
        }
        text
    }

    #[test]
    fn reload_reports_parse_error_then_summary() {
        let paths = TestDaemonPaths::new();
        let cfg_dir = TempDir::new().expect("cfg tempdir");
        let cfg_path = cfg_dir.path().join("hub.toml");
        let port = free_port();
        std::fs::write(&cfg_path, config_text(port, &["alpha"])).expect("write config A");

        let start = mcp_hub()
            .args(["start", "--daemon", "-c", cfg_path.to_str().unwrap()])
            .envs(paths.env_overrides())
            .output()
            .expect("start daemon");
        assert!(
            start.status.success(),
            "start failed: {}",
            String::from_utf8_lossy(&start.stderr)
        );

        // 1. Invalid TOML: reload must fail loudly and leave alpha running.
        std::fs::write(&cfg_path, "this is = [not valid toml").expect("write bad config");
        let bad = mcp_hub()
            .arg("reload")
            .envs(paths.env_overrides())
            .output()
            .expect("reload (bad config)");
        let status_after_bad = mcp_hub()
            .arg("status")
            .envs(paths.env_overrides())
            .output()
            .expect("status after bad reload");

        // 2. Valid config adding beta: reload succeeds with a summary.
        std::fs::write(&cfg_path, config_text(port, &["alpha", "beta"])).expect("write config B");
        let good = mcp_hub()
            .arg("reload")
            .envs(paths.env_overrides())
            .output()
            .expect("reload (good config)");
        let status_after_good = mcp_hub()
            .arg("status")
            .envs(paths.env_overrides())
            .output()
            .expect("status after good reload");

        let _ = mcp_hub().arg("stop").envs(paths.env_overrides()).output();

        let bad_stderr = String::from_utf8_lossy(&bad.stderr);
        let bad_stdout = String::from_utf8_lossy(&bad.stdout);
        assert!(
            !bad.status.success(),
            "reload of an invalid config must exit non-zero; stdout: {bad_stdout}"
        );
        assert!(
            bad_stderr.contains("Invalid TOML"),
            "stderr should carry the parse error, got: {bad_stderr}"
        );
        assert!(
            !bad_stdout.contains("Reload signal sent"),
            "must not claim success, got: {bad_stdout}"
        );
        let status_bad = String::from_utf8_lossy(&status_after_bad.stdout);
        assert!(
            status_bad.contains("alpha") && status_bad.to_lowercase().contains("running"),
            "alpha should still be running after a failed reload, got: {status_bad}"
        );

        let good_stdout = String::from_utf8_lossy(&good.stdout);
        assert!(
            good.status.success(),
            "reload of a valid config must exit 0; stderr: {}",
            String::from_utf8_lossy(&good.stderr)
        );
        assert!(
            good_stdout.contains("1 added") && good_stdout.contains("0 removed"),
            "summary should report 1 added / 0 removed, got: {good_stdout}"
        );
        let status_good = String::from_utf8_lossy(&status_after_good.stdout);
        assert!(
            status_good.contains("alpha") && status_good.contains("beta"),
            "status should list both servers after reload, got: {status_good}"
        );
    }

    #[test]
    fn sighup_still_reloads_config() {
        let paths = TestDaemonPaths::new();
        let cfg_dir = TempDir::new().expect("cfg tempdir");
        let cfg_path = cfg_dir.path().join("hub.toml");
        let port = free_port();
        std::fs::write(&cfg_path, config_text(port, &["alpha"])).expect("write config A");

        let start = mcp_hub()
            .args(["start", "--daemon", "-c", cfg_path.to_str().unwrap()])
            .envs(paths.env_overrides())
            .output()
            .expect("start daemon");
        assert!(start.status.success(), "start failed");

        std::fs::write(&cfg_path, config_text(port, &["alpha", "gamma"])).expect("write config B");
        let pid = read_pid(&paths.pid).expect("pid file");
        let _ = std::process::Command::new("kill")
            .args(["-HUP", &pid.to_string()])
            .output();

        let saw_gamma = wait_for(
            || {
                mcp_hub()
                    .arg("status")
                    .envs(paths.env_overrides())
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).contains("gamma"))
                    .unwrap_or(false)
            },
            Duration::from_secs(5),
        );

        let _ = mcp_hub().arg("stop").envs(paths.env_overrides()).output();
        assert!(saw_gamma, "SIGHUP should still apply the new config");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // #22: shutdown must stop servers even with a lingering control client;
    //      a live PID must never be treated as stale
    // ─────────────────────────────────────────────────────────────────────────

    /// True while `pid` is a running process (zombies count as dead).
    fn process_alive(pid: u32) -> bool {
        use nix::sys::signal::kill;
        use nix::unistd::Pid;

        let Ok(raw) = i32::try_from(pid) else {
            return false;
        };
        if kill(Pid::from_raw(raw), None).is_err() {
            return false;
        }
        // A zombie still answers kill(pid, 0); /proc tells them apart on Linux.
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => !stat
                .rsplit(')')
                .next()
                .is_some_and(|rest| rest.trim_start().starts_with('Z')),
            Err(_) => true,
        }
    }

    /// Config with one server that records its PID in `pid_file`, then `exec`s
    /// `sleep` so the recorded PID is the long-lived process.
    ///
    /// With `ignore_term` the server ignores SIGTERM (the disposition survives
    /// `exec`), so only the supervisor's SIGKILL escalation after its 5 s grace
    /// period stops it -- which requires the daemon to wait for its supervisors.
    fn write_pid_recording_config(
        dir: &TempDir,
        pid_file: &std::path::Path,
        ignore_term: bool,
    ) -> PathBuf {
        let cfg = dir.path().join("hub.toml");
        let trap = if ignore_term { "trap \"\" TERM; " } else { "" };
        std::fs::write(
            &cfg,
            format!(
                "[hub]\nweb_port = {}\n\n[servers.recorder]\ncommand = \"bash\"\n\
                 args = ['-c', '{}echo $$ > {}; exec sleep 600']\ntransport = \"stdio\"\n",
                free_port(),
                trap,
                pid_file.display()
            ),
        )
        .expect("write config");
        cfg
    }

    #[test]
    fn stop_stops_servers_even_with_idle_control_client() {
        let paths = TestDaemonPaths::new();
        let cfg_dir = TempDir::new().expect("cfg tempdir");
        let server_pid_file = cfg_dir.path().join("server.pid");
        let cfg = write_pid_recording_config(&cfg_dir, &server_pid_file, true);

        let start = mcp_hub()
            .args(["start", "--daemon", "-c", cfg.to_str().unwrap()])
            .envs(paths.env_overrides())
            .output()
            .expect("start daemon");
        assert!(
            start.status.success(),
            "start failed: {}",
            String::from_utf8_lossy(&start.stderr)
        );

        assert!(
            wait_for(
                || read_pid(&server_pid_file).is_some(),
                Duration::from_secs(5)
            ),
            "managed server never recorded its PID"
        );
        let server_pid = read_pid(&server_pid_file).expect("server pid");
        let daemon_pid = read_pid(&paths.pid).expect("daemon pid");

        // A client that connects and never sends a request keeps a
        // `DaemonState` clone alive inside the daemon.
        let idle = std::os::unix::net::UnixStream::connect(&paths.sock).expect("idle client");

        let stop = mcp_hub()
            .arg("stop")
            .envs(paths.env_overrides())
            .output()
            .expect("mcp-hub stop");

        let daemon_gone = wait_for(|| !process_alive(daemon_pid), Duration::from_secs(8));
        let server_gone = wait_for(|| !process_alive(server_pid), Duration::from_secs(8));
        let pid_file_gone = wait_for(|| !paths.pid.exists(), Duration::from_secs(2));

        // Do not leave orphans behind if an assertion below fails.
        if !server_gone {
            let _ = std::process::Command::new("kill")
                .args(["-9", &server_pid.to_string()])
                .output();
        }
        if !daemon_gone {
            let _ = std::process::Command::new("kill")
                .args(["-9", &daemon_pid.to_string()])
                .output();
        }
        drop(idle);

        assert!(
            stop.status.success(),
            "stop should exit 0; stderr: {}",
            String::from_utf8_lossy(&stop.stderr)
        );
        assert!(daemon_gone, "daemon process should exit after stop");
        assert!(server_gone, "managed server must be stopped on shutdown");
        assert!(pid_file_gone, "PID file should be removed after stop");
    }

    /// Number of processes whose command line contains `needle`.
    fn count_processes_matching(needle: &str) -> Option<usize> {
        let out = std::process::Command::new("pgrep")
            .args(["-f", "-c", needle])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    }

    #[test]
    fn live_daemon_with_missing_socket_blocks_second_start() {
        let paths = TestDaemonPaths::new();
        let cfg_dir = TempDir::new().expect("cfg tempdir");
        let server_pid_file = cfg_dir.path().join("server.pid");
        let cfg = write_pid_recording_config(&cfg_dir, &server_pid_file, false);
        let cfg_str = cfg.to_str().unwrap().to_string();

        let start = mcp_hub()
            .args(["start", "--daemon", "-c", &cfg_str])
            .envs(paths.env_overrides())
            .output()
            .expect("start daemon");
        assert!(start.status.success(), "first start failed");
        let daemon_pid = read_pid(&paths.pid).expect("daemon pid");

        // Simulate the stale-socket case: PID alive, socket file gone.
        std::fs::remove_file(&paths.sock).expect("remove socket");

        let second = mcp_hub()
            .args(["start", "--daemon", "-c", &cfg_str])
            .envs(paths.env_overrides())
            .output()
            .expect("second start");

        // Give a wrongly-started second daemon time to show up.
        std::thread::sleep(Duration::from_millis(500));
        let sock_recreated = paths.sock.exists();
        let pid_after = read_pid(&paths.pid);
        let daemon_alive = process_alive(daemon_pid);
        let matching = count_processes_matching(&cfg_str);

        // The socket is gone, so `stop` cannot reach the daemon: use SIGTERM.
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &daemon_pid.to_string()])
            .output();
        let exited = wait_for(|| !process_alive(daemon_pid), Duration::from_secs(8));
        if !exited {
            let _ = std::process::Command::new("kill")
                .args(["-9", &daemon_pid.to_string()])
                .output();
        }

        assert!(
            !second.status.success(),
            "second start must fail while the first daemon's PID is alive"
        );
        let stderr = String::from_utf8_lossy(&second.stderr);
        assert!(
            stderr.contains("already running") && stderr.contains(&daemon_pid.to_string()),
            "stderr should name the running daemon's PID, got: {stderr}"
        );
        assert!(!sock_recreated, "a second daemon bound a new socket");
        assert_eq!(pid_after, Some(daemon_pid), "PID file must be untouched");
        assert!(daemon_alive, "original daemon must keep running");
        if let Some(n) = matching {
            assert_eq!(n, 1, "exactly one daemon process may use this config");
        }
        assert!(exited, "daemon should exit on SIGTERM");
    }
}
