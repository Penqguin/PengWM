//! The release check, asked once and answered in one shape.
//!
//! One module owns the whole "is there a newer PengWM release?" question —
//! curl, tag extraction, and version comparison — so the menubar's update
//! watcher and the CLI's `pengwm update` ask the same seam instead of each
//! carrying a private copy (they had drifted: one required a strictly newer
//! numeric core, the other also counted equal-core/different-tag re-releases).
//!
//! Comparison semantics, unified: strip a `v` prefix and any prerelease /
//! build suffix (`-rc.1`, `+build`), then compare the remaining dotted
//! numbers component-wise. Only a strictly newer numeric core counts — an
//! equal-core tag (a re-published same-version release) is *not* an update,
//! so nobody is prompted for a rebuild they cannot distinguish from what
//! they already run.
//!
//! Network fetches shell out to the system curl so core stays
//! dependency-free; failures are values (`Failed`), never panics — the
//! callers run unattended and simply show nothing / stay put.

use std::process::Command;

const RELEASES_URL: &str = "https://api.github.com/repos/Penqguin/PengWM/releases/latest";

/// The answer to "is there a newer release than `current`?"
#[derive(Debug, PartialEq)]
pub enum UpdateCheck {
    /// A strictly newer release exists; `latest` names its tag.
    UpdateAvailable { latest: String },
    /// The newest published release is not newer than this build.
    UpToDate { latest: String },
    /// The check itself failed (network, HTTP, parse). `reason` explains
    /// why; callers treat this as "no update", possibly loudly.
    Failed { reason: String },
}

impl UpdateCheck {
    /// `true` only when a strictly newer release was found. The convenience
    /// the menubar poller wants; the CLI wants the full match.
    pub fn is_update_available(&self) -> bool {
        matches!(self, UpdateCheck::UpdateAvailable { .. })
    }
}

/// Ask GitHub for the latest release and compare it against `current`.
pub fn check(current: &str) -> UpdateCheck {
    let body = match fetch() {
        Ok(body) => body,
        Err(reason) => return UpdateCheck::Failed { reason },
    };
    match parse_tag(&body) {
        Some(tag) => {
            if is_newer(&tag, current) {
                UpdateCheck::UpdateAvailable { latest: tag }
            } else {
                UpdateCheck::UpToDate { latest: tag }
            }
        }
        None => UpdateCheck::Failed {
            reason: format!("could not parse a release tag from {RELEASES_URL}"),
        },
    }
}

/// Fetch the latest-release JSON via the system curl.
fn fetch() -> Result<String, String> {
    let output = Command::new("/usr/bin/curl")
        .args(["-fsSL", "--max-time", "30", "-H", "Accept: application/vnd.github+json"])
        .arg(RELEASES_URL)
        .output()
        .map_err(|e| format!("curl failed to start: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "curl exited {} ({}{})",
            output.status.code().unwrap_or(-1),
            "",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("non-UTF-8 response: {e}"))
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
    let parts: Vec<u64> = core
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
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

    #[test]
    fn equal_core_different_tag_is_not_an_update() {
        // The unified semantic: a re-published same-version release never
        // counts as "update available" — the user cannot distinguish it from
        // what they already run, so do not ask them to pay a re-grant.
        assert!(!is_newer("v0.6.1", "0.6.1"));
        assert!(!is_newer("v0.6.1-rc.1", "0.6.1"));
    }

    #[test]
    fn availability_convenience() {
        assert!(UpdateCheck::UpdateAvailable { latest: "v1.0.0".into() }.is_update_available());
        assert!(!UpdateCheck::UpToDate { latest: "v0.6.1".into() }.is_update_available());
        assert!(!UpdateCheck::Failed { reason: "x".into() }.is_update_available());
    }
}
