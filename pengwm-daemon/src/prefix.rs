use std::time::{Duration, Instant};

use pengwm_core::command::Command;

use crate::config::keybinds::{
    key_name_to_keycode, parse_modifiers, split_keybind_str, ModifierFlags, MODIFIER_ALT,
    MODIFIER_NONE,
};

/// The chord (keycode + modifiers) that arms the prefix, plus how long the
/// armed window lasts. Defaults to `alt-space` / 1s (Q3).
#[derive(Debug, Clone)]
pub struct PrefixConfig {
    pub keycode: u16,
    pub modifiers: ModifierFlags,
    pub timeout: Duration,
}

impl Default for PrefixConfig {
    fn default() -> Self {
        Self {
            keycode: 0x31, // space
            modifiers: MODIFIER_ALT,
            timeout: Duration::from_secs(1),
        }
    }
}

impl PrefixConfig {
    /// Parse `"alt-space"` plus a timeout, falling back to defaults on bad
    /// input so a typo in the config can't brick the prefix.
    pub fn parse_or_default(s: &str, timeout_ms: u64) -> Self {
        let timeout = Duration::from_millis(timeout_ms.max(1));
        let (mod_str, key_name) = split_keybind_str(s);
        match key_name_to_keycode(key_name) {
            Some(keycode) => Self {
                keycode,
                modifiers: parse_modifiers(mod_str),
                timeout,
            },
            None => Self {
                timeout,
                ..Self::default()
            },
        }
    }
}

/// What `PrefixKey::on_keydown` decided for one key event.
#[derive(Debug, PartialEq)]
pub enum PrefixOutcome {
    /// The prefix chord itself: arm (or re-arm) and swallow.
    Armed,
    /// A follow-up while armed: fire this command and swallow on queue.
    Fire(Command),
    /// Not ours: fall through to the normal direct-bind lookup.
    Passthrough,
}

/// The tmux-style prefix state machine. Pure (no FFI, no channels) so it is
/// testable through its own interface: feed key events in, get outcomes out.
/// The event tap owns the `Arc<Mutex<PrefixKey>>` and the actual swallowing.
pub struct PrefixKey {
    config: PrefixConfig,
    armed_until: Option<Instant>,
}

impl PrefixKey {
    pub fn new(config: PrefixConfig) -> Self {
        Self {
            config,
            armed_until: None,
        }
    }

    pub fn set_config(&mut self, config: PrefixConfig) {
        self.config = config;
        self.armed_until = None;
    }

    pub fn is_armed(&self, now: Instant) -> bool {
        matches!(self.armed_until, Some(until) if now <= until)
    }

    /// Classify one keydown. `lookup` is the normal keybind table; the prefix
    /// never keeps its own table (full mirror — one table, two triggers).
    pub fn on_keydown(
        &mut self,
        keycode: u16,
        modifiers: ModifierFlags,
        now: Instant,
        lookup: &dyn Fn(u16, ModifierFlags) -> Option<Command>,
    ) -> PrefixOutcome {
        if keycode == self.config.keycode && modifiers == self.config.modifiers {
            self.armed_until = Some(now + self.config.timeout);
            return PrefixOutcome::Armed;
        }
        if !self.is_armed(now) {
            self.armed_until = None;
            return PrefixOutcome::Passthrough;
        }
        // Exact chord first (works for any custom binds); then the short
        // follow-up: a bare key inherits the prefix modifiers, so with the
        // default `alt-space` prefix, `prefix, h` behaves like `alt-h` and
        // `prefix, 1` like `alt-1` (tmux-style window switching).
        let cmd = lookup(keycode, modifiers).or_else(|| {
            if modifiers == MODIFIER_NONE {
                lookup(keycode, self.config.modifiers)
            } else {
                None
            }
        });
        match cmd {
            Some(cmd) => {
                if cmd.is_repeatable() {
                    self.armed_until = Some(now + self.config.timeout);
                } else {
                    self.armed_until = None;
                }
                PrefixOutcome::Fire(cmd)
            }
            None => {
                self.armed_until = None;
                PrefixOutcome::Passthrough
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::keybinds::KeybindConfig;
    use pengwm_core::tree::Direction;

    fn lookup_for(config: &KeybindConfig) -> impl Fn(u16, ModifierFlags) -> Option<Command> + '_ {
        move |kc, md| crate::config::keybinds::find_keybind(kc, md, config)
    }

    fn now() -> Instant {
        Instant::now()
    }

    #[test]
    fn prefix_chord_arms() {
        let mut p = PrefixKey::new(PrefixConfig::default());
        let out = p.on_keydown(0x31, MODIFIER_ALT, now(), &|_, _| None);
        assert_eq!(out, PrefixOutcome::Armed);
        assert!(p.is_armed(now()));
    }

    #[test]
    fn unrelated_key_passes_through_unarmed() {
        let mut p = PrefixKey::new(PrefixConfig::default());
        let out = p.on_keydown(0x04, MODIFIER_ALT, now(), &|_, _| None);
        assert_eq!(out, PrefixOutcome::Passthrough);
    }

    #[test]
    fn bare_followup_inherits_prefix_modifiers() {
        let mut p = PrefixKey::new(PrefixConfig::default());
        let binds = KeybindConfig::default();
        let lookup = lookup_for(&binds);
        assert_eq!(
            p.on_keydown(0x31, MODIFIER_ALT, now(), &lookup),
            PrefixOutcome::Armed
        );
        // Bare `h` behaves like `alt-h` (focus left).
        assert_eq!(
            p.on_keydown(0x04, MODIFIER_NONE, now(), &lookup),
            PrefixOutcome::Fire(Command::Focus {
                direction: Direction::Left
            })
        );
    }

    #[test]
    fn repeatable_followup_extends_arm() {
        let mut p = PrefixKey::new(PrefixConfig::default());
        let binds = KeybindConfig::default();
        let lookup = lookup_for(&binds);
        p.on_keydown(0x31, MODIFIER_ALT, now(), &lookup);
        p.on_keydown(0x04, MODIFIER_NONE, now(), &lookup);
        assert!(p.is_armed(now()), "focus is repeatable: arm extends");
    }

    #[test]
    fn oneshot_followup_disarms() {
        let mut p = PrefixKey::new(PrefixConfig::default());
        let binds = KeybindConfig::default();
        let lookup = lookup_for(&binds);
        p.on_keydown(0x31, MODIFIER_ALT, now(), &lookup);
        // `alt-b` (toggle bar) is a one-shot: fires once, then disarms.
        assert_eq!(
            p.on_keydown(0x0B, MODIFIER_ALT, now(), &lookup),
            PrefixOutcome::Fire(Command::ToggleBar)
        );
        assert!(!p.is_armed(now()));
    }

    #[test]
    fn unknown_followup_disarms_and_passes() {
        let mut p = PrefixKey::new(PrefixConfig::default());
        let binds = KeybindConfig::default();
        let lookup = lookup_for(&binds);
        p.on_keydown(0x31, MODIFIER_ALT, now(), &lookup);
        assert_eq!(
            p.on_keydown(0xFF, MODIFIER_NONE, now(), &lookup),
            PrefixOutcome::Passthrough
        );
        assert!(!p.is_armed(now()));
    }

    #[test]
    fn expired_arm_passes_through() {
        let mut p = PrefixKey::new(PrefixConfig {
            timeout: Duration::ZERO,
            ..PrefixConfig::default()
        });
        let binds = KeybindConfig::default();
        let lookup = lookup_for(&binds);
        // Injected times, not now(): with a zero timeout, expiry is
        // `now <= until` — two successive real `Instant::now()` calls can
        // land on the same tick and the follow-up would fire instead of
        // passing through. Offsets make the "arm then expire" sequence
        // deterministic.
        let t0 = Instant::now();
        p.on_keydown(0x31, MODIFIER_ALT, t0, &lookup);
        assert_eq!(
            p.on_keydown(0x04, MODIFIER_NONE, t0 + Duration::from_nanos(1), &lookup),
            PrefixOutcome::Passthrough
        );
    }

    #[test]
    fn prefix_chord_rearms_while_armed() {
        let mut p = PrefixKey::new(PrefixConfig::default());
        let binds = KeybindConfig::default();
        let lookup = lookup_for(&binds);
        p.on_keydown(0x31, MODIFIER_ALT, now(), &lookup);
        assert_eq!(
            p.on_keydown(0x31, MODIFIER_ALT, now(), &lookup),
            PrefixOutcome::Armed
        );
    }

    #[test]
    fn parse_or_default_bad_key_falls_back() {
        let cfg = PrefixConfig::parse_or_default("alt-frobnicate", 500);
        assert_eq!(cfg.keycode, 0x31);
        assert_eq!(cfg.modifiers, MODIFIER_ALT);
        assert_eq!(cfg.timeout, Duration::from_millis(500));
    }

    #[test]
    fn parse_or_default_custom_chord() {
        let cfg = PrefixConfig::parse_or_default("ctrl-b", 2000);
        assert_eq!(cfg.keycode, 0x0B);
        assert_eq!(cfg.modifiers, crate::config::keybinds::MODIFIER_CTRL);
        assert_eq!(cfg.timeout, Duration::from_secs(2));
    }
}
