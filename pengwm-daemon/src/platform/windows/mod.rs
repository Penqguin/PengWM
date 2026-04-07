//! Windows implementation of the WindowManagerBackend trait.
//! Uses Win32 APIs (User32) to observe and control windows.

use async_trait::async_trait;
use crate::platform::WindowManagerBackend;
use crate::core::geometry::Rect;
use crate::core::types::{WindowId, SystemEvent};
use tokio::sync::mpsc::Sender;
use anyhow::Result;

#[cfg(target_os = "windows")]
use std::thread;

#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::{HWND, RECT},
    UI::WindowsAndMessaging::{
        GetMessageW, DispatchMessageW, TranslateMessage, MSG, SetWindowPos, 
        SWP_NOZORDER, SWP_NOACTIVATE, GetWindowLongW, GWL_STYLE, WS_VISIBLE, 
        GWL_EXSTYLE, WS_EX_TOOLWINDOW, GetWindowTextW, GetWindowRect
    },
};

/// Windows-specific window manager backend.
pub struct WindowsBackend;

#[async_trait]
impl WindowManagerBackend for WindowsBackend {
    /// Subscribes to window system events (stub implementation).
    async fn subscribe(&self, _event_sender: Sender<SystemEvent>) {
        #[cfg(target_os = "windows")]
        thread::spawn(move || {
            log::info!("Windows event hook started (stub)");
            let mut msg = MSG::default();
            unsafe {
                // Main Win32 message loop to receive system-wide window events.
                while GetMessageW(&mut msg, HWND(0), 0, 0).as_bool() {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        });
    }

    /// Moves and resizes a window to the specified rectangle using `SetWindowPos`.
    fn set_window_rect(&self, window: WindowId, rect: Rect) -> Result<()> {
        #[cfg(target_os = "windows")]
        unsafe {
            let hwnd = HWND(window.0 as isize);
            let _ = SetWindowPos(
                hwnd,
                HWND(0),
                rect.min_x(),
                rect.min_y(),
                rect.width() as i32,
                rect.height() as i32,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        log::info!("Windows: Moving window {:?} to {:?}", window, rect);
        Ok(())
    }

    /// Determines if a window should be managed by the window manager.
    /// Filters out tooltips, popups, and other non-standard windows based on styles.
    fn is_manageable(&self, window: WindowId) -> bool {
        #[cfg(target_os = "windows")]
        unsafe {
            let hwnd = HWND(window.0 as isize);
            
            // 1. Must be visible
            let style = GetWindowLongW(hwnd, GWL_STYLE);
            if (style as u32 & WS_VISIBLE.0) == 0 {
                return false;
            }

            // 2. Must not be a tool window
            let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
            if (ex_style as u32 & WS_EX_TOOLWINDOW.0) != 0 {
                return false;
            }

            // 3. Must have a title (simple heuristic)
            let mut title = [0u16; 256];
            let len = GetWindowTextW(hwnd, &mut title);
            if len == 0 {
                return false;
            }

            // 4. Must not be tiny (e.g. 0x0)
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_ok() {
                if rect.right - rect.left < 50 || rect.bottom - rect.top < 50 {
                    return false;
                }
            }

            true
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = window;
            true
        }
    }

    /// Returns the currently focused window (not implemented).
    fn get_focused_window(&self) -> Result<WindowId> {
        Ok(WindowId(0))
    }

    /// No-op on Windows.
    fn release_window(&self, _window: WindowId) {
        // Windows HWNDs don't need explicit releasing like AXUIElementRef.
    }
}
