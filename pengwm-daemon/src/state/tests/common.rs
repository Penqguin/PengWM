use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use super::super::StateManager;
use crate::adapter::DisplayInfo;
use crate::adapter_test::{TestAdapter, TestHandle};
use crate::bar_server::BarSender;
use crate::config::keybinds::KeybindConfig;

pub(super) use crate::adapter_test::Fault;

pub(super) fn make_adapter(display_count: u32) -> TestAdapter {
    let mut adapter = TestAdapter::new();
    if display_count == 1 {
        adapter.displays = vec![DisplayInfo {
            id: 1,
            origin: (0, 0),
            size: (1920, 1080),
        }];
    }
    if display_count == 2 {
        adapter.displays = vec![
            DisplayInfo {
                id: 1,
                origin: (0, 0),
                size: (1920, 1080),
            },
            DisplayInfo {
                id: 2,
                origin: (1920, 0),
                size: (1920, 1080),
            },
        ];
    }
    adapter.frontmost = Some(42);
    adapter.running_apps = vec![42];
    adapter
        .windows
        .borrow_mut()
        .entry(42)
        .or_default()
        .extend(vec![100, 200]);
    adapter.window_pids.borrow_mut().insert(100, 42);
    adapter.window_pids.borrow_mut().insert(200, 42);
    adapter
}

pub(super) fn test_prefix() -> Arc<Mutex<crate::prefix::PrefixKey>> {
    Arc::new(Mutex::new(crate::prefix::PrefixKey::new(
        crate::prefix::PrefixConfig::default(),
    )))
}

pub(super) fn setup(display_count: u32) -> StateManager {
    setup_with_handle(display_count).0
}

pub(super) fn setup_with_handle(display_count: u32) -> (StateManager, TestHandle) {
    let (tx, _) = mpsc::channel(64);
    let keybinds = Arc::new(Mutex::new(KeybindConfig::default()));
    let prefix = test_prefix();
    let adapter = make_adapter(display_count);
    let handle = TestHandle::new(adapter);
    let boxed: Box<dyn crate::adapter::OsAdapter> = Box::new(handle.shared());
    let (bar_tx, _) = mpsc::channel(64);
    (
        StateManager::new(tx, keybinds, prefix, boxed, BarSender::from_channel(bar_tx), vec![]),
        handle,
    )
}
