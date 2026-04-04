// Functions are wired in main.rs (Task 4 of plan 03-03); suppress dead_code until then.
#![allow(dead_code)]
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
/// Creates the parent directory if it does not already exist.
pub fn socket_path() -> anyhow::Result<PathBuf> {
    let config_dir =
        dirs::config_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine config directory"))?;
    let dir = config_dir.join("mcp-hub");
    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("Failed to create {}: {e}", dir.display()))?;
    Ok(dir.join("mcp-hub.sock"))
}

/// Resolve the daemon PID file path: `~/.config/mcp-hub/mcp-hub.pid`
pub fn pid_path() -> anyhow::Result<PathBuf> {
    let config_dir =
        dirs::config_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine config directory"))?;
    Ok(config_dir.join("mcp-hub").join("mcp-hub.pid"))
}

// ─────────────────────────────────────────────────────────────────────────────
// PID file management
// ─────────────────────────────────────────────────────────────────────────────

/// Write the current process ID to `path`.
pub fn write_pid_file(path: &std::path::Path) -> anyhow::Result<()> {
    std::fs::write(path, std::process::id().to_string())
        .map_err(|e| anyhow::anyhow!("Failed to write PID file {}: {e}", path.display()))
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
/// - If the connect succeeds, a live daemon is running — returns an error.
/// - If the connect fails, there is no live daemon; stale socket/PID files
///   are cleaned up and `Ok(())` is returned.
///
/// This must be called *before* `daemonize_process()` and *before* the Tokio
/// runtime is created.
#[cfg(unix)]
pub fn check_existing_daemon(
    sock_path: &std::path::Path,
    pid_path: &std::path::Path,
) -> anyhow::Result<()> {
    use std::os::unix::net::UnixStream as StdUnixStream;

    match StdUnixStream::connect(sock_path) {
        Ok(_) => {
            // Socket is live — a daemon is already running.
            anyhow::bail!(
                "A daemon is already running (socket: {}). Use `mcp-hub stop` to stop it.",
                sock_path.display()
            );
        }
        Err(_) => {
            // Socket not connectable. Remove any stale files.
            cleanup_stale_files(sock_path, pid_path);
            Ok(())
        }
    }
}

/// Remove stale socket and PID files left behind by a dead daemon process.
///
/// Uses `kill(pid, 0)` to check whether the process is still alive:
/// - `ESRCH` → process is dead → remove both files.
/// - `Ok(())` → process is alive but socket is broken → remove socket only.
/// - Other errors → remove both files to be safe.
#[cfg(unix)]
fn cleanup_stale_files(sock_path: &std::path::Path, pid_path: &std::path::Path) {
    if let Ok(pid_str) = std::fs::read_to_string(pid_path) {
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            use nix::sys::signal::kill;
            use nix::unistd::Pid;

            match kill(Pid::from_raw(pid), None) {
                Err(nix::errno::Errno::ESRCH) => {
                    // Process is dead — remove both stale files.
                    tracing::info!("Cleaning up stale daemon files (PID {pid} is dead)");
                    let _ = std::fs::remove_file(sock_path);
                    let _ = std::fs::remove_file(pid_path);
                }
                Ok(()) => {
                    // Process is alive but socket is not connectable — unusual.
                    // Remove stale socket so we can bind a new one.
                    tracing::warn!(
                        "PID {pid} is alive but socket is not connectable — removing stale socket"
                    );
                    let _ = std::fs::remove_file(sock_path);
                }
                Err(_) => {
                    // Permission or other error — clean up both to be safe.
                    let _ = std::fs::remove_file(sock_path);
                    let _ = std::fs::remove_file(pid_path);
                }
            }
            return;
        }
    }

    // No PID file (or parse error) — remove the stale socket if it exists.
    if sock_path.exists() {
        let _ = std::fs::remove_file(sock_path);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Fork / daemonize
// ─────────────────────────────────────────────────────────────────────────────

/// Fork the current process into a background daemon.
///
/// Performs the classic double-fork daemonization:
/// 1. First fork — the parent exits so the shell regains control.
/// 2. `setsid()` — create a new session so the daemon has no controlling terminal.
/// 3. Second fork — ensures the daemon cannot re-acquire a controlling terminal.
/// 4. Redirects stdin/stdout/stderr to `/dev/null`.
///
/// **Must be called before the Tokio runtime is created** — forking after
/// threads are running leads to undefined behaviour.
///
/// Note: `nix::unistd::daemon()` is only available on Linux/FreeBSD/Solaris;
/// this implementation works on any Unix including macOS.
#[cfg(unix)]
pub fn daemonize_process() -> anyhow::Result<()> {
    use nix::unistd::{fork, setsid, ForkResult};

    // First fork — parent exits, child continues.
    // SAFETY: called before Tokio runtime is created; no threads exist yet.
    match unsafe { fork() }.map_err(|e| anyhow::anyhow!("First fork failed: {e}"))? {
        ForkResult::Parent { .. } => {
            // Parent exits cleanly — shell regains control.
            std::process::exit(0);
        }
        ForkResult::Child => {}
    }

    // Create a new session — detach from the controlling terminal.
    setsid().map_err(|e| anyhow::anyhow!("setsid failed: {e}"))?;

    // Second fork — prevents the daemon from re-acquiring a controlling terminal.
    match unsafe { fork() }.map_err(|e| anyhow::anyhow!("Second fork failed: {e}"))? {
        ForkResult::Parent { .. } => {
            // Intermediate parent exits.
            std::process::exit(0);
        }
        ForkResult::Child => {}
    }

    // Redirect stdin/stdout/stderr to /dev/null.
    redirect_to_dev_null()?;

    Ok(())
}

/// Redirect the standard file descriptors to `/dev/null`.
#[cfg(unix)]
fn redirect_to_dev_null() -> anyhow::Result<()> {
    use std::fs::OpenOptions;
    use std::os::unix::io::IntoRawFd;

    let devnull = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")
        .map_err(|e| anyhow::anyhow!("Failed to open /dev/null: {e}"))?;

    let fd = devnull.into_raw_fd();

    // SAFETY: dup2 is safe when both fds are valid.
    for target_fd in [0i32, 1, 2] {
        if unsafe { nix::libc::dup2(fd, target_fd) } == -1 {
            return Err(anyhow::anyhow!(
                "dup2(/dev/null, {target_fd}) failed: {}",
                std::io::Error::last_os_error()
            ));
        }
    }

    // Close the extra fd (only if it is not one of the standard ones).
    if fd > 2 {
        unsafe { nix::libc::close(fd) };
    }

    Ok(())
}

/// Daemon mode is not supported on Windows.
#[cfg(not(unix))]
pub fn daemonize_process() -> anyhow::Result<()> {
    anyhow::bail!("Daemon mode is not supported on Windows. Use foreground mode instead.")
}
