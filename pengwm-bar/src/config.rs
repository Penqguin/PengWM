pub use pengwm_core::config::{themes_dir, BarConfig, ColorOverrides, DEFAULT_THEME};

/// Standard macOS window corner radius (in points) per major version. `10`
/// from Big Sur through Sequoia (11–15), `26` on Tahoe (26, Liquid Glass),
/// `20` on Golden Gate (27). Falls back to `20` for unknown/future majors.
pub fn corner_radius_for_macos(major: u32) -> f32 {
    match major {
        0..=15 => 10.0,
        26 => 26.0,
        _ => 20.0,
    }
}

/// Resolve the effective corner radius: an explicit `[bar].corner_radius`
/// wins, otherwise the macOS-version default is used.
pub fn resolve_corner_radius(config: &BarConfig) -> f32 {
    if let Some(r) = config.corner_radius {
        return r.max(0.0);
    }
    #[cfg(target_os = "macos")]
    {
        corner_radius_for_macos(crate::macos::macos_major_version())
    }
    #[cfg(not(target_os = "macos"))]
    {
        corner_radius_for_macos(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corner_radius_table_matches_os_versions() {
        assert_eq!(corner_radius_for_macos(11), 10.0);
        assert_eq!(corner_radius_for_macos(14), 10.0);
        assert_eq!(corner_radius_for_macos(15), 10.0);
        assert_eq!(corner_radius_for_macos(26), 26.0);
        assert_eq!(corner_radius_for_macos(27), 20.0);
        assert_eq!(corner_radius_for_macos(99), 20.0);
    }

    #[test]
    fn explicit_corner_radius_overrides_auto() {
        let cfg = BarConfig {
            corner_radius: Some(6.0),
            ..Default::default()
        };
        assert_eq!(resolve_corner_radius(&cfg), 6.0);
        let cfg = BarConfig {
            corner_radius: Some(-2.0),
            ..Default::default()
        };
        assert_eq!(resolve_corner_radius(&cfg), 0.0);
    }

    #[test]
    fn auto_corner_radius_resolves_from_version() {
        let cfg = BarConfig::default();
        let r = resolve_corner_radius(&cfg);
        assert!(r > 0.0);
    }
}
