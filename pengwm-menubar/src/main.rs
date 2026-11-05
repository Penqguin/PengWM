mod connection;
#[cfg(target_os = "macos")]
mod hud;
#[cfg(target_os = "macos")]
mod macos;
mod update_check;

use std::sync::{Arc, Mutex};

use pengwm_core::command::{BarState, PillCard};
use std::time::Instant;

#[derive(Clone)]
pub enum HudSlot {
    Hide,
    Show {
        cards: Vec<PillCard>,
        expires: Instant,
    },
}
impl HudSlot {
    /// The latest deadline alone determines visibility; there are no stale timers.
    fn visible_cards(&self, now: Instant) -> Option<&[PillCard]> {
        match self {
            Self::Show { cards, expires } if now < *expires && !cards.is_empty() => Some(cards),
            _ => None,
        }
    }

    fn retain_displays(&mut self, display_ids: &[u32]) {
        if let Self::Show { cards, .. } = self {
            cards.retain(|card| display_ids.contains(&card.display_id));
        }
    }
}
pub type HudSlotRef = Arc<Mutex<HudSlot>>;

/// Whether this process holds Accessibility trust; polled on a background
/// thread while the daemon is unreachable (macos.rs owns the poller).
type TrustedState = Arc<Mutex<bool>>;

/// Release tag of a strictly newer PengWM release, if the update check found
/// one (update_check.rs owns the watcher).
type UpdateState = Arc<Mutex<Option<String>>>;

fn main() {
    env_logger::init();

    // Single-instance guard: the daemon respawns the menubar at startup, and
    // an orphan menubar from a killed daemon can outlive it (it reconnects to
    // the next daemon's socket, so nothing asks it to leave). Without this,
    // one stray process = a second status icon. The lock is held for the
    // process lifetime; losing the try_lock means an instance already runs.
    let lock_path = std::env::temp_dir().join("pengwm-menubar.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .and_then(|f| {
            if f.try_lock().is_ok() {
                Ok(f)
            } else {
                Err(std::io::Error::other("already locked"))
            }
        });
    match file {
        // Keep the file handle alive: forgetting it means the lock lasts
        // until the process exits.
        Ok(file) => std::mem::forget(file),
        Err(e) => {
            log::info!(
                "another pengwm-menubar holds its instance lock ({}); exiting",
                e
            );
            return;
        }
    }

    let state: Arc<Mutex<Option<BarState>>> = Arc::new(Mutex::new(None));
    let trusted: TrustedState = Arc::new(Mutex::new(false));
    let update: UpdateState = Arc::new(Mutex::new(None));
    let hud: HudSlotRef = Arc::new(Mutex::new(HudSlot::Hide));
    {
        let state = Arc::clone(&state);
        let hud = Arc::clone(&hud);
        std::thread::spawn(move || {
            connection::subscribe(state, hud);
        });
    }
    {
        let update = Arc::clone(&update);
        std::thread::spawn(move || update_check::watch(update));
    }

    #[cfg(target_os = "macos")]
    {
        macos::run(state, trusted, update, hud);
    }
    #[cfg(not(target_os = "macos"))]
    {
        log::error!("pengwm-menubar is macOS-only");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod hud_tests {
    use super::*;
    use std::time::Duration;

    fn card(id: u32) -> PillCard {
        PillCard {
            display_id: id,
            number: id as usize,
            name: format!("Workspace {id}"),
            focused: id == 1,
            center: (0.0, 0.0),
        }
    }

    #[test]
    fn newer_flash_survives_previous_deadline_and_expires_at_its_own() {
        let now = Instant::now();
        let old_deadline = now + Duration::from_millis(500);
        let mut slot = HudSlot::Show {
            cards: vec![card(1)],
            expires: old_deadline,
        };
        assert_eq!(slot.visible_cards(now).unwrap()[0].number, 1);
        let new_deadline = old_deadline + Duration::from_millis(300);
        slot = HudSlot::Show {
            cards: vec![card(2)],
            expires: new_deadline,
        };
        assert_eq!(slot.visible_cards(old_deadline).unwrap()[0].number, 2);
        assert!(slot.visible_cards(new_deadline).is_none());
    }

    #[test]
    fn selective_removal_preserves_other_cards_and_original_deadline() {
        let now = Instant::now();
        let deadline = now + Duration::from_millis(500);
        let mut slot = HudSlot::Show {
            cards: vec![card(1), card(2)],
            expires: deadline,
        };
        slot.retain_displays(&[2]);
        assert_eq!(slot.visible_cards(now).unwrap(), &[card(2)]);
        assert!(slot.visible_cards(deadline).is_none());
        slot.retain_displays(&[]);
        assert!(slot.visible_cards(now).is_none());
    }
}
