//! `mcp-hub logs --follow` streaming tests (issue #16). Unix only.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Write as _};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use tempfile::TempDir;

fn bin() -> PathBuf {
    assert_cmd::cargo::cargo_bin("mcp-hub")
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

/// An isolated daemon; stopped (and its servers killed) on drop.
struct TestHub {
    dir: TempDir,
}

impl TestHub {
    /// `servers` is a list of (name, bash script).
    fn start(servers: &[(&str, &str)]) -> Self {
        let dir = TempDir::new().expect("tempdir");
        let mut cfg = format!("[hub]\nweb_port = {}\n", free_port());
        for (name, script) in servers {
            cfg.push_str(&format!(
                "\n[servers.{name}]\ncommand = \"bash\"\nargs = [\"-c\", {script:?}]\ntransport = \"stdio\"\n"
            ));
        }
        let cfg_path = dir.path().join("mcp-hub.toml");
        std::fs::File::create(&cfg_path)
            .and_then(|mut f| f.write_all(cfg.as_bytes()))
            .expect("write config");
        let hub = Self { dir };
        let out = hub
            .cmd()
            .args(["start", "--daemon", "-c"])
            .arg(&cfg_path)
            .output()
            .expect("start");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !hub.sock().exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        hub
    }

    fn sock(&self) -> PathBuf {
        self.dir.path().join("s.sock")
    }

    fn cmd(&self) -> Command {
        let mut c = Command::new(bin());
        c.env("MCP_HUB_SOCKET", self.sock())
            .env("MCP_HUB_PID", self.dir.path().join("s.pid"))
            .env("MCP_HUB_LOG_FILE", self.dir.path().join("d.log"));
        c
    }

    fn follow(&self, extra: &[&str]) -> Follower {
        let mut child = self
            .cmd()
            .args(["logs", "--follow"])
            .args(extra)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn follower");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Follower { child, rx }
    }
}

impl Drop for TestHub {
    fn drop(&mut self) {
        let _ = self.cmd().arg("stop").output();
    }
}

struct Follower {
    child: Child,
    rx: mpsc::Receiver<String>,
}

impl Follower {
    /// Wait for a line containing `needle`; returns all lines seen on success.
    fn expect_line(&self, needle: &str, within: Duration) -> Result<Vec<String>, Vec<String>> {
        let deadline = Instant::now() + within;
        let mut seen = Vec::new();
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.rx.recv_timeout(left) {
                Ok(l) => {
                    let hit = l.contains(needle);
                    seen.push(l);
                    if hit {
                        return Ok(seen);
                    }
                }
                Err(_) => break,
            }
        }
        Err(seen)
    }

    fn wait_exit(&mut self, within: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if let Ok(Some(st)) = self.child.try_wait() {
                return Some(st);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }
}

impl Drop for Follower {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

const SLOW: &str = "echo first >&2; sleep 1; echo second >&2; sleep 600";

#[test]
fn follow_streams_snapshot_then_live_lines() {
    let hub = TestHub::start(&[("a", SLOW)]);
    let f = hub.follow(&[]);
    f.expect_line("first", Duration::from_secs(5))
        .expect("snapshot line `first`");
    f.expect_line("second", Duration::from_secs(8))
        .expect("live line `second` (follow must keep streaming)");
}

#[test]
fn follow_server_filter_excludes_other_servers() {
    let hub = TestHub::start(&[
        ("a", "echo from-a >&2; sleep 1; echo live-a >&2; sleep 600"),
        ("b", "echo from-b >&2; sleep 1; echo live-b >&2; sleep 600"),
    ]);
    let f = hub.follow(&["--server", "a"]);
    let mut all = f
        .expect_line("live-a", Duration::from_secs(8))
        .expect("live line from a");
    // Give b's live line time to (not) arrive.
    std::thread::sleep(Duration::from_millis(500));
    while let Ok(l) = f.rx.try_recv() {
        all.push(l);
    }
    assert!(all.iter().any(|l| l.contains("from-a")), "{all:?}");
    assert!(
        all.iter().all(|l| !l.contains("-b")),
        "server b leaked into filtered follow: {all:?}"
    );
}

#[test]
fn follow_unknown_server_errors() {
    let hub = TestHub::start(&[("a", SLOW)]);
    let out = hub
        .cmd()
        .args(["logs", "--follow", "--server", "nope"])
        .output()
        .expect("run");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("Unknown server") && err.contains("nope"),
        "{err}"
    );
}

#[test]
fn follow_without_daemon_fails_like_logs() {
    let dir = TempDir::new().expect("tempdir");
    let out = Command::new(bin())
        .env("MCP_HUB_SOCKET", dir.path().join("none.sock"))
        .env("MCP_HUB_PID", dir.path().join("none.pid"))
        .env("MCP_HUB_LOG_FILE", dir.path().join("d.log"))
        .args(["logs", "--follow"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Cannot connect"));
}

#[test]
fn daemon_stop_closes_follower_and_stops_servers() {
    // Unique marker so pgrep only matches this test's server process.
    let tag = format!(
        "tag{}x{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    );
    let script = format!("echo first >&2; sleep 600; : {tag}");
    let hub = TestHub::start(&[("a", script.as_str())]);
    let pgrep = || {
        Command::new("pgrep")
            .args(["-f", &tag])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    let mut f = hub.follow(&[]);
    f.expect_line("first", Duration::from_secs(5))
        .expect("first");
    assert!(pgrep(), "server process should be running before stop");

    let out = hub.cmd().arg("stop").output().expect("stop");
    assert!(out.status.success());

    let st = f
        .wait_exit(Duration::from_secs(5))
        .expect("follower must exit when the daemon shuts down");
    assert!(st.success(), "{st:?}");

    let deadline = Instant::now() + Duration::from_secs(10);
    while (hub.sock().exists() || pgrep()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!hub.sock().exists(), "daemon did not shut down");
    assert!(!pgrep(), "managed server survived daemon stop");
}
