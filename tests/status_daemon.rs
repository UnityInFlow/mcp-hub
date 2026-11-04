/// `mcp-hub status` in daemon mode: table rendering, `--json`, and the
/// `gen-config --live` consumer of the same response.
///
/// Every daemon here uses throwaway socket, PID and log paths so a test can
/// never touch the real `~/.config/mcp-hub`.
#[cfg(unix)]
mod status_daemon_tests {
    use std::collections::HashSet;
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    use assert_cmd::Command;
    use tempfile::TempDir;

    /// Port that is never handed out twice within this test binary.
    fn free_port() -> u16 {
        static HANDED_OUT: OnceLock<Mutex<HashSet<u16>>> = OnceLock::new();
        let handed_out = HANDED_OUT.get_or_init(|| Mutex::new(HashSet::new()));
        loop {
            let port = std::net::TcpListener::bind("127.0.0.1:0")
                .expect("bind ephemeral port")
                .local_addr()
                .expect("local addr")
                .port();
            let mut guard = handed_out.lock().expect("port set lock");
            if guard.insert(port) {
                return port;
            }
        }
    }

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

    /// A running daemon with one `sleep` server; stopped on drop.
    struct Daemon {
        _dir: TempDir,
        sock: PathBuf,
        pid: PathBuf,
        log: PathBuf,
        config: PathBuf,
    }

    impl Daemon {
        fn cmd(&self) -> Command {
            let mut cmd = Command::cargo_bin("mcp-hub").expect("mcp-hub binary");
            cmd.env("MCP_HUB_SOCKET", &self.sock)
                .env("MCP_HUB_PID", &self.pid)
                .env("MCP_HUB_LOG_FILE", &self.log)
                .env_remove("NO_COLOR");
            cmd
        }

        fn start() -> Self {
            let dir = TempDir::new().expect("tempdir");
            let config = dir.path().join("mcp-hub.toml");
            let mut f = std::fs::File::create(&config).expect("config file");
            writeln!(
                f,
                "[hub]\nweb_port = {}\n\n[servers.status-server]\ncommand = \"sleep\"\nargs = [\"7777\"]\ntransport = \"stdio\"\n",
                free_port()
            )
            .expect("write config");
            let daemon = Self {
                sock: dir.path().join("mcp-hub.sock"),
                pid: dir.path().join("mcp-hub.pid"),
                log: dir.path().join("mcp-hub.log"),
                config,
                _dir: dir,
            };
            let out = daemon
                .cmd()
                .args(["start", "--daemon", "-c"])
                .arg(&daemon.config)
                .output()
                .expect("start daemon");
            assert!(
                out.status.success(),
                "daemon start failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                wait_for(|| daemon.sock.exists(), Duration::from_secs(5)),
                "socket never appeared"
            );
            // Wait until the supervised server reports `running`.
            let ready = wait_for(
                || {
                    daemon
                        .cmd()
                        .arg("status")
                        .output()
                        .map(|o| String::from_utf8_lossy(&o.stdout).contains("running"))
                        .unwrap_or(false)
                },
                Duration::from_secs(10),
            );
            assert!(ready, "server never reached running");
            daemon
        }
    }

    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.cmd().arg("stop").output();
            let _ = wait_for(|| !self.sock.exists(), Duration::from_secs(8));
        }
    }

    #[test]
    fn status_renders_table_in_daemon_mode() {
        let d = Daemon::start();
        let out = d
            .cmd()
            .args(["--no-color", "status"])
            .output()
            .expect("status");
        assert!(out.status.success());
        let stdout = String::from_utf8_lossy(&out.stdout);
        for header in [
            "Name",
            "State",
            "Health",
            "PID",
            "Uptime",
            "Restarts",
            "Transport",
            "Tools",
        ] {
            assert!(stdout.contains(header), "missing header {header}: {stdout}");
        }
        let row = stdout
            .lines()
            .find(|l| l.contains("status-server"))
            .unwrap_or_else(|| panic!("no server row: {stdout}"));
        assert!(row.contains("running"), "row: {row}");
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        // "" | Name | State | Health | PID | Uptime | ...
        let uptime = cells.get(5).copied().unwrap_or("");
        assert!(
            uptime.len() == 8 && uptime.chars().filter(|c| *c == ':').count() == 2,
            "uptime should be HH:MM:SS, got {uptime:?} in row: {row}"
        );
        assert!(
            !stdout.contains('\u{1b}'),
            "no ANSI escapes with --no-color"
        );
        assert!(
            !stdout.trim_start().starts_with('['),
            "must not be raw JSON"
        );
    }

    #[test]
    fn status_json_has_uptime_secs() {
        let d = Daemon::start();
        let out = d.cmd().args(["status", "--json"]).output().expect("status");
        assert!(out.status.success());
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
        let entry = value.as_array().and_then(|a| a.first()).expect("one entry");
        assert_eq!(entry["name"], "status-server");
        assert_eq!(entry["state"], "running");
        assert!(entry["uptime_secs"].is_u64(), "entry: {entry}");
        assert!(entry["pid"].is_u64());
    }

    #[test]
    fn gen_config_live_still_works() {
        let d = Daemon::start();
        let out = d
            .cmd()
            .args(["gen-config", "--format", "claude", "--live", "-c"])
            .arg(&d.config)
            .output()
            .expect("gen-config");
        assert!(
            out.status.success(),
            "gen-config --live failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains("status-server"));
    }
}
