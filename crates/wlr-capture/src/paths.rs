//! Filesystem locations shared by the tools (control sockets, single-instance locks).

use std::path::PathBuf;

/// The per-user runtime directory for control sockets and lock files.
///
/// Prefers `$XDG_RUNTIME_DIR` — the per-user, mode-0700 tmpfs mandated for any Wayland
/// session, so anything placed there is already private. When it is unset (rare outside a
/// login session) fall back to a private `wlr-utils-<uid>` sub-directory of the system
/// temp dir, created mode 0700, rather than dropping a predictably-named socket straight
/// into world-readable `/tmp`. If a safe private directory can't be established, fall back
/// to the bare temp dir. The path is a deterministic function of the environment, so a
/// client and daemon (or two would-be lock holders) always agree on it.
pub fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir);
    }
    private_temp_subdir().unwrap_or_else(std::env::temp_dir)
}

/// A `wlr-utils-<uid>` directory under the system temp dir, guaranteed to be a directory
/// we own with no group/other access. `None` if it can't be created, or an existing path
/// isn't safe to reuse (wrong type, wrong owner, or group/other-accessible — e.g. squatted
/// by another local user).
fn private_temp_subdir() -> Option<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    let uid = rustix::process::getuid().as_raw();
    let dir = std::env::temp_dir().join(format!("wlr-utils-{uid}"));
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        // Freshly created 0700 and owned by us.
        Ok(()) => Some(dir),
        // Already there: reuse only if it is a directory we own, private to us.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let md = std::fs::metadata(&dir).ok()?;
            let private = md.permissions().mode() & 0o077 == 0;
            (md.is_dir() && md.uid() == uid && private).then_some(dir)
        }
        Err(_) => None,
    }
}
