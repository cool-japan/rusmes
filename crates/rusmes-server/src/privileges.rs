//! Privilege-drop helpers: chroot + setuid/setgid.
//!
//! Callers must invoke [`PrivilegeDrop::apply`] **after** all sockets are
//! bound and TLS files are loaded into memory, and **before** any
//! `tokio::spawn` calls.  Violating this ordering may leave ports unbound
//! or TLS keys inaccessible post-drop.
//!
//! ## Platform support
//!
//! Only Linux performs the actual privilege drop.  All other targets (macOS,
//! etc.) emit a `tracing::warn!` if drop was requested and return `Ok(())`.
//!
//! ## DNS after chroot
//!
//! Once the server enters a chroot, `/etc/resolv.conf` is no longer
//! accessible.  Callers must pre-resolve any needed addresses before calling
//! `apply`, or ensure `/etc/resolv.conf` (and related glibc NSS files) are
//! staged under `runtime_dir` before startup.
//!
//! ## Bind-ordering limitation (current architecture)
//!
//! The current `rusmes-server/src/main.rs` architecture binds all listener
//! sockets **inside** `tokio::spawn` closures (i.e., post-drop).  This means
//! that when `run_as_user` / `run_as_group` / `chroot` are set, any privileged
//! ports (<1024) will **fail to bind** after the privilege drop has been
//! applied.  Operators using these fields should therefore either:
//!
//! - Use non-privileged ports (≥1024) and rely on a port-forwarding rule
//!   (e.g. `nftables`, `iptables REDIRECT`, or `CAP_NET_BIND_SERVICE`), or
//! - Wait for the planned listener-pre-bind refactor that hoists all
//!   `TcpListener::bind` calls above the first `tokio::spawn`.
//!
//! This limitation is tracked in `crates/rusmes-server/TODO.md`.

use std::path::PathBuf;

/// Requested privilege drop.
///
/// All fields are optional — `None` / `false` means "no change" (back-compat).
#[derive(Debug, Default)]
pub struct PrivilegeDrop {
    /// If `Some`, call `chroot(dir)` followed by `chdir("/")`.
    /// The directory becomes the filesystem root for all subsequent I/O.
    pub chroot_dir: Option<PathBuf>,
    /// Target UID.  `None` = don't call `setuid`.
    pub uid: Option<nix::unistd::Uid>,
    /// Target GID.  `None` = don't call `setgid`.
    pub gid: Option<nix::unistd::Gid>,
}

impl PrivilegeDrop {
    /// Apply chroot + setgid + setuid in the correct order.
    ///
    /// Ordering: chroot first (requires root), then setgroups/setgid, then
    /// setuid last (once root is dropped we cannot chroot or change group).
    ///
    /// # Bind-ordering caveat
    ///
    /// Because the current server architecture binds sockets inside spawned
    /// tasks, calling `apply()` before the first `tokio::spawn` means that
    /// sockets for privileged ports will be bound after root has been dropped.
    /// See the module-level documentation for details and operator guidance.
    #[cfg(target_os = "linux")]
    pub fn apply(&self) -> anyhow::Result<()> {
        use nix::unistd;

        if let Some(dir) = &self.chroot_dir {
            tracing::info!("chroot: entering {:?}", dir);
            unistd::chroot(dir).map_err(|e| anyhow::anyhow!("chroot({:?}) failed: {e}", dir))?;
            unistd::chdir("/")
                .map_err(|e| anyhow::anyhow!("chdir('/') after chroot failed: {e}"))?;
            tracing::info!("chroot: now rooted at {:?}", dir);
        }

        if let Some(gid) = self.gid {
            // Clear supplementary groups, then set primary GID.
            unistd::setgroups(&[gid])
                .map_err(|e| anyhow::anyhow!("setgroups([{gid}]) failed: {e}"))?;
            unistd::setgid(gid).map_err(|e| anyhow::anyhow!("setgid({gid}) failed: {e}"))?;
            tracing::info!("privilege-drop: gid set to {gid}");
        }

        if let Some(uid) = self.uid {
            unistd::setuid(uid).map_err(|e| anyhow::anyhow!("setuid({uid}) failed: {e}"))?;
            tracing::info!("privilege-drop: uid set to {uid}");
        }

        Ok(())
    }

    /// No-op implementation for non-Linux platforms.
    ///
    /// Emits a `tracing::warn!` if any non-default drop was requested,
    /// then returns `Ok(())` so callers behave identically across platforms.
    #[cfg(not(target_os = "linux"))]
    pub fn apply(&self) -> anyhow::Result<()> {
        if self.chroot_dir.is_some() || self.uid.is_some() || self.gid.is_some() {
            tracing::warn!(
                "privilege-drop requested (chroot={:?}, uid={:?}, gid={:?}) \
                 but skipped: only supported on Linux",
                self.chroot_dir,
                self.uid,
                self.gid
            );
        }
        Ok(())
    }
}

/// Resolve a username to a UID using the system user database.
///
/// Returns `None` if `name` is empty (no-op case).
/// Returns `Err` if the name is non-empty but not found in the system database.
///
/// # Examples
///
/// ```rust,no_run
/// use rusmes_server::privileges::resolve_uid;
///
/// // Empty string → no change (None)
/// assert!(resolve_uid("").unwrap().is_none());
///
/// // Non-existent user → Err
/// assert!(resolve_uid("__no_such_user__").is_err());
/// ```
pub fn resolve_uid(name: &str) -> anyhow::Result<Option<nix::unistd::Uid>> {
    if name.is_empty() {
        return Ok(None);
    }
    let user = nix::unistd::User::from_name(name)
        .map_err(|e| anyhow::anyhow!("lookup user {:?} failed: {e}", name))?
        .ok_or_else(|| anyhow::anyhow!("user {:?} not found in system database", name))?;
    Ok(Some(user.uid))
}

/// Resolve a group name to a GID using the system group database.
///
/// Returns `None` if `name` is empty (no-op case).
/// Returns `Err` if the name is non-empty but not found in the system database.
///
/// # Examples
///
/// ```rust,no_run
/// use rusmes_server::privileges::resolve_gid;
///
/// // Empty string → no change (None)
/// assert!(resolve_gid("").unwrap().is_none());
///
/// // Non-existent group → Err
/// assert!(resolve_gid("__no_such_group__").is_err());
/// ```
pub fn resolve_gid(name: &str) -> anyhow::Result<Option<nix::unistd::Gid>> {
    if name.is_empty() {
        return Ok(None);
    }
    let group = nix::unistd::Group::from_name(name)
        .map_err(|e| anyhow::anyhow!("lookup group {:?} failed: {e}", name))?
        .ok_or_else(|| anyhow::anyhow!("group {:?} not found in system database", name))?;
    Ok(Some(group.gid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_privilege_drop_noop_when_no_user_set() {
        let drop = PrivilegeDrop::default();
        drop.apply().expect("no-op drop must not fail");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_privilege_drop_warns_on_macos() {
        // On macOS, apply() must succeed (no-op) even when non-trivial values
        // are requested — it can only emit a warning, not fail.
        let drop = PrivilegeDrop {
            chroot_dir: Some(std::path::PathBuf::from("/tmp")),
            uid: Some(nix::unistd::Uid::from_raw(99)),
            gid: None,
        };
        assert!(drop.apply().is_ok(), "macOS path must be Ok()");
    }

    #[test]
    fn test_resolve_uid_empty_returns_none() {
        assert!(resolve_uid("").unwrap().is_none());
    }

    #[test]
    fn test_resolve_gid_empty_returns_none() {
        assert!(resolve_gid("").unwrap().is_none());
    }

    #[test]
    fn test_resolve_uid_nonexistent_returns_err() {
        assert!(resolve_uid("__nonexistent_rusmes_user__").is_err());
    }

    #[test]
    fn test_resolve_gid_nonexistent_returns_err() {
        assert!(resolve_gid("__nonexistent_rusmes_group__").is_err());
    }
}
