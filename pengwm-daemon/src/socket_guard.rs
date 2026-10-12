//! Socket identity for the daemon's UDS servers.
//!
//! `bind_first` makes a live listener's socket file unbindable by a rival
//! (bind on it fails `EADDRINUSE`, the probe distinguishes a live daemon
//! from a stale file) — but nothing stops a *rogue `rm`*: the file
//! vanishes, the listener survives on an unlinked vnode, and every CLI
//! connect hits `ECONNREFUSED` while the WM keeps tiling — the
//! ghost-daemon state. [`SocketGuard::tick`] is the per-tick self-heal:
//! compare the inode of the bound path against the identity captured at
//! bind time; if it is missing or foreign, log loudly and rebind.

use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::{Duration, Instant};

/// How long a failed rebind waits before trying again. The check itself
/// runs every accept poll; without a backoff a persistently unbindable
/// path would flood the log.
const RETRY_BACKOFF: Duration = Duration::from_secs(2);

/// Bind `path` under the bind-first contract (see
/// `ipc_server::start_ipc_server_with_path` for the full story): bind
/// first, and only on `Address already in use` probe — a live daemon
/// answers (we refuse to start, never stealing), a stale file left by a
/// crashed daemon is unlinked and rebound. `server` names the caller in
/// log lines ("IPC server", "UI push server").
pub fn bind_first(path: &str, server: &str) -> io::Result<UnixListener> {
    match UnixListener::bind(path) {
        Ok(l) => Ok(l),
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
            if UnixStream::connect(path).is_ok() {
                log::error!("Another daemon owns {path} — not starting the {server}.");
                return Err(e);
            }
            log::info!("Stale socket file at {path} — removing and rebinding");
            let _ = std::fs::remove_file(path);
            UnixListener::bind(path).inspect_err(|e| {
                log::error!("Failed to bind UDS at {path}: {e}");
            })
        }
        Err(e) => {
            log::error!("Failed to bind UDS at {path}: {e}");
            Err(e)
        }
    }
}

/// What the bound path looks like compared to the identity we captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketHealth {
    /// The path still resolves to our own socket file.
    Intact,
    /// The path is gone (rogue `rm`); our listener is on an unlinked vnode.
    Missing,
    /// The path resolves to a different socket (a rival bound it after the
    /// file was removed).
    Foreign,
}

/// Per-tick identity check for one bound socket. Capture right after a
/// successful `bind_first`, then call [`tick`](Self::tick) from the accept
/// loop; on [`SocketHealth::Missing`] or [`SocketHealth::Foreign`] it logs
/// loudly, rebinds, and hands back the fresh listener to swap in.
pub struct SocketGuard {
    path: String,
    /// `(st_dev, st_ino)` of our own socket file, captured at bind time.
    own: Option<(u64, u64)>,
    last_health: SocketHealth,
    next_retry: Option<Instant>,
}

impl SocketGuard {
    /// Capture the identity of the just-bound socket file. A failed capture
    /// is not fatal — the next tick sees `Missing` and rebinds, which
    /// recaptures.
    pub fn capture(path: &str) -> Self {
        let own = read_identity(path);
        if own.is_none() {
            log::warn!("Could not read socket identity at {path} right after bind");
        }
        Self {
            path: path.to_owned(),
            own,
            last_health: SocketHealth::Intact,
            next_retry: None,
        }
    }

    /// One identity check. Returns `Some(listener)` when the socket was
    /// rebound — the caller must swap it in for its accept loop (a fresh
    /// bind is a new socket; connections to the path would otherwise never
    /// reach us).
    pub fn tick(&mut self, now: Instant) -> Option<UnixListener> {
        let health = self.health();
        if health == SocketHealth::Intact {
            self.next_retry = None;
            return None;
        }
        // Log the *transition* only: the check runs every accept poll, and
        // an unhealable path must not become a log firehose.
        if health != self.last_health {
            self.last_health = health;
            match health {
                SocketHealth::Missing => log::error!(
                    "Socket identity lost: {} vanished (rogue rm?) — the listener \
                     is on an unlinked vnode and CLI connects would fail. Rebinding.",
                    self.path
                ),
                SocketHealth::Foreign => {
                    let live = UnixStream::connect(&self.path).is_ok();
                    log::error!(
                        "Socket identity lost: {} now belongs to another socket{} — \
                         reclaiming it for this daemon.",
                        self.path,
                        if live {
                            " (a live peer answers on it)"
                        } else {
                            ""
                        }
                    );
                }
                SocketHealth::Intact => unreachable!("handled above"),
            }
        }
        if self.next_retry.is_some_and(|t| now < t) {
            return None;
        }
        self.next_retry = Some(now + RETRY_BACKOFF);
        match self.rebind() {
            Ok(l) => {
                log::warn!("Rebound {} — socket identity restored", self.path);
                Some(l)
            }
            Err(e) => {
                log::error!(
                    "Rebind of {} failed: {} — retrying every {RETRY_BACKOFF:?}",
                    self.path,
                    e
                );
                None
            }
        }
    }

    fn health(&self) -> SocketHealth {
        match read_identity(&self.path) {
            Some(id) if self.own == Some(id) => SocketHealth::Intact,
            Some(_) => SocketHealth::Foreign,
            None => SocketHealth::Missing,
        }
    }

    /// Remove whatever occupies the path (nothing, a foreign file, a dead
    /// one — a live rival's socket is stolen too, loudly logged above) and
    /// bind a fresh listener, recapturing identity.
    fn rebind(&mut self) -> io::Result<UnixListener> {
        let _ = std::fs::remove_file(&self.path);
        let listener = UnixListener::bind(&self.path)?;
        self.own = read_identity(&self.path);
        self.last_health = SocketHealth::Intact;
        Ok(listener)
    }
}

fn read_identity(path: &str) -> Option<(u64, u64)> {
    // `symlink_metadata` lstats the socket file itself (a Unix socket file
    // is not a symlink, but this never follows a final component).
    std::fs::symlink_metadata(Path::new(path))
        .ok()
        .map(|m| (m.dev(), m.ino()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(tag: &str) -> String {
        let path =
            std::env::temp_dir().join(format!("pengwm-guard-{tag}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path.to_str().unwrap().to_owned()
    }

    #[test]
    fn health_is_intact_right_after_capture() {
        let path = test_path("intact");
        let listener = bind_first(&path, "test").unwrap();
        let mut guard = SocketGuard::capture(&path);
        assert_eq!(guard.health(), SocketHealth::Intact);
        // A tick on an intact socket is a no-op.
        assert!(guard.tick(Instant::now()).is_none());
        drop(listener);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn health_detects_missing_and_heals_with_a_fresh_listener() {
        let path = test_path("missing");
        let listener = bind_first(&path, "test").unwrap();
        let mut guard = SocketGuard::capture(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(guard.health(), SocketHealth::Missing);

        let fresh = guard.tick(Instant::now()).expect("rebind after rm");
        assert_eq!(guard.health(), SocketHealth::Intact);
        // The fresh listener is live on the path; the old one is now redundant.
        assert!(UnixStream::connect(&path).is_ok());
        drop(listener);
        drop(fresh);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn health_detects_foreign_and_reclaims() {
        let path = test_path("foreign");
        let listener = bind_first(&path, "test").unwrap();
        let mut guard = SocketGuard::capture(&path);
        // A rival steals the path the old way: unlink, then bind.
        std::fs::remove_file(&path).unwrap();
        let rival = UnixListener::bind(&path).unwrap();
        assert_eq!(guard.health(), SocketHealth::Foreign);

        let fresh = guard.tick(Instant::now()).expect("reclaim foreign socket");
        assert_eq!(guard.health(), SocketHealth::Intact);
        drop(listener);
        drop(rival);
        drop(fresh);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn bind_first_refuses_to_steal_a_live_socket() {
        let path = test_path("nosteal");
        let winner = bind_first(&path, "test").unwrap();
        // Live peer → hard stop, and the winner's file survives.
        assert!(bind_first(&path, "test").is_err());
        assert!(Path::new(&path).exists());
        drop(winner);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn bind_first_rebinds_a_stale_file() {
        let path = test_path("stale");
        // Dead socket file: bound, then the listener is dropped. The file
        // stays on disk but refuses connections.
        drop(UnixListener::bind(&path).unwrap());
        let listener = bind_first(&path, "test").expect("stale file must be rebound");
        assert!(UnixStream::connect(&path).is_ok());
        drop(listener);
        let _ = std::fs::remove_file(&path);
    }
}
