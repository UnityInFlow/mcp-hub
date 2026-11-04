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

/// Create `dir` (and any missing parents) with mode `0700`.
///
/// Directories that already exist are left untouched; only newly created
/// directories get the restrictive mode (subject to the process umask, which
/// can only make it stricter).
fn create_dir_all_private(dir: &std::path::Path) -> anyhow::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| anyhow::anyhow!("Failed to create {}: {e}", dir.display()))
}

/// Ensure `dir` exists and is accessible only by its owner (`0700`).
///
/// Creates the directory with mode `0700` when missing, and tightens the mode
/// of an existing directory that is more permissive. Intended for the mcp-hub
/// state directory that holds the control socket.
pub fn ensure_private_dir(dir: &std::path::Path) -> anyhow::Result<()> {
    create_dir_all_private(dir)?;

    {
        use std::os::unix::fs::PermissionsExt;

        let mode = std::fs::metadata(dir)
            .map_err(|e| anyhow::anyhow!("Failed to stat {}: {e}", dir.display()))?
            .permissions()
            .mode()
            & 0o777;
        if mode != 0o700 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| {
                anyhow::anyhow!(
                    "Failed to restrict {} to owner-only (0700): {e}",
                    dir.display()
                )
            })?;
        }
    }

    Ok(())
}

/// Longest socket path we accept. macOS limits `sun_path` to 104 bytes
/// (Linux: 108); stay safely below both.
const MAX_SOCKET_PATH_BYTES: usize = 100;

/// The per-user state directory holding every project's socket, PID file, meta
/// file and log: `$MCP_HUB_STATE_DIR` if set, else `<config_dir>/mcp-hub`
/// (`~/.config/mcp-hub` on Linux, `~/Library/Application Support/mcp-hub` on
/// macOS). Created with mode `0700`; the default directory is tightened to
/// `0700` if it already exists with looser permissions, while a pre-existing
/// `MCP_HUB_STATE_DIR` is never modified.
pub fn state_dir() -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("MCP_HUB_STATE_DIR").filter(|d| !d.is_empty()) {
        let dir = PathBuf::from(dir);
        create_dir_all_private(&dir)?;
        return Ok(dir);
    }
    let config_dir =
        dirs::config_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine config directory"))?;
    let dir = config_dir.join("mcp-hub");
    ensure_private_dir(&dir)?;
    Ok(dir)
}

/// 64-bit FNV-1a. Unlike `DefaultHasher` it is specified, so a project's key
/// (and therefore its socket name) is the same across Rust versions and builds.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes.iter().fold(OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
    })
}

/// The key identifying a project's daemon: 12 hex characters of the FNV-1a hash
/// of the canonical config path. Symlinks and relative paths that point at the
/// same file give the same key.
pub fn project_key(config_path: &std::path::Path) -> anyhow::Result<String> {
    use std::os::unix::ffi::OsStrExt as _;

    let canonical = std::fs::canonicalize(config_path)
        .map_err(|e| anyhow::anyhow!("Cannot resolve config {}: {e}", config_path.display()))?;
    let hash = fnv1a_64(canonical.as_os_str().as_bytes());
    Ok(format!("{hash:016x}")[..12].to_string())
}

fn check_socket_len(sock: PathBuf) -> anyhow::Result<PathBuf> {
    use std::os::unix::ffi::OsStrExt as _;

    let len = sock.as_os_str().as_bytes().len();
    if len > MAX_SOCKET_PATH_BYTES {
        anyhow::bail!(
            "Control socket path is too long ({len} bytes, limit {MAX_SOCKET_PATH_BYTES}): {}. \
             Set MCP_HUB_STATE_DIR to a shorter directory.",
            sock.display()
        );
    }
    Ok(sock)
}

/// Resolve the daemon Unix socket path for the project `key`:
/// `<state dir>/mcp-hub-<key>.sock`.
///
/// `MCP_HUB_SOCKET` overrides the path entirely (used by integration tests to
/// isolate daemons); its missing parent directory is created with mode `0700`
/// but a pre-existing parent (e.g. `/tmp`) is never modified.
pub fn socket_path(key: &str) -> anyhow::Result<PathBuf> {
    if let Some(p) = override_socket_path()? {
        return Ok(p);
    }
    check_socket_len(state_dir()?.join(format!("mcp-hub-{key}.sock")))
}

/// The `MCP_HUB_SOCKET` override, with its parent directory created.
pub fn override_socket_path() -> anyhow::Result<Option<PathBuf>> {
    let Some(raw) = std::env::var_os("MCP_HUB_SOCKET") else {
        return Ok(None);
    };
    let p = PathBuf::from(raw);
    if let Some(parent) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
        create_dir_all_private(parent)?;
    }
    Ok(Some(p))
}

/// The pre-per-project socket (`<state dir>/mcp-hub.sock`) of a daemon started
/// by an older mcp-hub.
pub fn legacy_socket_path() -> anyhow::Result<PathBuf> {
    Ok(state_dir()?.join("mcp-hub.sock"))
}

/// Whether something accepts connections on the Unix socket at `sock`.
pub fn socket_is_live(sock: &std::path::Path) -> bool {
    std::os::unix::net::UnixStream::connect(sock).is_ok()
}

/// Everything `start --daemon` needs to know about the project it starts.
#[derive(Debug, Clone)]
pub struct StartPaths {
    /// Canonical path of the config file that identifies the project.
    pub config_path: PathBuf,
    pub sock: PathBuf,
    pub pid: PathBuf,
    pub meta: PathBuf,
    pub log: PathBuf,
}

/// Resolve the project (config file) and derive its daemon paths.
///
/// With a `MCP_HUB_SOCKET` / `MCP_HUB_PID` override those win individually.
pub fn start_paths(
    config_flag: Option<&std::path::Path>,
    log_file: Option<&std::path::Path>,
) -> anyhow::Result<StartPaths> {
    let config = crate::config::locate_config(config_flag)?.ok_or_else(|| {
        anyhow::anyhow!("No mcp-hub.toml found. Create one or run `mcp-hub init`.")
    })?;
    let config_path = std::fs::canonicalize(&config).map_err(|e| {
        anyhow::anyhow!(
            "Failed to load config {}: cannot resolve path: {e}",
            config.display()
        )
    })?;
    let key = project_key(&config_path)?;
    let sock = socket_path(&key)?;
    Ok(StartPaths {
        config_path,
        pid: pid_path(&key)?,
        meta: meta_path_for(&sock),
        log: log_path(log_file, &key)?,
        sock,
    })
}

/// Refuse to start next to a live daemon from before per-project daemons.
///
/// Such a daemon owns `<state dir>/mcp-hub.sock` and may serve this very
/// config; there is no way to tell, so the user must stop it first. Skipped
/// when `MCP_HUB_SOCKET` is overridden (the legacy socket is then irrelevant).
pub fn check_legacy_daemon() -> anyhow::Result<()> {
    if std::env::var_os("MCP_HUB_SOCKET").is_some() {
        return Ok(());
    }
    let legacy = legacy_socket_path()?;
    if socket_is_live(&legacy) {
        anyhow::bail!(
            "A pre-upgrade mcp-hub daemon is already running (socket: {}). \
             Run `mcp-hub stop` to stop it first, then start again.",
            legacy.display()
        );
    }
    Ok(())
}

/// The daemon a client command talks to.
#[derive(Debug, Clone)]
pub struct Target {
    pub sock: PathBuf,
    /// What the daemon recorded about itself (absent for pre-upgrade daemons).
    pub meta: Option<DaemonMeta>,
}

/// Find the daemon a client command (`stop`, `status`, ...) should talk to.
///
/// 1. `MCP_HUB_SOCKET` set: that socket.
/// 2. A config resolves (`-c`, `./mcp-hub.toml`, global): the daemon of that
///    config; if it has no socket but a pre-upgrade daemon is live, that one.
/// 3. No config: the only live daemon in the state dir; several is an error
///    listing their configs, none the usual "no daemon" error.
pub fn resolve_target(config_flag: Option<&std::path::Path>) -> anyhow::Result<Target> {
    if let Some(sock) = override_socket_path()? {
        let meta = read_meta(&meta_path_for(&sock));
        return Ok(Target { sock, meta });
    }

    if let Some(config) = crate::config::locate_config(config_flag)? {
        let key = project_key(&config)?;
        let sock = socket_path(&key)?;
        if !sock.exists() {
            if let Some(target) = legacy_target()? {
                return Ok(target);
            }
        }
        let meta = read_meta(&meta_path_for(&sock));
        return Ok(Target { sock, meta });
    }

    let mut live = list_live_daemons()?;
    match live.len() {
        0 => {
            if let Some(target) = legacy_target()? {
                return Ok(target);
            }
            anyhow::bail!(
                "No mcp-hub daemon is running (and no mcp-hub.toml was found here).\n\
                 Start one with: mcp-hub start --daemon -c <config>"
            )
        }
        1 => {
            let (sock, meta) = live.remove(0);
            eprintln!("Using the daemon for {}", meta.config_path);
            Ok(Target {
                sock,
                meta: Some(meta),
            })
        }
        _ => {
            let list: String = live
                .iter()
                .map(|(_, m)| format!("  {} (pid {})\n", m.config_path, m.pid))
                .collect();
            anyhow::bail!(
                "Several mcp-hub daemons are running:\n{list}Pass -c <config> to choose one."
            )
        }
    }
}

/// The pre-upgrade daemon as a target, if one is live.
fn legacy_target() -> anyhow::Result<Option<Target>> {
    let legacy = legacy_socket_path()?;
    if !socket_is_live(&legacy) {
        return Ok(None);
    }
    eprintln!(
        "note: talking to a pre-upgrade mcp-hub daemon ({}); \
         run `mcp-hub stop` and start again to move it to a per-project daemon",
        legacy.display()
    );
    Ok(Some(Target {
        sock: legacy,
        meta: None,
    }))
}

/// Resolve the daemon log file path.
///
/// `explicit` (from `--log-file` / `MCP_HUB_LOG_FILE`) wins. Otherwise, with a
/// `MCP_HUB_SOCKET` override the log lives next to that socket as `daemon.log`
/// (keeping test daemons isolated); by default it is `daemon-<key>.log` in the
/// state directory.
pub fn log_path(explicit: Option<&std::path::Path>, key: &str) -> anyhow::Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    if let Some(sock) = override_socket_path()? {
        let dir = sock
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Socket path {} has no parent", sock.display()))?;
        return Ok(dir.join("daemon.log"));
    }
    Ok(state_dir()?.join(format!("daemon-{key}.log")))
}

/// The meta file that sits next to a daemon's socket (`mcp-hub-<key>.json`).
pub fn meta_path_for(sock: &std::path::Path) -> PathBuf {
    sock.with_extension("json")
}

/// What a running daemon records about itself in its meta file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DaemonMeta {
    pub config_path: String,
    pub pid: u32,
    pub web_port: u16,
    pub started_at: String,
}

/// Write `meta` to `path` with mode `0600`.
pub fn write_meta(path: &std::path::Path, meta: &DaemonMeta) -> anyhow::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

    let json = serde_json::to_string_pretty(meta)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| anyhow::anyhow!("Failed to write {}: {e}", path.display()))?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(json.as_bytes())?;
    Ok(())
}

/// Read a meta file; `None` if it is missing or malformed.
pub fn read_meta(path: &std::path::Path) -> Option<DaemonMeta> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Live daemons in the state directory, found through their meta files and
/// sorted by config path. A meta file whose socket no longer accepts
/// connections belongs to a dead daemon and is skipped.
pub fn list_live_daemons() -> anyhow::Result<Vec<(PathBuf, DaemonMeta)>> {
    let dir = state_dir()?;
    let mut found = Vec::new();
    let entries = std::fs::read_dir(&dir)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_meta = path.extension().is_some_and(|e| e == "json")
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("mcp-hub-"));
        if !is_meta {
            continue;
        }
        let sock = path.with_extension("sock");
        if let Some(meta) = read_meta(&path).filter(|_| socket_is_live(&sock)) {
            found.push((sock, meta));
        }
    }
    found.sort_by(|a, b| a.1.config_path.cmp(&b.1.config_path));
    Ok(found)
}

/// Open (creating if needed) the daemon log file for appending, mode 0600.
///
/// The mode is enforced on pre-existing files too, since the log can contain
/// MCP server output.
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

/// Resolve the daemon PID file path: `<state dir>/mcp-hub-<key>.pid`.
///
/// `MCP_HUB_PID` overrides it (used by integration tests).
pub fn pid_path(key: &str) -> anyhow::Result<PathBuf> {
    if let Some(override_path) = std::env::var_os("MCP_HUB_PID") {
        return Ok(PathBuf::from(override_path));
    }
    Ok(state_dir()?.join(format!("mcp-hub-{key}.pid")))
}

// ─────────────────────────────────────────────────────────────────────────────
// PID file management
// ─────────────────────────────────────────────────────────────────────────────

/// An exclusive advisory lock on the PID file, held for the daemon's lifetime.
///
/// The lock lives on the open file description, so it is released when this
/// guard is dropped or the process dies (even via `SIGKILL`). The descriptor is
/// `O_CLOEXEC`, so managed servers never inherit it.
#[derive(Debug)]
pub struct PidFileLock {
    _file: nix::fcntl::Flock<std::fs::File>,
}

/// Take an exclusive, non-blocking `flock` on `file`.
///
/// `Ok(None)` means another process holds the lock.
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
        // A path that cannot be stat'ed is no longer linked to the locked file.
        let linked = std::fs::metadata(path)
            .map(|m| m.dev() == locked.dev() && m.ino() == locked.ino())
            .unwrap_or(false);
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
/// If the socket or PID file is owned by a different user, the check fails
/// without unlinking anything.
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
pub fn check_existing_daemon(
    sock_path: &std::path::Path,
    pid_path: &std::path::Path,
) -> anyhow::Result<()> {
    use std::os::unix::net::UnixStream as StdUnixStream;

    ensure_owned_by_current_user(sock_path)?;
    ensure_owned_by_current_user(pid_path)?;

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
        // `io::ErrorKind` is non-exhaustive; any open failure (missing file
        // included) means there is no PID file to lock.
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
fn describe_recorded_pid(pid_path: &std::path::Path) -> String {
    match read_recorded_pid(pid_path) {
        Some(pid) => format!("PID {pid}"),
        None => format!("PID file {} is locked", pid_path.display()),
    }
}

/// Fail if `path` exists and is owned by a uid other than the current one.
///
/// A socket or PID file planted by another user must never be trusted or
/// unlinked on their behalf. A missing file is fine.
fn ensure_owned_by_current_user(path: &std::path::Path) -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt;

    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => anyhow::bail!("Cannot stat {}: {e}", path.display()),
    };
    let current = nix::unistd::getuid().as_raw();
    if meta.uid() != current {
        anyhow::bail!(
            "{} is owned by another user (uid {}, expected {current}); refusing to use or remove it",
            path.display(),
            meta.uid()
        );
    }
    Ok(())
}

/// Whether process `pid` exists (`kill(pid, 0)`); `EPERM` still means it exists.
///
/// On Linux a zombie (killed but not yet reaped, e.g. by a slow init) counts as
/// dead: it holds no sockets or locks any more.
fn pid_is_alive(pid: i32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    match kill(Pid::from_raw(pid), None) {
        Ok(()) | Err(Errno::EPERM) => !is_zombie(pid),
        Err(Errno::ESRCH) => false,
        // `Errno` is non-exhaustive; anything else cannot be a live process we
        // could be racing with.
        Err(_) => false,
    }
}

/// Whether process `pid` is an `mcp-hub` binary, judged by `/proc/<pid>/comm`.
///
/// `None` when that cannot be determined (not Linux, or the entry is
/// unreadable); callers should then err on the side of "running".
fn looks_like_mcp_hub(pid: i32) -> Option<bool> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(comm.trim_end().starts_with("mcp-hub"))
}

/// True if `/proc/<pid>/stat` reports state `Z` (always false off Linux).
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
pub struct ReadyNotifier {
    pipe: std::fs::File,
}

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

/// Outcome of waiting on the readiness pipe, as seen by the foreground parent.
#[derive(Debug, PartialEq, Eq)]
enum ReadyOutcome {
    Ready { pid: String },
    Failed(String),
    ExitedEarly,
    TimedOut,
}

/// Interpret the bytes a daemon wrote to the readiness pipe.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_known_vectors() {
        assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a_64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn project_key_is_stable_twelve_hex_and_distinguishes_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a.toml");
        let b = dir.path().join("b.toml");
        std::fs::write(&a, "").expect("write a");
        std::fs::write(&b, "").expect("write b");

        let ka = project_key(&a).expect("key a");
        assert_eq!(ka.len(), 12);
        assert!(ka.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(ka, project_key(&a).expect("key a again"));
        assert_ne!(ka, project_key(&b).expect("key b"));
    }

    #[test]
    fn project_key_matches_for_symlinks_and_non_canonical_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let real = dir.path().join("real.toml");
        std::fs::write(&real, "").expect("write");
        let link = dir.path().join("link.toml");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");
        std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
        let dotted = dir.path().join("sub").join("..").join("real.toml");

        let key = project_key(&real).expect("key");
        assert_eq!(key, project_key(&link).expect("link key"));
        assert_eq!(key, project_key(&dotted).expect("dotted key"));
    }

    #[test]
    fn project_key_of_missing_file_is_an_error() {
        assert!(project_key(std::path::Path::new("/nonexistent/mcp-hub.toml")).is_err());
    }

    #[test]
    fn macos_style_default_socket_path_fits_sun_path() {
        let sock = format!(
            "/Users/{}/Library/Application Support/mcp-hub/mcp-hub-0123456789ab.sock",
            "a-reasonably-long-user-name"
        );
        assert!(sock.len() <= MAX_SOCKET_PATH_BYTES, "{}", sock.len());
        assert!(check_socket_len(PathBuf::from(&sock)).is_ok());
        let too_long = PathBuf::from(format!("/{}/mcp-hub-0123456789ab.sock", "x".repeat(100)));
        assert!(check_socket_len(too_long).is_err());
    }

    #[test]
    fn meta_round_trips_with_mode_0600() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = meta_path_for(&dir.path().join("mcp-hub-0123456789ab.sock"));
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("mcp-hub-0123456789ab.json")
        );
        let meta = DaemonMeta {
            config_path: "/p/mcp-hub.toml".into(),
            pid: 42,
            web_port: 3456,
            started_at: "2026-01-01T00:00:00+00:00".into(),
        };
        write_meta(&path, &meta).expect("write");
        assert_eq!(read_meta(&path), Some(meta));
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
