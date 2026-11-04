//! One daemon per project (issue #21): two configs run two independent daemons,
//! and every client command resolves the daemon from the config it targets.
//!
//! Tests point `MCP_HUB_STATE_DIR` (and `HOME` / `XDG_CONFIG_HOME`) at temp
//! dirs and never set `MCP_HUB_SOCKET`, so the real per-project resolution is
//! exercised without touching a developer's real config dir.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use assert_cmd::Command;
use tempfile::TempDir;

/// A free TCP port that is never handed out twice within this test binary.
fn free_port() -> u16 {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};

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

/// Isolated environment: state dir and fake home.
struct Env {
    state: TempDir,
    home: TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            state: TempDir::new().expect("state dir"),
            home: TempDir::new().expect("home dir"),
        }
    }

    fn cmd(&self, cwd: &Path) -> Command {
        let mut cmd = Command::cargo_bin("mcp-hub").expect("mcp-hub binary");
        cmd.env_remove("MCP_HUB_SOCKET")
            .env_remove("MCP_HUB_PID")
            .env_remove("MCP_HUB_LOG_FILE")
            .env("MCP_HUB_STATE_DIR", self.state.path())
            .env("HOME", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("xdg"))
            .env("NO_COLOR", "true")
            .current_dir(cwd);
        cmd
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> (bool, String, String) {
        let out = self
            .cmd(cwd)
            .args(args)
            .timeout(Duration::from_secs(30))
            .output()
            .expect("run mcp-hub");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

/// A project directory with its own `mcp-hub.toml` and one distinctly named server.
struct Project {
    dir: TempDir,
    config: PathBuf,
}

impl Project {
    fn new(server: &str) -> Self {
        let dir = TempDir::new().expect("project dir");
        let config = dir.path().join("mcp-hub.toml");
        let mut f = std::fs::File::create(&config).expect("create config");
        writeln!(
            f,
            "[hub]\nweb_port = {}\n\n[servers.{server}]\ncommand = \"sleep\"\nargs = [\"9999\"]\ntransport = \"stdio\"\n",
            free_port()
        )
        .expect("write config");
        Self { dir, config }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn cfg(&self) -> &str {
        self.config.to_str().expect("utf-8 path")
    }
}

/// Stops every daemon of an [`Env`] on drop, then kills leftovers by PID file.
struct Cleanup<'a>(&'a Env, Vec<&'a Project>);

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        let elsewhere = TempDir::new().ok();
        for p in &self.1 {
            if let Some(cwd) = &elsewhere {
                let _ = self.0.run(cwd.path(), &["stop", "-c", p.cfg()]);
            }
        }
        if let Ok(entries) = std::fs::read_dir(self.0.state.path()) {
            for entry in entries.flatten() {
                if entry.path().extension().is_some_and(|e| e == "pid") {
                    if let Ok(s) = std::fs::read_to_string(entry.path()) {
                        if let Ok(pid) = s.trim().parse::<i32>() {
                            // SAFETY: plain kill(2) on a PID this test started.
                            unsafe { nix::libc::kill(pid, nix::libc::SIGKILL) };
                        }
                    }
                }
            }
        }
    }
}

fn wait_until<F: Fn() -> bool>(cond: F) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn stopped(env: &Env, p: &Project) -> bool {
    let (ok, _, _) = env.run(p.path(), &["status", "-c", p.cfg()]);
    !ok
}

#[test]
fn two_projects_run_independent_daemons() {
    let env = Env::new();
    let a = Project::new("alpha-server");
    let b = Project::new("beta-server");
    let _cleanup = Cleanup(&env, vec![&a, &b]);
    let elsewhere = TempDir::new().expect("elsewhere");

    let (ok, out, err) = env.run(elsewhere.path(), &["start", "--daemon", "-c", a.cfg()]);
    assert!(ok, "start a failed: {out}{err}");
    let (ok, out, err) = env.run(elsewhere.path(), &["start", "--daemon", "-c", b.cfg()]);
    assert!(
        ok,
        "start b failed (second project must get its own daemon): {out}{err}"
    );

    // Each status shows only its own project's server.
    let (ok, out, err) = env.run(elsewhere.path(), &["status", "-c", a.cfg()]);
    assert!(ok, "status a: {err}");
    assert!(out.contains("alpha-server"), "{out}");
    assert!(!out.contains("beta-server"), "{out}");
    let (ok, out, err) = env.run(elsewhere.path(), &["status", "-c", b.cfg()]);
    assert!(ok, "status b: {err}");
    assert!(out.contains("beta-server"), "{out}");
    assert!(!out.contains("alpha-server"), "{out}");

    // Second start of a running project is refused.
    let (ok, out, err) = env.run(elsewhere.path(), &["start", "--daemon", "-c", a.cfg()]);
    assert!(!ok, "second start of a must fail: {out}");
    assert!(err.contains("already running"), "{err}");

    // Stopping a leaves b alone.
    let (ok, _, err) = env.run(elsewhere.path(), &["stop", "-c", a.cfg()]);
    assert!(ok, "stop a: {err}");
    assert!(wait_until(|| stopped(&env, &a)), "a did not stop");
    let (ok, out, _) = env.run(elsewhere.path(), &["status", "-c", b.cfg()]);
    assert!(
        ok && out.contains("beta-server"),
        "b must keep running: {out}"
    );

    let (ok, _, err) = env.run(elsewhere.path(), &["stop", "-c", b.cfg()]);
    assert!(ok, "stop b: {err}");
    assert!(wait_until(|| stopped(&env, &b)), "b did not stop");
}

#[test]
fn stop_without_config_flag_targets_the_cwd_project() {
    let env = Env::new();
    let a = Project::new("alpha-server");
    let b = Project::new("beta-server");
    let _cleanup = Cleanup(&env, vec![&a, &b]);

    assert!(env.run(a.path(), &["start", "--daemon"]).0, "start a");
    assert!(env.run(b.path(), &["start", "--daemon"]).0, "start b");

    // Plain `status` / `stop` in a's directory resolve a's ./mcp-hub.toml.
    let (ok, out, err) = env.run(a.path(), &["status"]);
    assert!(ok, "status in a: {err}");
    assert!(
        out.contains("alpha-server") && !out.contains("beta-server"),
        "{out}"
    );

    let (ok, _, err) = env.run(a.path(), &["stop"]);
    assert!(ok, "stop in a: {err}");
    assert!(wait_until(|| stopped(&env, &a)), "a did not stop");
    let (ok, out, _) = env.run(b.path(), &["status"]);
    assert!(
        ok && out.contains("beta-server"),
        "b must keep running: {out}"
    );
    assert!(env.run(b.path(), &["stop"]).0, "stop b");
    assert!(wait_until(|| stopped(&env, &b)), "b did not stop");
}

#[test]
fn stop_without_any_config_lists_daemons_or_picks_the_only_one() {
    let env = Env::new();
    let a = Project::new("alpha-server");
    let b = Project::new("beta-server");
    let _cleanup = Cleanup(&env, vec![&a, &b]);
    let elsewhere = TempDir::new().expect("elsewhere");

    // None running: the usual "no daemon" error.
    let (ok, _, err) = env.run(elsewhere.path(), &["stop"]);
    assert!(!ok);
    assert!(err.to_lowercase().contains("daemon"), "{err}");

    assert!(
        env.run(elsewhere.path(), &["start", "--daemon", "-c", a.cfg()])
            .0
    );
    // Exactly one: used, and the config it serves is named on stderr.
    let (ok, out, err) = env.run(elsewhere.path(), &["status"]);
    assert!(ok, "status with one daemon: {err}");
    assert!(out.contains("alpha-server"), "{out}");
    assert!(
        err.contains("mcp-hub.toml"),
        "should name the config: {err}"
    );

    assert!(
        env.run(elsewhere.path(), &["start", "--daemon", "-c", b.cfg()])
            .0
    );
    // Several: ambiguous, error lists each config and asks for -c.
    let (ok, _, err) = env.run(elsewhere.path(), &["stop"]);
    assert!(!ok, "ambiguous stop must fail");
    let a_cfg = std::fs::canonicalize(&a.config).expect("canon a");
    let b_cfg = std::fs::canonicalize(&b.config).expect("canon b");
    assert!(err.contains(&a_cfg.display().to_string()), "{err}");
    assert!(err.contains(&b_cfg.display().to_string()), "{err}");
    assert!(err.contains("-c"), "{err}");

    assert!(env.run(elsewhere.path(), &["stop", "-c", a.cfg()]).0);
    assert!(env.run(elsewhere.path(), &["stop", "-c", b.cfg()]).0);
    assert!(wait_until(|| stopped(&env, &a) && stopped(&env, &b)));
}

#[test]
fn status_prints_daemon_header_but_not_with_json() {
    let env = Env::new();
    let a = Project::new("alpha-server");
    let _cleanup = Cleanup(&env, vec![&a]);

    assert!(env.run(a.path(), &["start", "--daemon"]).0);
    let (ok, out, err) = env.run(a.path(), &["status"]);
    assert!(ok, "{err}");
    let canon = std::fs::canonicalize(&a.config).expect("canon");
    let first = out.lines().next().unwrap_or_default();
    assert!(
        first.starts_with(&format!("Daemon for {} (pid ", canon.display()))
            && first.contains("web UI http://127.0.0.1:"),
        "{first}"
    );
    let (ok, out, err) = env.run(a.path(), &["status", "--json"]);
    assert!(ok, "{err}");
    assert!(
        serde_json::from_str::<serde_json::Value>(&out).is_ok(),
        "{out}"
    );
    assert!(env.run(a.path(), &["stop"]).0);
    assert!(wait_until(|| stopped(&env, &a)));
}

#[test]
fn same_web_port_error_names_web_port_and_config() {
    let env = Env::new();
    let a = Project::new("alpha-server");
    let b = Project::new("beta-server");
    let _cleanup = Cleanup(&env, vec![&a, &b]);
    // Give b the same web port as a.
    let a_text = std::fs::read_to_string(&a.config).expect("read a");
    let port_line = a_text
        .lines()
        .find(|l| l.starts_with("web_port"))
        .expect("port line")
        .to_string();
    let b_text = std::fs::read_to_string(&b.config).expect("read b");
    let new_b: String = b_text
        .lines()
        .map(|l| {
            if l.starts_with("web_port") {
                port_line.as_str()
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&b.config, new_b).expect("write b");

    assert!(env.run(a.path(), &["start", "--daemon"]).0);
    let (ok, _, err) = env.run(b.path(), &["start", "--daemon"]);
    assert!(!ok);
    assert!(err.contains("web_port"), "{err}");
    let b_canon = std::fs::canonicalize(&b.config).expect("canon b");
    assert!(err.contains(&b_canon.display().to_string()), "{err}");
    assert!(env.run(a.path(), &["stop"]).0);
    assert!(wait_until(|| stopped(&env, &a)));
}

#[test]
fn live_legacy_socket_blocks_start_and_is_used_by_clients_with_a_note() {
    let env = Env::new();
    let a = Project::new("alpha-server");
    // Stand-in for a pre-upgrade daemon: something listening on the old socket.
    let legacy = env.state.path().join("mcp-hub.sock");
    let listener = std::os::unix::net::UnixListener::bind(&legacy).expect("bind legacy");
    std::thread::spawn(move || for _conn in listener.incoming() {});

    let (ok, _, err) = env.run(a.path(), &["start", "--daemon"]);
    assert!(!ok, "start must refuse next to a live legacy daemon");
    assert!(
        err.contains("pre-upgrade") && err.contains("mcp-hub stop"),
        "{err}"
    );

    // No per-project socket exists, so clients fall back to the legacy one.
    let (_, _, err) = env.run(a.path(), &["status"]);
    assert!(err.contains("pre-upgrade"), "{err}");
}
