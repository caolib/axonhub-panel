//! Shared placement and focus-loss behavior for the two transient windows.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, GetCursorPos, GetForegroundWindow, PostMessageW, WM_APP,
};

use crate::config::WindowState;

pub const WM_DISMISS: u32 = WM_APP + 41;

pub fn cursor() -> POINT {
    let mut point = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    point
}

pub fn click_point(hwnd: HWND, lp: LPARAM) -> POINT {
    let mut point = POINT {
        x: (lp.0 & 0xFFFF) as i16 as i32,
        y: ((lp.0 >> 16) & 0xFFFF) as i16 as i32,
    };
    unsafe {
        let _ = ClientToScreen(hwnd, &mut point);
    }
    point
}

/// Anchor the top-left corner at the click, shifting inward near screen edges.
pub fn at_point(point: POINT, width: i32, height: i32) -> WindowState {
    let work = crate::work_area_at(point.x, point.y);
    let width = width.max(1).min((work.2 - work.0).max(1));
    let height = height.max(1).min((work.3 - work.1).max(1));
    WindowState {
        x: point.x.clamp(work.0, work.2 - width),
        y: point.y.clamp(work.1, work.3 - height),
        width,
        height,
    }
}

pub fn dismiss_later(hwnd: HWND) {
    // WM_ACTIVATE is sent in the middle of a focus handoff. Destroying an owned
    // window there can interrupt activation of the next window.
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_DISMISS, WPARAM(0), LPARAM(0));
    }
}

pub fn dismiss_if_inactive(hwnd: HWND) {
    unsafe {
        // A native search field is a child: its top-level foreground window is
        // still the settings window. Temporary Z-order changes can reactivate
        // it before this deferred message arrives and must not close it.
        if GetForegroundWindow() != hwnd {
            let _ = DestroyWindow(hwnd);
        }
    }
}
