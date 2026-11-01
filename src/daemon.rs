/// Daemon mode helpers: PID file management, socket path resolution,
/// duplicate daemon detection, stale file cleanup, and process forking.
///
/// All functions in this module are safe to call from a synchronous context
/// *before* the Tokio runtime is created — this is required because `fork(2)`
/// must happen before any threads (including Tokio threads) are spawned.
use std::path::PathBuf;

// ─────────────────────────────────────────────────────────────────────────────
// Path resolution
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve the daemon Unix socket path: `~/.config/mcp-hub/mcp-hub.sock`
///
/// The path can be overridden with the `MCP_HUB_SOCKET` environment variable.
/// This override is used by integration tests to isolate daemon instances.
///
/// Creates the parent directory if it does not already exist.
pub fn socket_path() -> anyhow::Result<PathBuf> {
    if let Ok(override_path) = std::env::var("MCP_HUB_SOCKET") {
        let p = PathBuf::from(override_path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("Failed to create {}: {e}", parent.display()))?;
        }
        return Ok(p);
    }
    let config_dir =
        dirs::config_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine config directory"))?;
    let dir = config_dir.join("mcp-hub");
    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("Failed to create {}: {e}", dir.display()))?;
    Ok(dir.join("mcp-hub.sock"))
}

/// Resolve the daemon log file path.
///
/// `explicit` (from `--log-file` / `MCP_HUB_LOG_FILE`) wins; otherwise the log
/// lives next to the control socket as `daemon.log`, so a `MCP_HUB_SOCKET`
/// override keeps test daemons isolated.
pub fn log_path(explicit: Option<&std::path::Path>) -> anyhow::Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    let sock = socket_path()?;
    let dir = sock
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Socket path {} has no parent", sock.display()))?;
    Ok(dir.join("daemon.log"))
}

/// Open (creating if needed) the daemon log file for appending, mode 0600.
///
/// The mode is enforced on pre-existing files too, since the log can contain
/// MCP server output.
#[cfg(unix)]
pub fn open_log_file(path: &std::path::Path) -> anyhow::Result<std::fs::File> {
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("Failed to create {}: {e}", parent.display()))?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| anyhow::anyhow!("Failed to open log file {}: {e}", path.display()))?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|e| anyhow::anyhow!("Failed to chmod log file {}: {e}", path.display()))?;
    Ok(file)
}

/// Resolve the daemon PID file path: `~/.config/mcp-hub/mcp-hub.pid`
///
/// The path can be overridden with the `MCP_HUB_PID` environment variable.
/// This override is used by integration tests to isolate daemon instances.
pub fn pid_path() -> anyhow::Result<PathBuf> {
    if let Ok(override_path) = std::env::var("MCP_HUB_PID") {
        return Ok(PathBuf::from(override_path));
    }
    let config_dir =
        dirs::config_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine config directory"))?;
    Ok(config_dir.join("mcp-hub").join("mcp-hub.pid"))
}

// ─────────────────────────────────────────────────────────────────────────────
// PID file management
// ─────────────────────────────────────────────────────────────────────────────

/// An exclusive advisory lock on the PID file, held for the daemon's lifetime.
///
/// The lock lives on the open file description, so it is released when this
/// guard is dropped or the process dies (even via `SIGKILL`). The descriptor is
/// `O_CLOEXEC`, so managed servers never inherit it.
#[cfg(unix)]
#[derive(Debug)]
pub struct PidFileLock {
    _file: nix::fcntl::Flock<std::fs::File>,
}

/// Non-Unix stub: there is no daemon mode, hence nothing to lock.
#[cfg(not(unix))]
#[derive(Debug)]
pub struct PidFileLock;

/// Take an exclusive, non-blocking `flock` on `file`.
///
/// `Ok(None)` means another process holds the lock.
#[cfg(unix)]
fn try_lock_exclusive(
    file: std::fs::File,
) -> std::io::Result<Option<nix::fcntl::Flock<std::fs::File>>> {
    use nix::errno::Errno;
    use nix::fcntl::{Flock, FlockArg};

    match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
        Ok(lock) => Ok(Some(lock)),
        Err((_, Errno::EWOULDBLOCK)) => Ok(None),
        Err((_, errno)) => Err(std::io::Error::from(errno)),
    }
}

/// Write the current process ID to `path` and lock it exclusively.
///
/// The returned guard must be kept alive for as long as the daemon runs: it is
/// what makes a second daemon (and `check_existing_daemon`) see this one as
/// running. Fails if another process already holds the lock; the existing
/// file's contents are left untouched in that case.
#[cfg(unix)]
pub fn write_pid_file(path: &std::path::Path) -> anyhow::Result<PidFileLock> {
    use std::io::Write as _;
    use std::os::unix::fs::MetadataExt as _;

    let fail = |e: &dyn std::fmt::Display| {
        anyhow::anyhow!("Failed to write PID file {}: {e}", path.display())
    };

    // Retry if the file is unlinked between our open and our lock (a daemon
    // exiting removes its PID file): a lock on an unlinked inode protects nothing.
    for _ in 0..5 {
        // No truncation before the lock is held: a losing writer must not
        // clobber the running daemon's PID.
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .map_err(|e| fail(&e))?;
        let Some(mut lock) = try_lock_exclusive(file).map_err(|e| fail(&e))? else {
            anyhow::bail!(
                "Failed to write PID file {}: another daemon holds the lock \
                 (already running)",
                path.display()
            );
        };

        let locked = lock.metadata().map_err(|e| fail(&e))?;
        let linked = match std::fs::metadata(path) {
            Ok(m) => m.dev() == locked.dev() && m.ino() == locked.ino(),
            Err(_) => false,
        };
        if !linked {
            continue;
        }

        lock.set_len(0).map_err(|e| fail(&e))?;
        lock.write_all(std::process::id().to_string().as_bytes())
            .map_err(|e| fail(&e))?;
        lock.flush().map_err(|e| fail(&e))?;
        return Ok(PidFileLock { _file: lock });
    }
    anyhow::bail!(
        "Failed to write PID file {}: file kept disappearing while locking",
        path.display()
    )
}

/// Non-Unix stub: write the PID without locking.
#[cfg(not(unix))]
pub fn write_pid_file(path: &std::path::Path) -> anyhow::Result<PidFileLock> {
    std::fs::write(path, std::process::id().to_string())
        .map_err(|e| anyhow::anyhow!("Failed to write PID file {}: {e}", path.display()))?;
    Ok(PidFileLock)
}

/// Remove the PID file at `path`, silently ignoring errors.
pub fn remove_pid_file(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

// ─────────────────────────────────────────────────────────────────────────────
// Duplicate daemon detection and stale file cleanup (Unix only)
// ─────────────────────────────────────────────────────────────────────────────

/// Check whether a daemon is already running by attempting a synchronous
/// connect to `sock_path`.
///
/// A daemon counts as running when any of these holds:
/// - another process holds the exclusive lock on the PID file (see
///   [`write_pid_file`]),
/// - the control socket accepts a connection, or
/// - the PID recorded in the PID file is alive **and** is (or may be) an
///   `mcp-hub` process — a live PID that demonstrably belongs to something
///   else was reused after a reboot and is treated as stale.
///
/// In those cases an error naming the PID and socket is returned and nothing
/// is deleted. Only when none holds (dead or foreign PID, unreadable PID file,
/// or no PID file) are the leftover socket and PID file removed and `Ok(())`
/// returned.
///
/// This must be called *before* `daemonize_process()` and *before* the Tokio
/// runtime is created. The daemon itself closes the remaining race by locking
/// the PID file in [`write_pid_file`], which fails if another daemon won.
#[cfg(unix)]
pub fn check_existing_daemon(
    sock_path: &std::path::Path,
    pid_path: &std::path::Path,
) -> anyhow::Result<()> {
    use std::os::unix::net::UnixStream as StdUnixStream;

    // Hold the lock (if the file exists) while we inspect and clean up, so a
    // daemon that is mid-startup cannot have its files removed under it.
    let _guard = match std::fs::File::open(pid_path) {
        Ok(file) => match try_lock_exclusive(file) {
            Ok(Some(guard)) => Some(guard),
            Ok(None) => anyhow::bail!(
                "A daemon is already running ({}; socket: {}). Use `mcp-hub stop` to stop it.",
                describe_recorded_pid(pid_path),
                sock_path.display()
            ),
            Err(e) => anyhow::bail!("Failed to lock PID file {}: {e}", pid_path.display()),
        },
        Err(_) => None,
    };

    if StdUnixStream::connect(sock_path).is_ok() {
        anyhow::bail!(
            "A daemon is already running (socket: {}). Use `mcp-hub stop` to stop it.",
            sock_path.display()
        );
    }

    if let Some(pid) = read_recorded_pid(pid_path) {
        if pid_is_alive(pid) {
            // Every daemon of this version holds the PID-file lock and a live
            // older daemon answers on its socket, so reaching here with a live
            // PID almost always means the PID was reused after a reboot. Only
            // refuse when the process actually looks like mcp-hub (or we cannot
            // tell).
            match looks_like_mcp_hub(pid) {
                Some(false) => tracing::info!(
                    "PID {pid} from the PID file is alive but is not an mcp-hub process \
                     (reused after a reboot?); treating daemon files as stale"
                ),
                Some(true) | None => anyhow::bail!(
                    "A daemon with PID {pid} is already running (socket: {}, PID file: {}). \
                     Use `mcp-hub stop` to stop it.",
                    sock_path.display(),
                    pid_path.display()
                ),
            }
        } else {
            tracing::info!("Cleaning up stale daemon files (PID {pid} is dead)");
        }
    }

    // No live daemon: remove what a dead one left behind.
    cleanup_stale_files(sock_path, pid_path);
    Ok(())
}

/// The PID recorded in `pid_path`, if the file exists and holds a valid PID.
#[cfg(unix)]
fn read_recorded_pid(pid_path: &std::path::Path) -> Option<i32> {
    std::fs::read_to_string(pid_path)
        .ok()?
        .trim()
        .parse::<i32>()
        // 0 and negative values would make `kill` address process groups.
        .ok()
        .filter(|pid| *pid > 0)
}

/// Human-readable form of the PID recorded in `pid_path`, for error messages.
#[cfg(unix)]
fn describe_recorded_pid(pid_path: &std::path::Path) -> String {
    match read_recorded_pid(pid_path) {
        Some(pid) => format!("PID {pid}"),
        None => format!("PID file {} is locked", pid_path.display()),
    }
}

/// Whether process `pid` exists (`kill(pid, 0)`); `EPERM` still means it exists.
///
/// On Linux a zombie (killed but not yet reaped, e.g. by a slow init) counts as
/// dead: it holds no sockets or locks any more.
#[cfg(unix)]
fn pid_is_alive(pid: i32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    match kill(Pid::from_raw(pid), None) {
        Ok(()) | Err(Errno::EPERM) => !is_zombie(pid),
        Err(Errno::ESRCH) => false,
        // Anything else cannot be a live process we could be racing with.
        Err(_) => false,
    }
}

/// Whether process `pid` is an `mcp-hub` binary, judged by `/proc/<pid>/comm`.
///
/// `None` when that cannot be determined (not Linux, or the entry is
/// unreadable); callers should then err on the side of "running".
#[cfg(unix)]
fn looks_like_mcp_hub(pid: i32) -> Option<bool> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(comm.trim_end().starts_with("mcp-hub"))
}

/// True if `/proc/<pid>/stat` reports state `Z` (always false off Linux).
#[cfg(unix)]
fn is_zombie(pid: i32) -> bool {
    // Format: `pid (comm) S ...`; `comm` may contain spaces and parentheses,
    // so the state follows the *last* `)`.
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            stat.rsplit_once(')')
                .map(|(_, rest)| rest.trim_start().starts_with('Z'))
        })
        .unwrap_or(false)
}

/// Remove the socket and PID file left behind by a dead daemon.
///
/// Callers must have established that no live daemon owns them.
#[cfg(unix)]
fn cleanup_stale_files(sock_path: &std::path::Path, pid_path: &std::path::Path) {
    let _ = std::fs::remove_file(sock_path);
    let _ = std::fs::remove_file(pid_path);
}

// ─────────────────────────────────────────────────────────────────────────────
// Fork / daemonize
// ─────────────────────────────────────────────────────────────────────────────

/// How long the foreground parent waits for the daemon to report readiness.
///
/// Slightly above the 10 s the daemon itself may spend waiting for servers to
/// reach their initial state before it binds its sockets.
pub const READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Write end of the readiness pipe, owned by the daemon child.
///
/// The foreground parent blocks on the read end until the daemon reports that
/// it finished starting (`ready`) or why it could not (`fail`). Dropping the
/// notifier without calling either closes the pipe, which the parent reports
/// as "daemon exited before becoming ready".
#[cfg(unix)]
pub struct ReadyNotifier {
    pipe: std::fs::File,
}

/// Non-Unix stub: daemon mode is unavailable, so there is nothing to notify.
#[cfg(not(unix))]
pub struct ReadyNotifier;

#[cfg(unix)]
impl ReadyNotifier {
    /// Tell the parent the daemon is up (control socket bound, web UI listening).
    ///
    /// The payload is `b"1"` followed by the daemon's PID so the parent can
    /// print it.
    pub fn ready(self) {
        let msg = format!("1{}", std::process::id());
        self.send(msg.as_bytes());
    }

    /// Tell the parent startup failed; `message` is printed on the parent's stderr.
    pub fn fail(self, message: &str) {
        let mut payload = Vec::with_capacity(message.len() + 1);
        payload.push(b'E');
        payload.extend_from_slice(message.as_bytes());
        self.send(&payload);
    }

    fn send(mut self, payload: &[u8]) {
        use std::io::Write as _;
        // The parent may already have timed out and gone away; nothing to do then.
        let _ = self.pipe.write_all(payload);
    }
}

#[cfg(not(unix))]
impl ReadyNotifier {
    /// No-op on platforms without daemon support.
    pub fn ready(self) {}
    /// No-op on platforms without daemon support.
    pub fn fail(self, _message: &str) {}
}

/// Outcome of waiting on the readiness pipe, as seen by the foreground parent.
#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
enum ReadyOutcome {
    Ready { pid: String },
    Failed(String),
    ExitedEarly,
    TimedOut,
}

/// Interpret the bytes a daemon wrote to the readiness pipe.
#[cfg(unix)]
fn parse_ready_message(bytes: &[u8]) -> ReadyOutcome {
    match bytes.split_first() {
        None => ReadyOutcome::ExitedEarly,
        Some((b'1', rest)) => ReadyOutcome::Ready {
            pid: String::from_utf8_lossy(rest).into_owned(),
        },
        Some((b'E', rest)) => ReadyOutcome::Failed(String::from_utf8_lossy(rest).into_owned()),
        Some((_, _)) => ReadyOutcome::Failed(format!(
            "unexpected readiness message: {}",
            String::from_utf8_lossy(bytes)
        )),
    }
}

/// Read the daemon's readiness message from `pipe`, giving up after `timeout`.
#[cfg(unix)]
fn wait_for_ready(pipe: &mut std::fs::File, timeout: std::time::Duration) -> ReadyOutcome {
    use std::io::Read as _;
    use std::os::unix::io::AsRawFd as _;

    let deadline = std::time::Instant::now() + timeout;
    let mut buf = Vec::new();

    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return ReadyOutcome::TimedOut;
        }
        let mut pfd = nix::libc::pollfd {
            fd: pipe.as_raw_fd(),
            events: nix::libc::POLLIN,
            revents: 0,
        };
        let millis = remaining.as_millis().min(i32::MAX as u128) as nix::libc::c_int;
        // SAFETY: `pfd` is a valid pollfd for the duration of the call and nfds is 1.
        let rc = unsafe { nix::libc::poll(&mut pfd, 1, millis) };
        if rc < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return ReadyOutcome::Failed(format!(
                "waiting for the daemon failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        if rc == 0 {
            return ReadyOutcome::TimedOut;
        }
        let mut chunk = [0u8; 1024];
        match pipe.read(&mut chunk) {
            // EOF: the writer closed the pipe, the message (if any) is complete.
            Ok(0) => return parse_ready_message(&buf),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return ReadyOutcome::Failed(format!("reading daemon readiness failed: {e}")),
        }
    }
}

/// Fork the current process into a background daemon.
///
/// Performs the classic double-fork daemonization:
/// 1. First fork — the original process stays in the foreground and **waits for
///    the daemon to report readiness** over a pipe, then exits 0 (ready) or 1
///    (startup failed / timed out), relaying the daemon's error to its stderr.
/// 2. `setsid()` — create a new session so the daemon has no controlling terminal.
/// 3. Second fork — ensures the daemon cannot re-acquire a controlling terminal.
/// 4. Redirects stdin to `/dev/null` and stdout/stderr to the log file at
///    `log_path` (opened before forking so problems surface in the terminal).
///
/// Only the daemon returns from this function; it receives the
/// [`ReadyNotifier`] it must call once startup finished (or failed).
///
/// **Must be called before the Tokio runtime is created** — forking after
/// threads are running leads to undefined behaviour.
///
/// Note: `nix::unistd::daemon()` is only available on Linux/FreeBSD/Solaris;
/// this implementation works on any Unix including macOS.
#[cfg(unix)]
pub fn daemonize_process(log_path: &std::path::Path) -> anyhow::Result<ReadyNotifier> {
    use nix::fcntl::OFlag;
    use nix::unistd::{fork, pipe2, ForkResult};

    // Open the log before forking: a bad path is reported on the terminal.
    let log_file = open_log_file(log_path)?;

    // CLOEXEC so the managed MCP servers never inherit the pipe and keep the
    // parent waiting after the daemon is done with it.
    let (read_fd, write_fd) =
        pipe2(OFlag::O_CLOEXEC).map_err(|e| anyhow::anyhow!("pipe failed: {e}"))?;

    // First fork — parent waits for readiness, child continues.
    // SAFETY: called before Tokio runtime is created; no threads exist yet.
    match unsafe { fork() }.map_err(|e| anyhow::anyhow!("First fork failed: {e}"))? {
        ForkResult::Parent { .. } => {
            // Close our copy of the write end so EOF is observable.
            drop(write_fd);
            let mut pipe = std::fs::File::from(read_fd);
            let code = match wait_for_ready(&mut pipe, READY_TIMEOUT) {
                ReadyOutcome::Ready { pid } => {
                    println!("Daemon started (pid {pid}), logs: {}", log_path.display());
                    0
                }
                ReadyOutcome::Failed(message) => {
                    eprintln!("Error: {message}");
                    1
                }
                ReadyOutcome::ExitedEarly => {
                    eprintln!("Error: daemon exited before becoming ready");
                    1
                }
                ReadyOutcome::TimedOut => {
                    eprintln!(
                        "Error: daemon did not become ready within {}s",
                        READY_TIMEOUT.as_secs()
                    );
                    1
                }
            };
            std::process::exit(code);
        }
        ForkResult::Child => drop(read_fd),
    }

    let notifier = ReadyNotifier {
        pipe: std::fs::File::from(write_fd),
    };

    // Everything between here and the return reports failures to the parent
    // before propagating them, since stderr is about to disappear.
    match detach_from_terminal(&log_file) {
        Ok(()) => Ok(notifier),
        Err(e) => {
            notifier.fail(&format!("{e:#}"));
            Err(e)
        }
    }
}

/// `setsid`, second fork, and stdio redirection — the daemon-side half of
/// [`daemonize_process`].
#[cfg(unix)]
fn detach_from_terminal(log_file: &std::fs::File) -> anyhow::Result<()> {
    use nix::unistd::{fork, setsid, ForkResult};

    // Create a new session — detach from the controlling terminal.
    setsid().map_err(|e| anyhow::anyhow!("setsid failed: {e}"))?;

    // Second fork — prevents the daemon from re-acquiring a controlling terminal.
    // SAFETY: still single-threaded; the Tokio runtime has not been built.
    match unsafe { fork() }.map_err(|e| anyhow::anyhow!("Second fork failed: {e}"))? {
        ForkResult::Parent { .. } => {
            // Intermediate parent exits; its copy of the pipe closes with it.
            std::process::exit(0);
        }
        ForkResult::Child => {}
    }

    // stdin from /dev/null; stdout and stderr into the log file.
    redirect_stdio(log_file)
}

/// Point stdin at `/dev/null` and stdout/stderr at `log_file`.
#[cfg(unix)]
fn redirect_stdio(log_file: &std::fs::File) -> anyhow::Result<()> {
    use std::fs::OpenOptions;
    use std::os::unix::io::AsRawFd as _;

    let devnull = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")
        .map_err(|e| anyhow::anyhow!("Failed to open /dev/null: {e}"))?;

    let redirects = [
        (devnull.as_raw_fd(), 0i32),
        (log_file.as_raw_fd(), 1),
        (log_file.as_raw_fd(), 2),
    ];
    for (source_fd, target_fd) in redirects {
        // SAFETY: dup2 is safe when both fds are valid; `devnull` and
        // `log_file` outlive this loop.
        if unsafe { nix::libc::dup2(source_fd, target_fd) } == -1 {
            return Err(anyhow::anyhow!(
                "dup2({source_fd}, {target_fd}) failed: {}",
                std::io::Error::last_os_error()
            ));
        }
    }

    // `devnull` and `log_file` are dropped (closed) by their owners; fds 0-2
    // keep the duplicated descriptions open.
    Ok(())
}

/// Daemon mode is not supported on Windows.
#[cfg(not(unix))]
pub fn daemonize_process(_log_path: &std::path::Path) -> anyhow::Result<ReadyNotifier> {
    anyhow::bail!("Daemon mode is not supported on Windows. Use foreground mode instead.")
}
