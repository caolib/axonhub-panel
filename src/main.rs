//! Window lifetime, message loop and input wiring.

#![windows_subsystem = "windows"]

mod app;
mod client;
mod config;
mod format;
mod model;
mod theme;
mod time;
mod token;
mod ui;
mod worker;

use std::cell::RefCell;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDCW, CreateSolidBrush,
    DeleteDC, DeleteObject, EndPaint, FillRect, GetDeviceCaps, GetMonitorInfoW, HGDIOBJ, HORZSIZE,
    InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MONITORINFOEXW, MonitorFromPoint,
    MonitorFromWindow, PAINTSTRUCT, SRCCOPY, SelectObject, VERTSIZE,
};
use windows::Win32::Graphics::GdiPlus::{
    GdiplusShutdown, GdiplusStartup, GdiplusStartupInput, GdiplusStartupOutput,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, SetFocus, VK_A, VK_C, VK_CONTROL, VK_V, VK_X,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_BACK, VK_ESCAPE, VK_RETURN, VK_TAB,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, *,
};
use windows::core::PCWSTR;

use app::{App, View};
use config::{Config, CredentialMode, Credentials, Stored, WindowState};
use theme::{Fonts, Painter};
use time::now_unix;
use ui::layout::Metrics;
use ui::login::{self, Action as LoginAction, Field, Method};
use ui::panel::{self, ListView};
use worker::{Command, Worker};

const CLASS_NAME: &str = "AHPanelWindow";
/// UTF-16 form of `CLASS_NAME`, keeping the literal alive for the window class.
const CLASS_WIDE: [u16; 14] = [
    b'A' as u16,
    b'H' as u16,
    b'P' as u16,
    b'a' as u16,
    b'n' as u16,
    b'e' as u16,
    b'l' as u16,
    b'W' as u16,
    b'i' as u16,
    b'n' as u16,
    b'd' as u16,
    b'o' as u16,
    b'w' as u16,
    0,
];
/// Sent by the tracking requested in `WM_MOUSEMOVE`. Not exported by the
/// windows crate, so it is spelled out here.
const WM_MOUSELEAVE: u32 = 0x02A3;
const TIMER_POLL: usize = 1;
const TIMER_CARET: usize = 2;
/// Repaints so relative timestamps ("3 分钟前") stay honest while idle.
const TIMER_CLOCK: usize = 3;

/// Read Unicode text from the clipboard. Returns `None` when the clipboard is
/// empty, holds no text, or is momentarily locked by another process.
fn clipboard_text() -> Option<String> {
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_err() {
            return None;
        }
        OpenClipboard(None).ok()?;
        // Every exit below must close the clipboard, or the rest of the desktop
        // is left unable to use copy/paste.
        let result = (|| {
            let handle = GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?;
            let hglobal = windows::Win32::Foundation::HGLOBAL(handle.0);
            let ptr = GlobalLock(hglobal) as *const u16;
            if ptr.is_null() {
                return None;
            }
            // Bound the scan by the allocation size: clipboard text is
            // NUL-terminated but the buffer may not be exactly sized.
            let max_units = GlobalSize(hglobal) / std::mem::size_of::<u16>();
            let mut len = 0usize;
            while len < max_units && *ptr.add(len) != 0 {
                len += 1;
            }
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
            let _ = GlobalUnlock(hglobal);
            Some(text)
        })();
        let _ = CloseClipboard();
        result
    }
}

/// Whether Ctrl (or Shift for Ctrl+Shift+V) is held.
fn ctrl_down() -> bool {
    unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 }
}

/// Keep the scroll offset inside the range the current viewport allows.
fn clamp_scroll(hwnd: HWND) {
    let m = metrics_for(hwnd);
    State::with(|s| {
        let max = m.max_scroll(s.app.rows.len());
        s.app.scroll = s.app.scroll.clamp(0.0, max);
    });
}

/// Refit the window when the monitor under it changes *without* a Windows DPI
/// change.
///
/// `WM_DPICHANGED` only fires when the system's per-monitor DPI setting
/// differs between two screens, so dragging the panel between two
/// 100%-scaled monitors of different pixels-per-inch would otherwise leave it
/// the wrong size. This recomputes the physical-density scale, reloads the
/// fonts and resizes the window by the scale ratio, mirroring the
/// `WM_DPICHANGED` path minus its suggested rectangle (the move that triggered
/// this has already placed the window). It is a no-op when the scale is
/// unchanged, so it is cheap to call on every `WM_MOVE`.
fn sync_scale(hwnd: HWND) {
    let scale = window_scale(hwnd);
    let old = State::with(|s| s.scale).unwrap_or(scale);
    if (scale - old).abs() <= 0.01 {
        return;
    }
    State::with(|s| {
        s.fonts = Fonts::load(scale);
        s.scale = scale;
    });

    let mut rc = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rc);
    }
    let ratio = if old > 0.0 { scale / old } else { 1.0 };
    let width = ((rc.right - rc.left) as f32 * ratio).round() as i32;
    let mut height = ((rc.bottom - rc.top) as f32 * ratio).round() as i32;

    // Whole cards only: derive the height from the row count so no partial
    // card is left over, and keep the panel inside this monitor's work area.
    let work = work_area_for(hwnd);
    let fitted = ui::layout::height_for_rows(config_rows(hwnd), scale).min(work.3 - work.1);
    if fitted > 0 {
        height = fitted;
    }
    let placed = config::clamp_to_virtual_screen(
        WindowState {
            x: rc.left,
            y: rc.top,
            width,
            height,
        },
        work,
        scale,
    );
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            placed.x,
            placed.y,
            placed.width,
            placed.height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    clamp_scroll(hwnd);
    invalidate(hwnd);
}

/// The row count configured by the user (derived from the window height).
fn config_rows(hwnd: HWND) -> usize {
    let _ = hwnd;
    State::with(|s| s.app.config.row_limit.max(1) as usize).unwrap_or(12)
}

/// Rows that fit in the window at its current size.
fn current_visible_rows(hwnd: HWND) -> usize {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    let height = (rc.bottom - rc.top) as f32;
    ui::layout::rows_in_height(height, window_scale(hwnd))
}

/// Context-menu command ids.
const MENU_REFRESH: usize = 1001;
const MENU_PAUSE: usize = 1002;
const MENU_SIGNIN: usize = 1100;
const MENU_OPEN: usize = 1101;
const MENU_TOPMOST: usize = 1200;
const MENU_CLEAR_CREDS: usize = 1201;
const MENU_PIN_POSITION: usize = 1202;
const MENU_BOTTOMMOST: usize = 1203;
const MENU_ROWS_5: usize = 1400;
const MENU_ROWS_10: usize = 1401;
const MENU_ROWS_15: usize = 1402;
const MENU_ROWS_20: usize = 1403;
const MENU_QUIT: usize = 1300;
/// Font-size submenu items: `MENU_FONT_BASE` = 10 px, `MENU_FONT_BASE + 14` = 24 px.
const MENU_FONT_BASE: usize = 1500;
const MENU_FONT_MAX: usize = MENU_FONT_BASE + 14;

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

struct State {
    app: App,
    worker: Worker,
    fonts: Fonts,
    /// Whether the window currently lacks `WS_EX_NOACTIVATE`, i.e. can take
    /// keyboard focus. Mirrors `app.is_login()`.
    activatable: bool,
    /// Draw scale (monitor physical density × `config.fontSize` / 12.5) the
    /// `fonts` were built for. Tracked so a monitor change can tell how much
    /// to grow the window: the panel's size is the same in logical units on
    /// every monitor, so device pixels must change by the ratio of the two
    /// scales.
    scale: f32,
}

impl State {
    fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
        STATE.with(|s| s.borrow_mut().as_mut().map(f))
    }
}

/// Width of the invisible grab margin along each window edge.
const RESIZE_BORDER: f32 = 6.0;

/// Which window edge the pointer is over, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Edge {
    fn hit_test(self) -> u32 {
        match self {
            Edge::Left => HTLEFT,
            Edge::Right => HTRIGHT,
            Edge::Top => HTTOP,
            Edge::Bottom => HTBOTTOM,
            Edge::TopLeft => HTTOPLEFT,
            Edge::TopRight => HTTOPRIGHT,
            Edge::BottomLeft => HTBOTTOMLEFT,
            Edge::BottomRight => HTBOTTOMRIGHT,
        }
    }

    /// Full-window cursor shown while hovering this edge.
    fn cursor_id(self) -> PCWSTR {
        match self {
            Edge::Left | Edge::Right => IDC_SIZEWE,
            Edge::Top | Edge::Bottom => IDC_SIZENS,
            Edge::TopLeft | Edge::BottomRight => IDC_SIZENWSE,
            Edge::TopRight | Edge::BottomLeft => IDC_SIZENESW,
        }
    }
}

/// Distance from the pointer to whichever edge is within the grab margin.
fn edge_at(m: Metrics, x: f32, y: f32) -> Option<Edge> {
    let (w, h) = (m.width, m.height);
    if x < 0.0 || y < 0.0 || x > w || y > h {
        return None;
    }
    let left = x <= RESIZE_BORDER;
    let right = x >= w - RESIZE_BORDER;
    let top = y <= RESIZE_BORDER;
    let bottom = y >= h - RESIZE_BORDER;
    Some(match (left, right, top, bottom) {
        (true, _, true, _) => Edge::TopLeft,
        (_, true, true, _) => Edge::TopRight,
        (true, _, _, true) => Edge::BottomLeft,
        (_, true, _, true) => Edge::BottomRight,
        (true, _, _, _) => Edge::Left,
        (_, true, _, _) => Edge::Right,
        (_, _, true, _) => Edge::Top,
        (_, _, _, true) => Edge::Bottom,
        _ => return None,
    })
}

/// Decide what the pointer is over, in the coordinates Windows expects from
/// `WM_NCHITTEST`.
///
/// * window margins  -> a resize edge
/// * anything clickable (cards on the list, controls on the sign-in form)
///   -> `HTCLIENT`, otherwise the press would be eaten as a window drag
fn hit_test(m: Metrics, x: f32, y: f32) -> isize {
    let pinned = State::with(|s| s.app.config.pin_position).unwrap_or(false);

    if !pinned {
        if let Some(edge) = edge_at(m, x, y) {
            return edge.hit_test() as isize;
        }
    }

    let clickable = State::with(|s| {
        if s.app.is_login() {
            s.app
                .login
                .as_ref()
                .is_some_and(|form| login::hit(form, m, x, y).is_some())
        } else {
            // Cards are not clickable: the whole surface is a drag handle so
            // a stray click can never open a browser. The request detail page
            // is reachable from the right-click menu instead.
            false
        }
    })
    .unwrap_or(false);

    if clickable || pinned {
        HTCLIENT as isize
    } else {
        HTCAPTION as isize
    }
}

fn metrics_for(hwnd: HWND) -> Metrics {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    Metrics::new(
        (rc.right - rc.left) as f32,
        (rc.bottom - rc.top) as f32,
        window_scale(hwnd),
    )
}

/// Windows' per-monitor DPI setting as a scale over the 96-DPI baseline GDI+
/// fonts are authored at. This is the system's *guess* at density and is used
/// only as a fallback when a monitor reports no usable physical size.
fn windows_dpi_scale(hwnd: HWND) -> f32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 }
}

/// Scale that keeps the panel the same *physical* size on whichever monitor it
/// sits on, so two screens with the same Windows scaling but different
/// pixels-per-inch no longer leave the panel too small on the denser one.
///
/// The scale is the monitor's physical density (device pixels per screen inch,
/// from the EDID-reported glass size) over the 96-DPI authoring baseline, then
/// multiplied by `config.fontSize / 12.5` so the whole panel tracks the
/// user-chosen body font size. `GetDpiForWindow` is the fallback when the
/// driver reports no usable EDID.
fn window_scale(hwnd: HWND) -> f32 {
    // `font_size` is the body font in px at the 96-DPI baseline (authored
    // 12.5); scaling by `font_size / 12.5` keeps the whole panel in proportion
    // with the user-chosen text size.
    let tuner = State::with(|s| s.app.config.font_size).unwrap_or(12.5) / 12.5;
    let scale = monitor_density(hwnd)
        .map(|d| d * tuner)
        .unwrap_or_else(|| windows_dpi_scale(hwnd) * tuner);
    scale.clamp(0.25, 8.0)
}

/// Physical-density scale for the monitor nearest `hwnd`, or `None` when the
/// driver reports no usable EDID size (the common case for which `window_scale`
/// falls back to the Windows DPI guess).
fn monitor_density(hwnd: HWND) -> Option<f32> {
    unsafe {
        let hmon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if !GetMonitorInfoW(hmon, &mut info.monitorInfo).as_bool() {
            return None;
        }
        let rc = info.monitorInfo.rcMonitor;
        let px_w = rc.right - rc.left;
        let px_h = rc.bottom - rc.top;
        // szDevice holds the adapter path (`\\.\DISPLAY1`); CreateDC's driver
        // argument accepts it directly, and a DC on that device reports the
        // monitor's own physical dimensions via GetDeviceCaps.
        if info.szDevice[0] == 0 {
            return None;
        }
        let hdc = CreateDCW(
            PCWSTR(info.szDevice.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            None,
        );
        if hdc.is_invalid() {
            return None;
        }
        let mm_w = GetDeviceCaps(Some(hdc), HORZSIZE) as f32;
        let mm_h = GetDeviceCaps(Some(hdc), VERTSIZE) as f32;
        let _ = DeleteDC(hdc);
        ui::layout::physical_scale(px_w, px_h, mm_w, mm_h)
    }
}

fn main() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // Dark Win32 menus: load SetPreferredAppMode from uxtheme.dll.
        let uxtheme: Vec<u16> = "uxtheme.dll"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        if let Ok(h) = GetModuleHandleW(PCWSTR(uxtheme.as_ptr())) {
            if let Some(proc) =
                GetProcAddress(h, windows::core::PCSTR(b"SetPreferredAppMode\0".as_ptr()))
            {
                let func: unsafe extern "system" fn(u32) -> i32 = std::mem::transmute(proc);
                func(3); // AllowDark
            }
        }

        let mut token = 0usize;
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            DebugEventCallback: 0,
            SuppressBackgroundThread: windows::core::BOOL(0),
            SuppressExternalCodecs: windows::core::BOOL(0),
        };
        let mut output = GdiplusStartupOutput::default();
        if GdiplusStartup(&mut token, &input, &mut output).0 != 0 {
            return;
        }

        let config = Config::load();
        let stored = config::load_stored();

        // Startup precedence for credentials:
        //   1. a token in the configured environment variable (nothing on disk),
        //   2. a stored token,
        //   3. stored email+password, exchanged for a token,
        //   4. the sign-in form.
        let env_token = config::token_from_env(&config.token_env_var);
        let stored_token = stored.as_ref().and_then(|s| s.token.clone());
        let startup_token = env_token
            .clone()
            .or_else(|| stored_token.clone())
            .filter(|t| !token::is_expired(t, now_unix()));

        // An expired token is a routine event (AxonHub issues 7-day tokens with
        // no refresh), so say so rather than letting the first poll 401.
        let expired_notice = startup_token
            .is_none()
            .then(|| {
                env_token
                    .as_ref()
                    .or(stored_token.as_ref())
                    .filter(|t| token::is_expired(t, now_unix()))
                    .map(|t| {
                        format!(
                            "访问令牌已过期({}),请重新获取",
                            token::describe(t, now_unix())
                        )
                    })
            })
            .flatten();

        let worker = Worker::spawn(config.clone(), startup_token.clone());

        let Ok(instance) = windows::Win32::System::LibraryLoader::GetModuleHandleW(None) else {
            return;
        };

        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(CLASS_WIDE.as_ptr()),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let prefill = stored.as_ref().and_then(|s| s.credentials());
        let mut app = App::new(config.clone(), startup_token, prefill.clone());

        if app.token.is_none() {
            // No usable token: collect one, either by signing in with stored
            // credentials or by showing the form.
            match prefill {
                Some(creds) => {
                    if let Some(form) = app.login.as_mut() {
                        // A sign-in is already in flight for these credentials.
                        form.busy = true;
                    }
                    worker.send(Command::SignIn { creds });
                }
                None => {
                    if let Some(form) = app.login.as_mut() {
                        form.error = expired_notice.clone();
                    }
                }
            }
        }

        // The sign-in form needs the keyboard; the request list deliberately
        // does not, so that reading it never steals focus from other windows.
        let wants_focus = app.is_login();
        let state = State {
            app,
            worker,
            fonts: Fonts::load(1.0),
            activatable: wants_focus,
            scale: 1.0,
        };
        STATE.with(|s| *s.borrow_mut() = Some(state));

        let style = WS_POPUP;
        // WS_EX_NOACTIVATE is what keeps the list from stealing focus; it is
        // omitted while the sign-in form is up, since a window that is never
        // activated never receives keyboard input.
        let mut ex_style = WS_EX_TOOLWINDOW;
        if !wants_focus {
            ex_style |= WS_EX_NOACTIVATE;
        }
        if config.always_on_top && !config.always_on_bottom {
            ex_style |= WS_EX_TOPMOST;
        }

        // Create at the saved position with the saved size as a provisional
        // guess. The size that is actually correct depends on the DPI of the
        // monitor the window lands on, and that is only known once the window
        // exists (`GetDpiForWindow`); `GetDpiForMonitor` is deprecated and
        // returns 1.0 for a per-monitor-DPI-aware process, which is why
        // resolving the scale before creation does not work. So: create, read
        // the real scale, then resize to logical x scale.
        let saved = config.window;
        let saved_was_logical = config.logical_window;
        let provisional = if saved.x < 0 && saved.y < 0 {
            // First run: near the top-right of the primary work area. Moved to
            // the exact edge below, once the real size is known.
            let work = work_area_at(0, 0);
            (work.2 - saved.width - 24, work.1 + 24)
        } else {
            (saved.x, saved.y)
        };
        let cls = CLASS_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>();
        let title = "AxonHub 请求面板"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>();
        let Ok(hwnd) = CreateWindowExW(
            ex_style,
            PCWSTR(cls.as_ptr()),
            PCWSTR(title.as_ptr()),
            style,
            provisional.0,
            provisional.1,
            saved.width,
            saved.height,
            None,
            None,
            Some(instance.into()),
            None,
        ) else {
            GdiplusShutdown(token);
            return;
        };

        // Per-monitor DPI: fonts were authored for 96 DPI, so scale them to the
        // monitor the window actually landed on.
        let scale = window_scale(hwnd);
        State::with(|s| {
            s.fonts = Fonts::load(scale);
            s.scale = scale;
        });

        // Rounded corners and a dark title-less frame on Windows 11.
        apply_window_chrome(hwnd);

        // The size is logical from here on; record that so the one-time
        // conversion above is never repeated.
        if !saved_was_logical {
            State::with(|s| {
                s.app.config.logical_window = true;
                s.app.save();
            });
        }

        // Now that the scale is known, size the window so the panel is the same
        // apparent size on this monitor as on any other: the logical size times
        // the scale, with the height snapped to whole cards and clamped to the
        // work area so it can never extend below the screen.
        //
        // A config written before `logicalWindow` existed holds device pixels
        // measured on whatever monitor it was last saved on. Convert it once
        // here, where that monitor's scale is known (it is the one the window
        // just landed on); afterwards the file is written back in logical units.
        let logical_width = if saved_was_logical {
            saved.width
        } else {
            ui::layout::logical_px(saved.width.max(1), scale)
        };
        let work = work_area_for(hwnd);
        let width = ui::layout::device_px(logical_width, scale);
        let height = ui::layout::height_for_rows(config.row_limit.max(1) as usize, scale)
            .min(work.3 - work.1);
        let (x, y) = if saved.x < 0 && saved.y < 0 {
            // First run: pin to the top-right of the primary work area now that
            // the real width is known.
            (work.2 - width - 24, work.1 + 24)
        } else {
            (provisional.0, provisional.1)
        };
        let placed = config::clamp_to_virtual_screen(
            WindowState {
                x,
                y,
                width,
                height,
            },
            work,
            scale,
        );
        let _ = SetWindowPos(
            hwnd,
            None,
            placed.x,
            placed.y,
            placed.width,
            placed.height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );

        let _ = ShowWindow(
            hwnd,
            if wants_focus {
                SW_SHOWNORMAL
            } else {
                SW_SHOWNOACTIVATE
            },
        );
        if wants_focus {
            // Get the keyboard immediately so the user can start typing.
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(Some(hwnd));
        }
        let _ = SetTimer(Some(hwnd), TIMER_POLL, 250, None);
        let _ = SetTimer(Some(hwnd), TIMER_CARET, 530, None);
        let _ = SetTimer(Some(hwnd), TIMER_CLOCK, 1000, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        State::with(|s| s.app.save());
        STATE.with(|s| *s.borrow_mut() = None);
        GdiplusShutdown(token);
    }
}

/// Work area of the monitor `hwnd` currently sits on.
///
/// A null `HWND` would make `MonitorFromWindow` answer for the *primary*
/// monitor, which is wrong the moment the panel lives on a second screen: the
/// saved size would be clamped against the wrong work area and the panel would
/// snap back to the primary monitor on every launch.
fn work_area_for(hwnd: HWND) -> (i32, i32, i32, i32) {
    unsafe {
        let hmon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmon, &mut info).as_bool() {
            return (
                info.rcWork.left,
                info.rcWork.top,
                info.rcWork.right,
                info.rcWork.bottom,
            );
        }
    }
    (0, 0, 1920, 1080)
}

/// Work area of the monitor containing a screen point, for clamping a position
/// that has not been applied to a window yet.
fn work_area_at(x: i32, y: i32) -> (i32, i32, i32, i32) {
    unsafe {
        let hmon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmon, &mut info).as_bool() {
            return (
                info.rcWork.left,
                info.rcWork.top,
                info.rcWork.right,
                info.rcWork.bottom,
            );
        }
    }
    (0, 0, 1920, 1080)
}

fn apply_window_chrome(hwnd: HWND) {
    use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DwmSetWindowAttribute};
    unsafe {
        // DWMWCP_ROUND: match the rounded cards inside the panel.
        let preference: i32 = 2;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &preference as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
        // Dark frame so any system-drawn edge matches the panel background.
        const DWMWA_USE_IMMERSIVE_DARK_MODE: i32 = 20;
        let dark: i32 = 1;
        let _ = DwmSetWindowAttribute(
            hwnd,
            windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(DWMWA_USE_IMMERSIVE_DARK_MODE),
            &dark as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // Side effects are parked here rather than run inside the handlers: calls
    // like ShellExecuteW, SetWindowPos and DestroyWindow pump messages, which
    // re-enters this proc. Anything done while `STATE` is borrowed would then
    // panic on the re-entrant borrow.
    let mut actions: Vec<Action> = Vec::new();

    match msg {
        WM_PAINT => paint(hwnd),
        WM_NCHITTEST => {
            // A borderless WS_POPUP window has neither a sizing frame nor a
            // draggable title, so both are synthesised here. Anything that must
            // receive clicks has to be reported as HTCLIENT: HTCAPTION makes
            // Windows treat the press as a window drag and the control never
            // sees WM_LBUTTONDOWN.
            let mut rc = RECT::default();
            unsafe {
                let _ = GetWindowRect(hwnd, &mut rc);
            }
            let screen_x = (lp.0 & 0xFFFF) as i16 as f32;
            let screen_y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32;
            let x = screen_x - rc.left as f32;
            let y = screen_y - rc.top as f32;
            let m = Metrics::new(
                (rc.right - rc.left) as f32,
                (rc.bottom - rc.top) as f32,
                window_scale(hwnd),
            );

            return LRESULT(hit_test(m, x, y));
        }
        WM_SETCURSOR => {
            let mut cursor = POINT::default();
            let mut rc = RECT::default();
            unsafe {
                let _ = GetCursorPos(&mut cursor);
                let _ = GetWindowRect(hwnd, &mut rc);
            }
            let m = Metrics::new(
                (rc.right - rc.left) as f32,
                (rc.bottom - rc.top) as f32,
                window_scale(hwnd),
            );
            let local_x = (cursor.x - rc.left) as f32;
            let local_y = (cursor.y - rc.top) as f32;
            let pinned = State::with(|s| s.app.config.pin_position).unwrap_or(false);
            let shape = match edge_at(m, local_x, local_y) {
                Some(edge) => edge.cursor_id(),
                // The hand signals "this is clickable"; pinned mode is not, so
                // show an arrow even where `hit_test` reports `HTCLIENT`.
                None if !pinned && hit_test(m, local_x, local_y) == HTCLIENT as isize => IDC_HAND,
                None => IDC_ARROW,
            };
            if let Ok(handle) = unsafe { LoadCursorW(None, shape) } {
                unsafe {
                    SetCursor(Some(handle));
                }
                return LRESULT(1);
            }
        }
        WM_GETMINMAXINFO => {
            // Keep the panel usable: wide enough for the metric row, tall enough
            // for the header and a couple of cards, in device pixels.
            let scale = window_scale(hwnd);
            let info = unsafe { &mut *(lp.0 as *mut MINMAXINFO) };
            info.ptMinTrackSize = POINT {
                x: (320.0 * scale) as i32,
                y: (200.0 * scale) as i32,
            };
            return LRESULT(0);
        }
        WM_EXITSIZEMOVE => {
            // A manual resize changes how many rows fit; remember it and refetch
            // exactly that many so the list fills the window without overflow.
            let rows = current_visible_rows(hwnd);
            clamp_scroll(hwnd);
            State::with(|s| {
                s.app.config.row_limit = rows as i64;
            });
            actions.push(Action::Save);
            actions.push(Action::RefreshRowCount(rows));
        }
        WM_WINDOWPOSCHANGING => {
            // Keep the panel pinned to the bottom of the Z order when enabled:
            // any attempt to raise it is redirected to HWND_BOTTOM so it never
            // floats above other windows.
            if State::with(|s| s.app.config.always_on_bottom).unwrap_or(false) {
                let wp = lp.0 as *mut WINDOWPOS;
                if !wp.is_null() {
                    unsafe {
                        (*wp).hwndInsertAfter = HWND_BOTTOM;
                    }
                }
            }
        }
        WM_ERASEBKGND => {}
        WM_TIMER => on_timer(hwnd, wp.0, &mut actions),
        WM_MOUSEMOVE => on_mouse_move(hwnd, lp, &mut actions),
        WM_MOUSELEAVE => {
            let changed = State::with(|s| {
                let had = s.app.hover.is_some();
                s.app.hover = None;
                had
            });
            if changed == Some(true) {
                invalidate(hwnd);
            }
        }
        WM_LBUTTONDOWN => on_left_down(hwnd, lp, &mut actions),
        WM_LBUTTONUP => on_left_up(hwnd, lp),
        WM_RBUTTONUP | WM_CONTEXTMENU | WM_NCRBUTTONUP => actions.push(Action::ShowMenu),
        WM_MOUSEWHEEL => on_wheel(hwnd, wp, &mut actions),
        WM_KEYDOWN | WM_CHAR | WM_SYSKEYDOWN => on_key(hwnd, msg, wp, &mut actions),
        WM_DPICHANGED => {
            // Dragging the panel between monitors of different scale changes how
            // large everything must be drawn. The layout is authored in logical
            // units and `Metrics` multiplies by the scale per frame, so the
            // *window* has to grow by the same factor or the panel would render
            // a 1.5x layout inside an unscaled box and look cramped on the
            // high-DPI screen. Width and height are both rescaled from the
            // previous scale, then the height is snapped to whole cards.
            let old_scale = State::with(|s| s.scale).unwrap_or(1.0);
            let scale = window_scale(hwnd);
            let ratio = if old_scale > 0.0 {
                scale / old_scale
            } else {
                1.0
            };
            State::with(|s| {
                s.fonts = Fonts::load(scale);
                s.scale = scale;
            });

            let mut rc = RECT::default();
            unsafe {
                let _ = GetWindowRect(hwnd, &mut rc);
            }
            let (mut x, mut y) = (rc.left, rc.top);
            let width = ((rc.right - rc.left) as f32 * ratio).round() as i32;
            let mut height = ((rc.bottom - rc.top) as f32 * ratio).round() as i32;

            // The suggested rectangle carries the new position; take it so the
            // window lands where Windows wants it on the new monitor.
            if lp.0 != 0 {
                let suggested = unsafe { &*(lp.0 as *const RECT) };
                x = suggested.left;
                y = suggested.top;
            }

            // Whole cards only: derive the height from the row count instead of
            // trusting the scaled pixel value, so no partial card is left over.
            let work = work_area_at(x, y);
            let fitted = ui::layout::height_for_rows(config_rows(hwnd), scale).min(work.3 - work.1);
            if fitted > 0 {
                height = fitted;
            }

            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    x,
                    y,
                    width,
                    height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            clamp_scroll(hwnd);
            invalidate(hwnd);
        }
        WM_MOVE => sync_scale(hwnd),
        WM_DISPLAYCHANGE => {
            // A resolution or monitor-configuration change can alter the
            // physical density of the screen the panel sits on without a
            // Windows DPI change, so recompute the scale and refit.
            sync_scale(hwnd);
        }
        WM_COMMAND => on_command(wp, &mut actions),
        WM_CLOSE => actions.push(Action::Quit),
        WM_DESTROY => {
            save_window_state(hwnd);
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_POLL);
                let _ = KillTimer(Some(hwnd), TIMER_CARET);
                let _ = KillTimer(Some(hwnd), TIMER_CLOCK);
                PostQuitMessage(0);
            }
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }

    run_actions(hwnd, actions);
    LRESULT(0)
}

/// Side effects that must run outside a `State` borrow.
enum Action {
    Redraw,
    Quit,
    /// State must be written to disk (after a drag or a settings change).
    Save,
    ShowMenu,
    /// Sign in with the credentials currently in the form.
    SignIn,
    OpenUrl(String),
    /// Apply an always-on-top change to the live window.
    SetTopmost(bool),
    /// Pin the window to the bottom of the Z order; mutually exclusive with topmost.
    SetBottom(bool),
    /// Re-read credentials from disk to prefill the form.
    OpenLogin(Option<String>),
    /// The window was resized by hand; fetch this many rows.
    RefreshRowCount(usize),
    /// Change the row count from the menu and resize the window to fit.
    ResizeToRows(usize),
    /// Change the body font size from the menu and refit the window; the whole
    /// panel scales with it.
    SetFontSize(f32),
    /// Toggle `WS_EX_NOACTIVATE` so the window can (or cannot) take focus.
    SetActivatable(bool),
    ClearCredentials,
    /// Paste the clipboard into the focused sign-in field.
    Paste,
    /// Clear the focused sign-in field.
    ClearField,
}

fn run_actions(hwnd: HWND, mut actions: Vec<Action>) {
    // Actions can queue further actions (e.g. entering the sign-in view also
    // toggles the activation style), so drain rather than iterate once.
    while let Some(action) = actions.pop() {
        match action {
            Action::Redraw => invalidate(hwnd),
            Action::Quit => unsafe {
                let _ = DestroyWindow(hwnd);
            },
            Action::Save => save_window_state(hwnd),
            Action::ShowMenu => show_menu(hwnd),
            Action::SignIn => submit_login(hwnd),
            Action::OpenUrl(url) => open_url_async(&url),
            Action::SetTopmost(on) => {
                let insert_after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
                unsafe {
                    let _ = SetWindowPos(
                        hwnd,
                        Some(insert_after),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
            }
            Action::SetBottom(on) => {
                State::with(|s| s.app.config.always_on_bottom = on);
                unsafe {
                    if on {
                        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                        let _ = SetWindowLongPtrW(
                            hwnd,
                            GWL_EXSTYLE,
                            style & !(WS_EX_TOPMOST.0 as isize),
                        );
                    }
                    let _ = SetWindowPos(
                        hwnd,
                        Some(if on { HWND_BOTTOM } else { HWND_NOTOPMOST }),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
            }
            Action::RefreshRowCount(rows) => {
                let limit = rows.max(1) as i64;
                State::with(|s| {
                    if s.app.config.row_limit != limit {
                        s.app.config.row_limit = limit;
                        s.app.scroll = 0.0;
                        // The worker keeps its own copy of the config, so the new
                        // limit has to be sent to it rather than only stored here.
                        s.worker.send(Command::SetRowLimit(limit));
                    }
                });
            }
            Action::ResizeToRows(rows) => {
                let limit = rows.max(1) as i64;
                State::with(|s| {
                    s.app.config.row_limit = limit;
                    s.app.scroll = 0.0;
                    s.worker.send(Command::SetRowLimit(limit));
                });
                let scale = window_scale(hwnd);
                let work = work_area_for(hwnd);
                let height = ui::layout::height_for_rows(rows, scale).min(work.3 - work.1);
                let mut rc = RECT::default();
                unsafe {
                    let _ = GetWindowRect(hwnd, &mut rc);
                }
                let _ = unsafe {
                    SetWindowPos(
                        hwnd,
                        None,
                        0,
                        0,
                        rc.right - rc.left,
                        height,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                };
                save_window_state(hwnd);
                invalidate(hwnd);
            }
            Action::SetFontSize(size) => {
                // Clamp here too, so a value that slipped past the menu (e.g. a
                // stale config from a future range) cannot blow the layout up.
                let size = size.clamp(10.0, 24.0);
                State::with(|s| s.app.config.font_size = size);
                // Re-derive the draw scale from the new font size, reload the
                // fonts and refit the window — `sync_scale` is the same path a
                // monitor change takes, so the panel stays consistent. It
                // reloads fonts, resizes, clamps scroll and invalidates.
                sync_scale(hwnd);
                save_window_state(hwnd);
            }
            Action::Paste => {
                if let Some(text) = clipboard_text() {
                    State::with(|s| {
                        if let Some(form) = s.app.login.as_mut() {
                            form.paste(&text);
                        }
                    });
                    invalidate(hwnd);
                }
            }
            Action::ClearField => {
                State::with(|s| {
                    if let Some(form) = s.app.login.as_mut() {
                        form.clear_focused();
                    }
                });
                invalidate(hwnd);
            }
            Action::SetActivatable(enabled) => {
                apply_activation(hwnd, enabled);
                State::with(|s| s.activatable = enabled);
            }
            Action::OpenLogin(message) => {
                // Prefill from the stored blob when present; a user switching
                // accounts simply overwrites the fields.
                let prefill = config::load_stored().and_then(|s| s.credentials());
                State::with(|s| s.app.open_login(message, prefill));
                sync_activation(&mut actions);
                invalidate(hwnd);
            }
            Action::ClearCredentials => {
                config::clear_stored();
                // Reopen on the token tab, since a cleared credential is
                // usually followed by pasting a fresh token.
                State::with(|s| {
                    s.app.token = None;
                    s.worker.send(Command::SetToken(String::new()));
                    s.app.open_login(Some("已清除本机保存的凭据".into()), None);
                    if let Some(form) = s.app.login.take() {
                        s.app.login = Some(form.with_token(""));
                    }
                });
                sync_activation(&mut actions);
                invalidate(hwnd);
            }
        }
    }
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn save_window_state(hwnd: HWND) {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rc);
    }
    // Persist the size in logical (96-DPI) units. Storing device pixels would
    // make the remembered size depend on which monitor the panel happened to be
    // on when it was last closed, so reopening on a different screen would come
    // back the wrong size. Position stays in device pixels — it is a screen
    // coordinate, not a length.
    let scale = window_scale(hwnd);
    State::with(|s| {
        s.app.config.logical_window = true;
        s.app.config.window = WindowState {
            x: rc.left,
            y: rc.top,
            width: ui::layout::logical_px((rc.right - rc.left).max(320), scale),
            height: ui::layout::logical_px((rc.bottom - rc.top).max(240), scale),
        };
        s.app.save();
    });
}

fn paint(hwnd: HWND) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let m = Metrics::new(
            (rc.right - rc.left) as f32,
            (rc.bottom - rc.top) as f32,
            window_scale(hwnd),
        );

        // Draw the whole frame into an off-screen bitmap and present it with one
        // BitBlt. Painting the window DC directly lets each invalidate show a
        // partially drawn frame, which the user sees as flicker.
        let mem = CreateCompatibleDC(Some(hdc));
        let bitmap = CreateCompatibleBitmap(hdc, rc.right.max(1), rc.bottom.max(1));
        let previous = SelectObject(mem, HGDIOBJ(bitmap.0));

        let brush = CreateSolidBrush(COLORREF(theme::BG & 0x00FF_FFFF));
        FillRect(mem, &rc, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));

        if let Some(painter) = Painter::new(mem) {
            State::with(|s| {
                if s.app.is_login() {
                    if let Some(form) = s.app.login.as_ref() {
                        login::draw(&painter, &s.fonts, form, m);
                    }
                } else {
                    let view = ListView {
                        rows: &s.app.rows,
                        total: s.app.total,
                        now: now_unix(),
                        scroll: s.app.scroll,
                        hover: s.app.hover,
                        selected: s.app.selected,
                        status_text: s.app.status.clone(),
                        user: s.app.user_name.as_deref(),
                        pinned: s.app.config.pin_position,
                    };
                    panel::draw(&painter, &s.fonts, m, &view);
                }
            });
        }

        let _ = BitBlt(hdc, 0, 0, rc.right, rc.bottom, Some(mem), 0, 0, SRCCOPY);
        SelectObject(mem, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem);
        let _ = EndPaint(hwnd, &ps);
    }
}

fn on_timer(hwnd: HWND, id: usize, actions: &mut Vec<Action>) {
    match id {
        TIMER_POLL => pump_worker(actions),
        // Only the ages change between polls, and only while the list is shown.
        TIMER_CLOCK => {
            if State::with(|s| s.app.is_login()) == Some(false) {
                actions.push(Action::Redraw);
            }
        }
        TIMER_CARET => {
            // Blink the caret while a text field holds focus.
            let blink = State::with(|s| {
                if !s.app.is_login() {
                    return false;
                }
                let Some(form) = s.app.login.as_mut() else {
                    return false;
                };
                if form.focus == Field::Remember {
                    return false;
                }
                form.caret_on = !form.caret_on;
                true
            });
            if blink == Some(true) {
                actions.push(Action::Redraw);
            }
        }
        _ => {}
    }
    let _ = hwnd;
}

/// Apply worker output; the list redraws only when something actually changed.
fn pump_worker(actions: &mut Vec<Action>) {
    let changed = State::with(|s| {
        let updates = s.worker.drain();
        if updates.is_empty() {
            return false;
        }
        let worker = &s.worker;
        s.app.apply_updates(updates, worker);
        true
    });
    if changed == Some(true) {
        actions.push(Action::Redraw);
        sync_activation(actions);
    }
}

/// The window must be activatable exactly while the sign-in form is showing.
/// Called after anything that can change the view, and cheap enough to run
/// unconditionally: it only queues work on an actual transition.
fn sync_activation(actions: &mut Vec<Action>) {
    let wants = State::with(|s| s.app.is_login()).unwrap_or(false);
    let has = State::with(|s| s.activatable).unwrap_or(wants);
    if wants != has {
        actions.push(Action::SetActivatable(wants));
    }
}

/// Add or remove `WS_EX_NOACTIVATE`, taking the keyboard when enabling.
fn apply_activation(hwnd: HWND, enabled: bool) {
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let no_activate = WS_EX_NOACTIVATE.0 as isize;
        let next = if enabled {
            current & !no_activate
        } else {
            current | no_activate
        };
        if next != current {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next);
        }
    }
    if enabled {
        unsafe {
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(Some(hwnd));
        }
    }
}

fn on_mouse_move(hwnd: HWND, lp: LPARAM, actions: &mut Vec<Action>) {
    let x = (lp.0 & 0xFFFF) as i16 as f32;
    let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32;
    let m = metrics_for(hwnd);

    let mut needs_paint = false;
    State::with(|s| {
        if s.app.is_login() {
            return;
        }
        let hover = m.row_at(y, s.app.scroll, s.app.rows.len());
        if hover != s.app.hover {
            s.app.hover = hover;
            needs_paint = true;
        }
    });

    unsafe {
        let mut tme = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        let _ = TrackMouseEvent(&mut tme);
    }
    let _ = x;
    if needs_paint {
        actions.push(Action::Redraw);
    }
}

fn on_left_down(hwnd: HWND, lp: LPARAM, actions: &mut Vec<Action>) {
    let x = (lp.0 & 0xFFFF) as i16 as f32;
    let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32;
    let m = metrics_for(hwnd);

    let mut redraw = false;

    State::with(|s| {
        if s.app.is_login() {
            let action = s.app.login.as_ref().and_then(|f| login::hit(f, m, x, y));
            redraw = true;
            match action {
                Some(LoginAction::Submit) => actions.push(Action::SignIn),
                Some(LoginAction::CycleMode) => {
                    if let Some(f) = s.app.login.as_mut() {
                        let next = match f.mode {
                            CredentialMode::None => CredentialMode::Token,
                            CredentialMode::Token => CredentialMode::Password,
                            CredentialMode::Password => CredentialMode::None,
                        };
                        f.set_mode(next);
                    }
                }
                Some(LoginAction::SetMethod(method)) => {
                    if let Some(f) = s.app.login.as_mut() {
                        f.method = method;
                        f.focus = match method {
                            Method::Password => Field::Email,
                            Method::Token => Field::Token,
                        };
                        f.error = None;
                        f.caret_on = true;
                    }
                }
                Some(LoginAction::Focus(field)) => {
                    if let Some(f) = s.app.login.as_mut() {
                        f.focus = field;
                        f.caret_on = true;
                    }
                }
                None => {}
            }
            return;
        }

        // Selection is recorded here so the row highlights on press; the
        // browser is only opened on release (see `on_left_up`). Pinned mode is
        // display-only: a click must not select a card, only the right-click
        // menu opens the detail page.
        if !s.app.config.pin_position {
            if let Some(index) = panel::hit_row(m, &list_view(s), x, y) {
                s.app.selected = Some(index);
            }
        }
    });

    if redraw {
        actions.push(Action::Redraw);
    }
    let _ = hwnd;
}

fn on_left_up(hwnd: HWND, lp: LPARAM) {
    let x = (lp.0 & 0xFFFF) as i16 as f32;
    let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32;
    let m = metrics_for(hwnd);

    // Only open the request if the pointer is still over the row it went down
    // on, so dragging off a card cancels the action. Pinned mode is display-only.
    let url = State::with(|s| {
        if s.app.is_login() || s.app.config.pin_position {
            return None;
        }
        let index = panel::hit_row(m, &list_view(s), x, y)?;
        if s.app.selected != Some(index) {
            return None;
        }
        s.app.click_row(index)
    })
    .flatten();

    if let Some(url) = url {
        open_url_async(&url);
    }
}

fn list_view(s: &State) -> ListView<'_> {
    ListView {
        rows: &s.app.rows,
        total: s.app.total,
        now: now_unix(),
        scroll: s.app.scroll,
        hover: s.app.hover,
        selected: s.app.selected,
        status_text: None,
        user: None,
        pinned: s.app.config.pin_position,
    }
}

fn on_wheel(hwnd: HWND, wp: WPARAM, actions: &mut Vec<Action>) {
    let delta = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
    let m = metrics_for(hwnd);
    State::with(|s| {
        if s.app.is_login() {
            return;
        }
        let step = m.pitch();
        let max = m.max_scroll(s.app.rows.len());
        s.app.scroll = (s.app.scroll - delta * step).clamp(0.0, max);
    });
    actions.push(Action::Redraw);
}

fn on_key(hwnd: HWND, msg: u32, wp: WPARAM, actions: &mut Vec<Action>) {
    // WM_CHAR carries text; WM_KEYDOWN carries the virtual key.
    if msg == WM_CHAR {
        let Some(ch) = char::from_u32(wp.0 as u32) else {
            return;
        };
        if matches!(ch, '\r' | '\t' | '\u{8}' | '\u{1b}') {
            return;
        }
        // Ctrl+V and friends also produce WM_CHAR; the KEYDOWN handler already
        // acted on them, so typing them again would duplicate the paste.
        if ctrl_down() {
            return;
        }
        let changed = State::with(|s| s.app.login.as_mut().map(|f| f.insert(ch)));
        if changed.flatten().is_some() {
            actions.push(Action::Redraw);
        }
        return;
    }

    let vk = wp.0 as u16;

    // Clipboard shortcuts. Handled here because this window draws its own
    // controls: there is no edit control to implement Ctrl+V for us.
    if ctrl_down() && State::with(|s| s.app.is_login() == true).unwrap_or(false) {
        // A plain 'v' arrives as WM_CHAR too, so swallow that one; see below.
        match vk {
            v if v == VK_V.0 => {
                actions.push(Action::Paste);
                return;
            }
            // Clear the focused field: the panel has no selection model, so
            // Ctrl+A and Ctrl+X both mean "replace this value".
            v if v == VK_A.0 || v == VK_X.0 || v == VK_C.0 => {
                actions.push(Action::ClearField);
                return;
            }
            _ => {}
        }
    }

    let mut quit = false;
    let mut submit = false;
    let handled = State::with(|s| {
        if s.app.is_login() {
            let Some(form) = s.app.login.as_mut() else {
                return false;
            };
            match vk {
                v if v == VK_BACK.0 => form.backspace(),
                v if v == VK_TAB.0 => form.focus_next(false),
                v if v == VK_RETURN.0 => submit = true,
                v if v == VK_ESCAPE.0 => quit = true,
                _ => return false,
            }
            return true;
        }
        if vk == VK_ESCAPE.0 {
            quit = true;
            return true;
        }
        false
    });

    if quit {
        actions.push(Action::Quit);
        return;
    }
    if submit {
        actions.push(Action::SignIn);
    } else if handled == Some(true) {
        actions.push(Action::Redraw);
    }
    let _ = hwnd;
}

/// Kick off sign-in with the form's current contents.
/// A prepared submission, produced under the state borrow and acted on after.
enum Submission {
    Password(Credentials),
    Token(String),
}

fn submit_login(hwnd: HWND) {
    let prepared = State::with(|s| {
        let form = s.app.login.as_mut()?;
        if form.busy {
            return None;
        }
        if let Err(message) = form.validate() {
            form.error = Some(message);
            return None;
        }
        form.busy = true;
        form.error = None;
        s.app.config.endpoint = form.endpoint.trim().to_string();
        // The mode chosen in the form is what governs persistence, so it has to
        // reach the config before `store` consults it. Without this the click on
        // `凭据:` was cosmetic and the token was dropped on the floor.
        s.app.config.credential_mode = form.mode;

        let submission = match form.method {
            Method::Password => Submission::Password(form.credentials()),
            Method::Token => Submission::Token(form.trimmed_token()),
        };
        let mode = form.mode;
        Some((submission, mode))
    });

    let Some((submission, mode)) = prepared.flatten() else {
        invalidate(hwnd);
        return;
    };

    match submission {
        Submission::Password(creds) => {
            // Remember what to persist once a token comes back; the password is
            // only written when the mode allows it.
            State::with(|s| {
                s.app.stored.email = creds.email.clone();
                s.app.stored.password = Some(creds.password.clone());
                s.app.save();
                s.worker.send(Command::SignIn { creds });
            });
        }
        Submission::Token(token) => {
            // A pasted token is used directly: no sign-in round trip, and the
            // password never enters the process.
            State::with(|s| {
                let stored = Stored {
                    email: String::new(),
                    password: None,
                    token: (mode != CredentialMode::None).then(|| token.clone()),
                };
                s.app.config.store(&stored);
                s.app.save();
                s.app.stored = stored;
                s.app.token = Some(token.clone());
                s.app.view = View::List;
                s.app.status = Some(("使用访问令牌".into(), false));
                s.worker.send(Command::SetToken(token));
            });
        }
    }
    invalidate(hwnd);
}

fn on_command(wp: WPARAM, actions: &mut Vec<Action>) {
    let id = (wp.0 & 0xFFFF) as usize;
    State::with(|s| match id {
        MENU_REFRESH => s.worker.send(Command::RefreshNow),
        MENU_PAUSE => {
            s.app.paused = !s.app.paused;
            s.worker.send(Command::Pause(s.app.paused));
        }
        MENU_TOPMOST => {
            let on = !s.app.config.always_on_top;
            s.app.config.always_on_top = on;
            if on {
                s.app.config.always_on_bottom = false;
                actions.push(Action::SetBottom(false));
            }
            actions.push(Action::SetTopmost(on));
        }
        MENU_BOTTOMMOST => {
            let on = !s.app.config.always_on_bottom;
            s.app.config.always_on_bottom = on;
            if on {
                s.app.config.always_on_top = false;
            }
            actions.push(Action::SetBottom(on));
        }
        MENU_PIN_POSITION => {
            s.app.config.pin_position = !s.app.config.pin_position;
        }
        MENU_OPEN => {
            let url = format!(
                "{}/project/requests",
                s.app.config.endpoint.trim_end_matches('/')
            );
            actions.push(Action::OpenUrl(url));
        }
        MENU_CLEAR_CREDS => actions.push(Action::ClearCredentials),
        MENU_SIGNIN => actions.push(Action::OpenLogin(None)),
        MENU_QUIT => actions.push(Action::Quit),
        MENU_ROWS_5 => actions.push(Action::ResizeToRows(5)),
        MENU_ROWS_10 => actions.push(Action::ResizeToRows(10)),
        MENU_ROWS_15 => actions.push(Action::ResizeToRows(15)),
        MENU_ROWS_20 => actions.push(Action::ResizeToRows(20)),
        _ if (MENU_FONT_BASE..=MENU_FONT_MAX).contains(&id) => {
            actions.push(Action::SetFontSize((10 + (id - MENU_FONT_BASE)) as f32));
        }
        _ => return,
    });
    // Settings changed by a menu command are persisted after the borrow ends.
    if id == MENU_TOPMOST || id == MENU_PIN_POSITION || id == MENU_BOTTOMMOST {
        actions.push(Action::Save);
    }
    actions.push(Action::Redraw);
}

fn show_menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);

        let label =
            |text: &str| -> Vec<u16> { text.encode_utf16().chain(std::iter::once(0)).collect() };

        let add = |text: &str, id: usize, checked: bool, enabled: bool| {
            let mut wide = label(text);
            let mut flags = MF_STRING;
            if checked {
                flags |= MF_CHECKED;
            }
            if !enabled {
                flags |= MF_GRAYED;
            }
            let _ = AppendMenuW(menu, flags, id, PCWSTR(wide.as_mut_ptr()));
        };

        let (pinned, bottom) =
            State::with(|s| (s.app.config.pin_position, s.app.config.always_on_bottom))
                .unwrap_or((false, false));
        // Row count submenu.
        let current_rows = State::with(|s| s.app.config.row_limit as usize).unwrap_or(12);
        if let Ok(rows_menu) = CreatePopupMenu() {
            let items: &[(&str, usize, usize)] = &[
                ("5 条", MENU_ROWS_5, 5),
                ("10 条", MENU_ROWS_10, 10),
                ("15 条", MENU_ROWS_15, 15),
                ("20 条", MENU_ROWS_20, 20),
            ];
            for &(text, mid, count) in items {
                let mut wide = text
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect::<Vec<u16>>();
                let mut flags = MF_STRING;
                if current_rows == count {
                    flags |= MF_CHECKED;
                }
                let _ = AppendMenuW(rows_menu, flags, mid, PCWSTR(wide.as_mut_ptr()));
            }
            let mut sub_label = "显示数量"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>();
            let _ = AppendMenuW(
                menu,
                MF_STRING | MF_POPUP,
                rows_menu.0 as usize,
                PCWSTR(sub_label.as_mut_ptr()),
            );
        }
        // Font-size submenu: body font in px at the 96-DPI baseline, 10–24.
        let current_font = State::with(|s| s.app.config.font_size).unwrap_or(12.5);
        if let Ok(font_menu) = CreatePopupMenu() {
            for size in 10..=24 {
                let text = size.to_string();
                let mut wide = text
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect::<Vec<u16>>();
                let mid = MENU_FONT_BASE + (size - 10) as usize;
                let mut flags = MF_STRING;
                // Only an exact integer match is checked, so the authored
                // 12.5 default shows no tick until the user picks a size.
                if (current_font - size as f32).abs() < 0.01 {
                    flags |= MF_CHECKED;
                }
                let _ = AppendMenuW(font_menu, flags, mid, PCWSTR(wide.as_mut_ptr()));
            }
            let mut sub_label = "字号"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>();
            let _ = AppendMenuW(
                menu,
                MF_STRING | MF_POPUP,
                font_menu.0 as usize,
                PCWSTR(sub_label.as_mut_ptr()),
            );
        }
        add("刷新", MENU_REFRESH, false, true);
        add("打开请求页", MENU_OPEN, false, true);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        add("清除保存的凭据", MENU_CLEAR_CREDS, false, true);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        add("固定窗口位置", MENU_PIN_POSITION, pinned, true);
        add("置底", MENU_BOTTOMMOST, bottom, true);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        add("退出", MENU_QUIT, false, true);

        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            cursor.x,
            cursor.y,
            Some(0),
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);

        if command.0 != 0 {
            let _ = SetForegroundWindow(hwnd);
            let _ = PostMessageW(
                Some(hwnd),
                WM_COMMAND,
                WPARAM(command.0 as usize),
                LPARAM(0),
            );
        }
    }
}

/// Open a URL in the default browser without stealing focus from the panel.
fn open_url_async(url: &str) {
    let wide_op: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
    let wide_url: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = ShellExecuteW(
            None,
            PCWSTR(wide_op.as_ptr()),
            PCWSTR(wide_url.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNOACTIVATE,
        );
    }
}
