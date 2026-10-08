//! Background release check. Polls the GitHub releases API on a thread so the
//! network never touches the UI thread; the menu reads the shared result on
//! every open.
//!
//! Version source: this crate's own `CARGO_PKG_VERSION`. The workspace does
//! NOT pin versions (`workspace.package` is unused), so pengwm-core,
//! pengwm-daemon, and pengwm-menubar each carry their own `version` field that
//! is kept in sync manually — release tags and this comparison depend on that
//! staying true.

use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const RELEASES_URL: &str = "https://api.github.com/repos/Penqguin/PengWM/releases/latest";
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Spawn the watcher thread. Runs forever; on every tick it overwrites
/// `latest` with the newest release tag whenever that release is strictly
/// newer than this build, and leaves it untouched on network/parse trouble.
pub fn watch(latest: Arc<Mutex<Option<String>>>) {
    loop {
        check(&latest);
        std::thread::sleep(CHECK_INTERVAL);
    }
}

fn check(latest: &Mutex<Option<String>>) {
    let body = fetch();
    let Some(tag) = body.as_deref().and_then(parse_tag) else {
        return;
    };
    if is_newer(&tag, env!("CARGO_PKG_VERSION")) {
        log::info!("update available: {tag} (running {})", env!("CARGO_PKG_VERSION"));
        *latest.lock().unwrap() = Some(tag);
    } else {
        log::debug!("no newer release than {}", env!("CARGO_PKG_VERSION"));
    }
}

/// Fetch the latest-release JSON via the system curl. Errors are logged, never
/// propagated: this runs unattended and the menu simply shows nothing when the
/// check fails.
fn fetch() -> Option<String> {
    let output = Command::new("/usr/bin/curl")
        .args(["-fsSL", "--max-time", "30", RELEASES_URL])
        .output()
        .map_err(|e| log::warn!("update check: curl failed to start: {e}"))
        .ok()?;
    if !output.status.success() {
        log::debug!(
            "update check: curl exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return None;
    }
    match String::from_utf8(output.stdout) {
        Ok(s) => Some(s),
        Err(e) => {
            log::warn!("update check: non-UTF-8 response: {e}");
            None
        }
    }
}

/// Pull `"tag_name":"..."` out of the releases JSON with plain string ops —
/// the payload is a single flat object and a serde dependency here would buy
/// nothing.
fn parse_tag(body: &str) -> Option<String> {
    const KEY: &str = "\"tag_name\"";
    let rest = &body[body.find(KEY)? + KEY.len()..];
    let rest = &rest[rest.find(':')? + 1..];
    let rest = &rest[rest.find('"')? + 1..];
    let tag = &rest[..rest.find('"')?];
    (!tag.is_empty()).then(|| tag.to_string())
}

/// Strip a `v` prefix and any prerelease/metadata suffix (`-rc.1`, `+build`)
/// and compare the remaining dotted numbers component-wise. Unparseable tags
/// yield `None`, i.e. "never report an update".
fn numeric_core(tag: &str) -> Option<Vec<u64>> {
    let tag = tag.trim().strip_prefix('v').unwrap_or(tag.trim());
    let core = tag.split(['-', '+']).next().unwrap_or(tag);
    let parts: Vec<u64> = core.split('.').map(str::parse).collect::<Result<_, _>>().ok()?;
    (!parts.is_empty()).then_some(parts)
}

fn is_newer(latest: &str, current: &str) -> bool {
    match (numeric_core(latest), numeric_core(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_thumbtack_tag() {
        let body = r#"{"url":"https://…","tag_name":"v0.6.0","name":"x"}"#;
        assert_eq!(parse_tag(body), Some("v0.6.0".to_string()));
    }

    #[test]
    fn parse_tag_missing_or_empty() {
        assert_eq!(parse_tag(r#"{"message":"Not Found"}"#), None);
        assert_eq!(parse_tag(r#"{"tag_name":""}"#), None);
    }

    #[test]
    fn dotted_compare() {
        assert!(is_newer("v0.6.0", "0.5.1"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("v0.5.1", "0.5.1"));
        assert!(!is_newer("v0.5.0", "0.5.1"));
        assert!(!is_newer("v0.4.0-rc.1", "0.5.1"));
    }

    #[test]
    fn prerelease_suffixes_are_ignored() {
        assert!(is_newer("v0.6.0-rc.2", "0.5.1"));
        assert!(!is_newer("v0.5.1+build.9", "0.5.1"));
    }

    #[test]
    fn garbage_tags_report_nothing() {
        assert!(!is_newer("banana", "0.5.1"));
    }
}
