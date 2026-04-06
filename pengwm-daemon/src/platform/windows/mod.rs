use async_trait::async_trait;
use crate::platform::WindowManagerBackend;
use crate::core::geometry::Rect;
use crate::core::types::{WindowId, SystemEvent};
use tokio::sync::mpsc::Sender;
use anyhow::Result;
// use std::thread; // thread is used when target_os = "windows"

#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{GetMessageW, DispatchMessageW, TranslateMessage, MSG, SetWindowPos, SWP_NOZORDER, SWP_NOACTIVATE},
    UI::Accessibility::SetWinEventHook,
};

pub struct WindowsBackend;

#[async_trait]
impl WindowManagerBackend for WindowsBackend {
    async fn subscribe(&self, _event_sender: Sender<SystemEvent>) {
        #[cfg(target_os = "windows")]
        thread::spawn(move || {
            // TODO: Initialize SetWinEventHook
            // let _hook = SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, ...);
            
            log::info!("Windows event hook started (stub)");
            let mut msg = MSG::default();
            unsafe {
                while GetMessageW(&mut msg, HWND(0), 0, 0).as_bool() {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        });
    }

    fn set_window_rect(&self, window: WindowId, rect: Rect) -> Result<()> {
        #[cfg(target_os = "windows")]
        unsafe {
            // SetWindowPos(HWND(window.0 as isize), HWND(0), rect.min_x(), rect.min_y(), rect.width(), rect.height(), SWP_NOZORDER | SWP_NOACTIVATE)?;
        }
        log::info!("Windows: Moving window {:?} to {:?}", window, rect);
        Ok(())
    }

    fn is_manageable(&self, _window: WindowId) -> bool {
        // TODO: Check WS_VISIBLE and DWMWA_CLOAKED
        true
    }

    fn get_focused_window(&self) -> Result<WindowId> {
        // let hwnd = unsafe { GetForegroundWindow() };
        Ok(WindowId(0))
    }
}
