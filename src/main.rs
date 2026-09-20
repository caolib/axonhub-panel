//! Window lifetime, message loop and input wiring.

#![windows_subsystem = "windows"]

mod app;
mod client;
mod config;
mod format;
mod font_catalog;
mod model;
mod popup;
mod settings_window;
mod theme;
mod time;
mod token;
mod ui;
mod worker;

use std::cell::RefCell;

use ui::clipboard::{clipboard_text, set_clipboard_text};

use windows::Win32::Foundation::{
    COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDCW, CreateSolidBrush,
    DeleteDC, DeleteObject, EndPaint, FillRect, GetDC, GetDeviceCaps, GetMonitorInfoW, HDC,
    HGDIOBJ, HORZSIZE, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MONITORINFOEXW,
    MonitorFromPoint, MonitorFromRect, MonitorFromWindow, PAINTSTRUCT, ReleaseDC, SRCCOPY,
    SelectObject, VERTSIZE,
};
use windows::Win32::Graphics::GdiPlus::{
    GdiplusShutdown, GdiplusStartup, GdiplusStartupInput, GdiplusStartupOutput,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, SetFocus, VK_A, VK_C, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_HOME, VK_LEFT,
    VK_NEXT, VK_PRIOR, VK_RIGHT, VK_SHIFT, VK_UP, VK_V, VK_X,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_BACK, VK_ESCAPE, VK_RETURN, VK_TAB,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, *,
};
use windows::core::PCWSTR;

use app::{App, LoginTarget, View};
use config::{Account, Config, CredentialMode, Credentials, Stored, WindowState};
use theme::{Fonts, Painter};
use time::now_unix;
use ui::detail::{self, Button};
use ui::layout::Metrics;
use ui::login::{self, Action as LoginAction, Field, Method};
use ui::panel::{self, ListView};
use worker::{Command, Target, Update, Worker};

const CLASS_NAME: &str = "AHPanelWindow";
/// The error-detail popup's class. A second window rather than an overlay: the
/// document is larger than the panel and outlives any single frame of it.
const DETAIL_CLASS: &str = "AHDetailWindow";
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

/// Whether Ctrl (or Shift for Ctrl+Shift+V) is held.
fn ctrl_down() -> bool {
    unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 }
}

/// Whether Shift is held, which extends the sign-in form's selection.
fn shift_down() -> bool {
    unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 }
}

/// Left button still down during a mouse move, i.e. a drag rather than a hover.
const MK_LBUTTON: usize = 0x0001;

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
        s.fonts = Fonts::load(scale, &s.app.config.font_family);
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
    let single = single_line_mode();
    let fitted = ui::layout::height_for_rows(config_rows(hwnd), scale, single).min(work.3 - work.1);
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
        single,
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

/// Whether cards are drawn as one line instead of two. Read on its own rather
/// than inside another `State::with`: `window_scale` already borrows the state,
/// and nesting the borrow would panic.
fn single_line_mode() -> bool {
    State::with(|s| s.app.config.single_line).unwrap_or(false)
}

/// Rows that fit in the window at its current size.
fn current_visible_rows(hwnd: HWND) -> usize {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    let height = (rc.bottom - rc.top) as f32;
    ui::layout::rows_in_height(height, window_scale(hwnd), single_line_mode())
}

/// Context-menu command ids.
const MENU_REFRESH: usize = 1001;
const MENU_SETTINGS: usize = 1002;
const MENU_OPEN: usize = 1101;
const MENU_TOPMOST: usize = 1200;
const MENU_PIN_POSITION: usize = 1202;
const MENU_BOTTOMMOST: usize = 1203;
const MENU_ROWS_5: usize = 1400;
const MENU_ROWS_10: usize = 1401;
const MENU_ROWS_15: usize = 1402;
const MENU_ROWS_20: usize = 1403;
const MENU_QUIT: usize = 1300;
/// Toggle the one-line card layout.
const MENU_SINGLE_LINE: usize = 1204;
/// Font-size submenu items: `MENU_FONT_BASE` = 10 px, `MENU_FONT_BASE + 14` = 24 px.
const MENU_FONT_BASE: usize = 1500;
const MENU_FONT_MAX: usize = MENU_FONT_BASE + 14;

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// The error-detail popup, while it is open. There is at most one: opening a
/// second request's detail replaces the content instead of stacking windows.
struct DetailPopup {
    hwnd: HWND,
    /// The card the popup was opened from. The fetched executions belong to it.
    row: model::Row,
    /// Deep link to the request's page in the console.
    url: String,
    /// The upstream attempts, once fetched. Kept so switching between the
    /// compact and the full document does not need another round trip.
    executions: Option<Vec<model::ExecutionDetail>>,
    /// Rendered from `executions` for the current `depth`.
    doc: Option<detail::Doc>,
    /// Why the fetch failed; replaces the document when set.
    error: Option<String>,
    /// Whether the document is the compact or the full one.
    depth: detail::Depth,
    scroll: f32,
    /// Content height as measured by the last paint, so a wheel event between
    /// two frames can clamp the offset without laying the document out again.
    content_h: f32,
    hover: Option<Button>,
}

struct State {
    app: App,
    worker: Worker,
    fonts: Fonts,
    detail: Option<DetailPopup>,
    settings: Option<settings_window::Popup>,
    settings_login: Option<settings_window::LoginReturn>,
    /// Whether the window currently lacks `WS_EX_NOACTIVATE`, i.e. can take
    /// keyboard focus. Mirrors `app.is_login()`.
    activatable: bool,
    /// Whether a mouse drag inside the sign-in form is in progress, i.e. the
    /// button went down on a field and has not been released yet.
    login_drag: bool,
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
            // A card is otherwise a drag handle, so a stray click can never
            // open anything; a card with something to explain — a request that
            // failed, or one that only succeeded after a failed attempt — is the
            // exception, since there the reason is one click away. The header
            // filter chips are clickable too.
            let view = list_view(s);
            panel::settings_button(m).contains(x, y)
                || panel::hit_filter(m, &view, x, y).is_some()
                || panel::hit_error_row(m, &view, x, y).is_some()
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
    .with_single_line(single_line_mode())
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

/// 初始化文件日志。在 `main` 中任何可能失败的操作之前调用一次。
///
/// 日志写到 `base_dir()/panel.log`：这是 GUI 程序，控制台不可见，文件才有意义。
/// 初始化失败（目录无法创建、文件无法打开）一律容错——面板照常运行，只是失去
/// 审计轨迹。幂等：重复调用不会 panic，只保留首个 subscriber。
fn init_logging() {
    let dir = config::base_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        // 连日志目录都建不了，放弃文件日志（不可见，无需崩溃）。
        return;
    }
    let path = dir.join("panel.log");

    // 每次写事件重新打开并追加：实现简单、跨线程安全，且无需日志轮转。
    // 失败时退化为 `io::sink`，绝不 panic。
    let make_writer = {
        let path = path.clone();
        move || -> Box<dyn std::io::Write + Send> {
            match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                Ok(file) => Box::new(file),
                Err(_) => Box::new(std::io::sink()),
            }
        }
    };

    let subscriber = tracing_subscriber::fmt()
        .with_writer(make_writer)
        .with_ansi(false)
        .with_target(true)
        .with_file(true)
        .with_line_number(true)
        .with_max_level(tracing::Level::INFO)
        .finish();

    // 幂等：已初始化则保留原有 subscriber，忽略错误。
    let _ = tracing::subscriber::set_global_default(subscriber);
}

fn main() {
    unsafe {
        init_logging();
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

        let mut config = Config::load();
        let mut stored = config::load_stored().unwrap_or_default();

        // An install from before multiple accounts carries one token in the
        // blob; fold it into the account list so it keeps working.
        if config::migrate_single_account(&mut config, &mut stored) {
            config.save();
            config.store(&stored);
        }

        // The environment variable is a single token, so it stands in for the
        // first account: exporting a fresh token is how a headless setup
        // refreshes one of them. With no accounts at all it also names the
        // account, from the endpoint it points at.
        let env_token = config::token_from_env(&config.token_env_var);
        if let Some(token) = &env_token {
            match config.accounts.first_mut() {
                Some(first) => {
                    let id = first.id.clone();
                    stored.set_token(&id, token);
                }
                None => {
                    let id = "a1".to_string();
                    config.upsert_account(Account {
                        id: id.clone(),
                        name: config::endpoint_host(&config.endpoint),
                        endpoint: config.endpoint.clone(),
                        project_id: config.project_id.clone(),
                    });
                    stored.set_token(&id, token);
                    config.save();
                }
            }
            config.store(&stored);
        }

        let targets = targets_from(&config, &stored);

        // An expired token is a routine event (AxonHub issues 7-day tokens with
        // no refresh), so say so rather than letting the first poll 401.
        let expired_notice = config
            .accounts
            .iter()
            .find(|a| {
                stored
                    .token_for(&a.id)
                    .is_some_and(|t| token::is_expired(&t, now_unix()))
            })
            .and_then(|a| {
                stored.token_for(&a.id).map(|t| {
                    format!(
                        "账号 {} 的访问令牌已过期({}),请重新获取",
                        a.name,
                        token::describe(&t, now_unix())
                    )
                })
            });

        let worker = Worker::spawn(config.clone(), targets.clone());

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

        // The detail popup's class: same drawing, separate lifetime.
        let detail_class: Vec<u16> = DETAIL_CLASS
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let detail_wc = WNDCLASSW {
            lpfnWndProc: Some(detail_proc),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(detail_class.as_ptr()),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&detail_wc);

        let mut app = App::new(config.clone());
        app.stored = stored.clone();
        if !targets.iter().any(|t| t.token.is_some()) {
            // No account can be polled: collect a token. An existing account is
            // re-signed-in from its own form; a fresh install starts on a new
            // one.
            let target = match config.accounts.first() {
                Some(account) => LoginTarget::Account(account.id.clone()),
                None => LoginTarget::Add,
            };
            app.open_login(expired_notice.clone(), target);
        }

        // The sign-in form needs the keyboard; the request list deliberately
        // does not, so that reading it never steals focus from other windows.
        let wants_focus = app.is_login();
        let state = State {
            app,
            worker,
            fonts: Fonts::load(1.0, &config.font_family),
            detail: None,
            settings: None,
            settings_login: None,
            activatable: wants_focus,
            login_drag: false,
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
            s.fonts = Fonts::load(scale, &s.app.config.font_family);
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
        let height = ui::layout::height_for_rows(
            config.row_limit.max(1) as usize,
            scale,
            config.single_line,
        )
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
            config.single_line,
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

/// Work area of the monitor a proposed window rectangle belongs to, i.e. the
/// one it overlaps most. Used while a move is still in flight: `MonitorFromWindow`
/// would answer for the monitor the window is *leaving*, which is exactly the
/// wrong screen to clamp a crossing drag against.
fn work_area_of_rect(rect: &RECT) -> (i32, i32, i32, i32) {
    unsafe {
        let hmon = MonitorFromRect(rect, MONITOR_DEFAULTTONEAREST);
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

/// Pull a proposed *outer* window rectangle back inside `work`. The size is
/// kept when it fits and shrunk to the work area when it does not, so the
/// result is always wholly visible — the panel has no title bar, so a piece
/// left off-screen could not be grabbed back.
fn clamp_window_rect(rect: RECT, work: (i32, i32, i32, i32), hwnd: HWND) -> WindowState {
    config::clamp_to_virtual_screen(
        WindowState {
            x: rect.left,
            y: rect.top,
            width: rect.right - rect.left,
            height: rect.bottom - rect.top,
        },
        work,
        window_scale(hwnd),
        single_line_mode(),
    )
}

/// Apply `clamp_window_rect` to a pending position/size change, honouring the
/// `SWP_NOMOVE`/`SWP_NOSIZE` bits so a caller that only wants to resize does
/// not get its origin moved.
fn constrain_to_work_area(hwnd: HWND, wp: &mut WINDOWPOS) {
    let reposition = !wp.flags.contains(SWP_NOMOVE);
    let resize = !wp.flags.contains(SWP_NOSIZE);
    if !reposition && !resize {
        return;
    }
    let mut current = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut current);
    }
    let (left, top) = if reposition {
        (wp.x, wp.y)
    } else {
        (current.left, current.top)
    };
    let (width, height) = if resize {
        (wp.cx, wp.cy)
    } else {
        (current.right - current.left, current.bottom - current.top)
    };
    if width <= 0 || height <= 0 {
        return;
    }
    let proposed = RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    };
    let placed = clamp_window_rect(proposed, work_area_of_rect(&proposed), hwnd);
    if reposition {
        wp.x = placed.x;
        wp.y = placed.y;
    }
    if resize {
        wp.cx = placed.width;
        wp.cy = placed.height;
    }
}

/// Re-clamp the live window against the work area of the monitor it sits on now.
///
/// The modal loop behind a hand move is position-only, so a panel stretched on a
/// large monitor would still be too big for a smaller one it was just dragged
/// onto — `WM_MOVING` cannot shrink it, this can.
fn fit_window_to_work_area(hwnd: HWND) {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rect);
    }
    let placed = clamp_window_rect(rect, work_area_for(hwnd), hwnd);
    if placed.x == rect.left
        && placed.y == rect.top
        && placed.width == rect.right - rect.left
        && placed.height == rect.bottom - rect.top
    {
        return;
    }
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
            )
            .with_single_line(single_line_mode());

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
            )
            .with_single_line(single_line_mode());
            let local_x = (cursor.x - rc.left) as f32;
            let local_y = (cursor.y - rc.top) as f32;
            // The sign-in form's text rows are the one place where the pointer
            // means "put the caret here", so they get the I-beam.
            let over_text = State::with(|s| {
                s.app.is_login()
                    && s.app
                        .login
                        .as_ref()
                        .is_some_and(|f| login::hit_text_field(f, m, local_x, local_y))
            })
            .unwrap_or(false);
            let pinned = State::with(|s| s.app.config.pin_position).unwrap_or(false);
            let shape = if over_text {
                IDC_IBEAM
            } else {
                match edge_at(m, local_x, local_y) {
                    Some(edge) => edge.cursor_id(),
                    // The hand signals "this is clickable"; pinned mode is not,
                    // so show an arrow even where `hit_test` reports `HTCLIENT`.
                    None if !pinned && hit_test(m, local_x, local_y) == HTCLIENT as isize => IDC_HAND,
                    None => IDC_ARROW,
                }
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
            // for the header and a couple of cards, in device pixels. The floor
            // is lower in single-line mode, where the five-row preset is only
            // ~176 logical pixels tall.
            let scale = window_scale(hwnd);
            let min_h = if single_line_mode() { 110.0 } else { 200.0 };
            let info = unsafe { &mut *(lp.0 as *mut MINMAXINFO) };
            info.ptMinTrackSize = POINT {
                x: (320.0 * scale) as i32,
                y: (min_h * scale) as i32,
            };
            return LRESULT(0);
        }
        WM_EXITSIZEMOVE => {
            // A hand move or resize just ended. The modal loop only moves a
            // window, so a panel stretched on a larger monitor is still too big
            // for the one it was just dragged onto; make it fit before deriving
            // the row count from its height.
            fit_window_to_work_area(hwnd);
            // The account form has a temporary working size; moving/resizing
            // it must not turn that height into a new request-count setting.
            if State::with(|s| s.settings_login.is_some()).unwrap_or(false) {
                save_window_state(hwnd);
                return LRESULT(0);
            }
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
        WM_SIZING => {
            // Hand resize: keep the drag rectangle inside the work area of the
            // monitor it is being dragged over, so the panel cannot be stretched
            // off a screen either.
            if lp.0 != 0 {
                let rect = unsafe { &mut *(lp.0 as *mut RECT) };
                let proposed = *rect;
                let placed = clamp_window_rect(proposed, work_area_of_rect(&proposed), hwnd);
                *rect = RECT {
                    left: placed.x,
                    top: placed.y,
                    right: placed.x + placed.width,
                    bottom: placed.y + placed.height,
                };
            }
        }
        WM_MOVING => {
            // The user is dragging the panel. Clamp the drag rectangle to the
            // work area of the monitor the cursor is over: the panel follows the
            // pointer onto whichever screen it is on, but can never be dragged
            // half off one.
            if lp.0 != 0 {
                let mut cursor = POINT::default();
                unsafe {
                    let _ = GetCursorPos(&mut cursor);
                }
                let rect = unsafe { &mut *(lp.0 as *mut RECT) };
                let placed = clamp_window_rect(*rect, work_area_at(cursor.x, cursor.y), hwnd);
                *rect = RECT {
                    left: placed.x,
                    top: placed.y,
                    right: placed.x + placed.width,
                    bottom: placed.y + placed.height,
                };
            }
        }
        WM_WINDOWPOSCHANGING => {
            let wp = lp.0 as *mut WINDOWPOS;
            if !wp.is_null() {
                // Keep the panel pinned to the bottom of the Z order when
                // enabled: any attempt to raise it is redirected to HWND_BOTTOM
                // so it never floats above other windows.
                if State::with(|s| s.app.config.always_on_bottom && !s.app.is_login())
                    .unwrap_or(false)
                {
                    unsafe {
                        (*wp).hwndInsertAfter = HWND_BOTTOM;
                    }
                }
                // Every move and resize funnels through here — a hand drag, the
                // row-count menu, a monitor change — so this is the one place
                // that can keep the borderless panel wholly on a screen.
                unsafe {
                    constrain_to_work_area(hwnd, &mut *wp);
                }
            }
        }
        WM_ERASEBKGND => {}
        WM_TIMER => on_timer(hwnd, wp.0, &mut actions),
        WM_MOUSEMOVE => on_mouse_move(hwnd, wp, lp, &mut actions),
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
        WM_LBUTTONUP => on_left_up(hwnd, lp, &mut actions),
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
                s.fonts = Fonts::load(scale, &s.app.config.font_family);
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
            let fitted = ui::layout::height_for_rows(config_rows(hwnd), scale, single_line_mode())
                .min(work.3 - work.1);
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
            // Windows DPI change, so recompute the scale and refit. `sync_scale`
            // returns early when the scale is unchanged, so the work-area clamp
            // is applied separately: a monitor that was just unplugged must not
            // leave the panel stranded off-screen.
            sync_scale(hwnd);
            fit_window_to_work_area(hwnd);
        }
        WM_COMMAND => on_command(wp, &mut actions),
        WM_CLOSE => actions.push(Action::Quit),
        WM_DESTROY => {
            save_window_state(hwnd);
            if let Some(popup) = State::with(|s| s.settings.as_ref().map(|p| p.hwnd)).flatten() {
                unsafe {
                    let _ = DestroyWindow(popup);
                }
            }
            // Owned windows go down with their owner anyway; closing the popup
            // here keeps the teardown explicit and its state cleanup ordered.
            if let Some(popup) = State::with(|s| s.detail.take().map(|p| p.hwnd)).flatten() {
                unsafe {
                    let _ = DestroyWindow(popup);
                }
            }
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
    ShowSettings(POINT),
    ReturnToSettings,
    /// Sign in with the credentials currently in the form.
    SignIn,
    OpenUrl(String),
    /// Apply an always-on-top change to the live window.
    SetTopmost(bool),
    /// Pin the window to the bottom of the Z order; mutually exclusive with topmost.
    SetBottom(bool),
    /// Forget one saved account and its stored secrets.
    RemoveAccount(String),
    /// The window was resized by hand; fetch this many rows.
    RefreshRowCount(usize),
    /// Change the row count from the menu and resize the window to fit.
    ResizeToRows(usize),
    /// Change the body font size from the menu and refit the window; the whole
    /// panel scales with it.
    SetFontSize(f32),
    SetFontFamily(String),
    /// Switch between the two-line and single-line card layouts. The row count
    /// is kept and the window height refitted to match.
    SetSingleLine(bool),
    /// Toggle `WS_EX_NOACTIVATE` so the window can (or cannot) take focus.
    SetActivatable(bool),
    /// Open the error-detail popup for a row of the list.
    ShowDetail(usize, POINT),
    /// Switch the open detail document between compact and full.
    ToggleDetail,
    /// Close the window whose own proc raised this action.
    CloseWindow,
    /// Copy the open detail document to the clipboard.
    CopyDetail,
    ClearCredentials,
    /// Paste the clipboard into the focused sign-in field.
    Paste,
    /// Clear the focused sign-in field.
    ClearField,
    /// Put the form's selection on the clipboard.
    CopySelection,
    /// Cut the form's selection: copy it, then remove it. With no selection
    /// this clears the field, which is what the old Ctrl+X did.
    CutSelection,
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
            Action::ShowSettings(point) => {
                if State::with(|s| s.settings_login.is_some()).unwrap_or(false) {
                    settings_window::finish_login(hwnd, true);
                } else {
                    settings_window::open(hwnd, point);
                }
            }
            Action::ReturnToSettings => settings_window::finish_login(hwnd, true),
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
                let height = ui::layout::height_for_rows(rows, scale, single_line_mode())
                    .min(work.3 - work.1);
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
            Action::SetFontFamily(name) => {
                if !name.is_empty() && !theme::font_available(&name) {
                    continue;
                }
                let detail = State::with(|s| {
                    s.app.config.font_family = name;
                    s.fonts = Fonts::load(s.scale, &s.app.config.font_family);
                    s.app.save();
                    s.detail.as_ref().map(|p| p.hwnd)
                }).flatten();
                settings_window::reload_fonts();
                invalidate(hwnd);
                if let Some(detail) = detail { invalidate(detail); }
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
            Action::SetSingleLine(on) => {
                State::with(|s| s.app.config.single_line = on);
                // Same row count, shorter cards: refit the window to whole
                // cards of the new height, keeping it on screen.
                let scale = window_scale(hwnd);
                let work = work_area_for(hwnd);
                let height =
                    ui::layout::height_for_rows(config_rows(hwnd), scale, on).min(work.3 - work.1);
                let mut rc = RECT::default();
                unsafe {
                    let _ = GetWindowRect(hwnd, &mut rc);
                }
                let placed = config::clamp_to_virtual_screen(
                    WindowState {
                        x: rc.left,
                        y: rc.top,
                        width: rc.right - rc.left,
                        height,
                    },
                    work,
                    scale,
                    on,
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
                save_window_state(hwnd);
                invalidate(hwnd);
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
            Action::CopySelection => {
                let text = State::with(|s| {
                    s.app
                        .login
                        .as_ref()
                        .and_then(|form| form.selected_text())
                })
                .flatten();
                if let Some(text) = text {
                    set_clipboard_text(&text);
                }
            }
            Action::CutSelection => {
                let cut = State::with(|s| {
                    s.app
                        .login
                        .as_mut()
                        .and_then(|form| form.take_selection())
                })
                .flatten();
                match cut {
                    Some(text) => set_clipboard_text(&text),
                    // Nothing selected: Ctrl+X keeps its old meaning of
                    // "replace this value".
                    None => actions.push(Action::ClearField),
                }
                invalidate(hwnd);
            }
            Action::SetActivatable(enabled) => {
                apply_activation(hwnd, enabled);
                State::with(|s| s.activatable = enabled);
            }
            Action::RemoveAccount(id) => {
                State::with(|s| {
                    s.app.config.remove_account(&id);
                    s.app
                        .config
                        .hidden_channels
                        .retain(|entry| entry.account_id != id);
                    s.app.issues.retain(|issue| issue.id != id);
                    s.app.open_accounts.retain(|account| account != &id);
                    s.app.accounts = s.app.config.accounts.clone();
                    let mut stored = s.app.stored.clone();
                    stored.forget_account(&id);
                    s.app.stored = stored;
                    s.app.config.store(&s.app.stored.clone());
                    s.app.save();
                    send_targets(s);
                    // Deleting the last account leaves nothing to poll, so the
                    // form comes back up for a new one.
                    if s.app.accounts.is_empty() {
                        s.app.open_login(Some("已删除账号,请添加一个".into()), LoginTarget::Add);
                    } else {
                        s.app.status = Some(("已删除账号".into(), false));
                    }
                });
                sync_activation(&mut actions);
                invalidate(hwnd);
            }
            Action::ShowDetail(index, point) => open_detail(hwnd, index, point),
            Action::ToggleDetail => {
                let popup = State::with(|s| {
                    let open = s.detail.as_mut()?;
                    open.depth = match open.depth {
                        detail::Depth::Compact => detail::Depth::Full,
                        detail::Depth::Full => detail::Depth::Compact,
                    };
                    rebuild_doc(open);
                    // The two documents have nothing in common position-wise.
                    open.scroll = 0.0;
                    Some((open.hwnd, open.depth))
                })
                .flatten();
                if let Some((hwnd, depth)) = popup {
                    fit_detail_height(hwnd, window_scale(hwnd), depth);
                    invalidate(hwnd);
                }
            }
            Action::CloseWindow => unsafe {
                let _ = DestroyWindow(hwnd);
            },
            Action::CopyDetail => copy_detail_text(),
            Action::ClearCredentials => {
                config::clear_stored();
                // The accounts keep their names and endpoints — only the
                // secrets go — so the menu still lists them and each can be
                // signed in again with a fresh token.
                State::with(|s| {
                    s.app.stored = Stored::default();
                    send_targets(s);
                    let target = match s.app.accounts.first() {
                        Some(account) => LoginTarget::Account(account.id.clone()),
                        None => LoginTarget::Add,
                    };
                    s.app.open_login(Some("已清除本机保存的凭据".into()), target);
                });
                sync_activation(&mut actions);
                invalidate(hwnd);
            }
        }
    }
    settings_window::redraw();
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
        // Account editing temporarily expands a compact panel. Persist its
        // original geometry rather than the form's working size.
        if let Some(previous) = &s.settings_login {
            let original = previous.geometry;
            rc = RECT {
                left: original.x,
                top: original.y,
                right: original.x + original.width,
                bottom: original.y + original.height,
            };
        }
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

/// Render a frame into an off-screen bitmap and present it with a single
/// `BitBlt`. Centralises the double-buffer scaffolding shared by `paint` and
/// `paint_detail`, and bails out cleanly if either GDI allocation fails
/// (design issue #14) so a failed `CreateCompatibleDC`/`CreateCompatibleBitmap`
/// can never feed a null handle into `SelectObject`/`BitBlt`. Returns whatever
/// `draw` produced, or `None` if the buffer could not be set up.
fn with_double_buffer<F, R>(hdc: HDC, rc: RECT, draw: F) -> Option<R>
where
    F: FnOnce(Painter) -> R,
{
    unsafe {
        let mem = CreateCompatibleDC(Some(hdc));
        if mem.is_invalid() {
            return None;
        }
        let bitmap = CreateCompatibleBitmap(hdc, rc.right.max(1), rc.bottom.max(1));
        if bitmap.is_invalid() {
            let _ = DeleteDC(mem);
            return None;
        }
        let previous = SelectObject(mem, HGDIOBJ(bitmap.0));

        let brush = CreateSolidBrush(COLORREF(theme::BG & 0x00FF_FFFF));
        FillRect(mem, &rc, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));

        let result = Painter::new(mem).map(draw);

        let _ = BitBlt(hdc, 0, 0, rc.right, rc.bottom, Some(mem), 0, 0, SRCCOPY);
        SelectObject(mem, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem);
        result
    }
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
        )
        .with_single_line(single_line_mode());

        // Draw the whole frame into an off-screen bitmap and present it with one
        // BitBlt. Painting the window DC directly lets each invalidate show a
        // partially drawn frame, which the user sees as flicker.
        let _ = with_double_buffer(hdc, rc, |painter| {
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
                        accounts: &s.app.accounts,
                        pinned: s.app.config.pin_position,
                        filter: s.app.filter,
                        hidden_fields: &s.app.config.hidden_fields,
                    };
                    panel::draw(&painter, &s.fonts, m, &view);
                }
            });
        });

        let _ = EndPaint(hwnd, &ps);
    }
}

fn on_timer(hwnd: HWND, id: usize, actions: &mut Vec<Action>) {
    match id {
        TIMER_POLL => {
            pump_worker(actions);
            let done =
                State::with(|s| s.settings_login.is_some() && !s.app.is_login()).unwrap_or(false);
            if done {
                settings_window::finish_login(hwnd, false);
            }
        }
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
    let mut popup: Option<HWND> = None;
    let changed = State::with(|s| {
        let updates = s.worker.drain();
        if updates.is_empty() {
            return false;
        }
        // Detail answers belong to the popup, not the list; everything else is
        // list state.
        let mut rest = Vec::with_capacity(updates.len());
        for update in updates {
            match update {
                Update::Detail { id, result } => apply_detail(s, &id, result),
                other => rest.push(other),
            }
        }
        s.app.apply_updates(rest);
        popup = s.detail.as_ref().map(|p| p.hwnd);
        true
    });
    if changed == Some(true) {
        actions.push(Action::Redraw);
        // The popup is a separate window: `Action::Redraw` only reaches the one
        // that raised it.
        if let Some(hwnd) = popup {
            invalidate(hwnd);
        }
        sync_activation(actions);
    }
}

/// Store one request's executions in the open popup. A reply for a request the
/// user has already clicked away from is dropped rather than painted over the
/// newer content.
fn apply_detail(
    s: &mut State,
    id: &str,
    result: Result<Vec<model::ExecutionDetail>, client::ApiError>,
) {
    let Some(open) = s.detail.as_mut() else {
        return;
    };
    if open.row.id != id {
        return;
    }
    match result {
        Ok(executions) => {
            open.executions = Some(executions);
            open.error = None;
        }
        Err(err) => {
            open.executions = None;
            open.error = Some(err.message());
        }
    }
    rebuild_doc(open);
    open.scroll = 0.0;
}

/// Re-render the document for the current depth. Cheap — it is string assembly
/// — so it runs whenever the attempts arrive or the depth is toggled.
fn rebuild_doc(open: &mut DetailPopup) {
    open.doc = open
        .executions
        .as_ref()
        .map(|executions| detail::Doc::build(&open.row, executions, now_unix(), open.depth));
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
    if enabled && !settings_window::is_visible() {
        unsafe {
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(Some(hwnd));
        }
    }
}

fn on_mouse_move(hwnd: HWND, wp: WPARAM, lp: LPARAM, actions: &mut Vec<Action>) {
    let x = (lp.0 & 0xFFFF) as i16 as f32;
    let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32;
    let m = metrics_for(hwnd);

    // A drag inside the sign-in form extends its selection, the way dragging
    // over text does anywhere else.
    let dragging = State::with(|s| s.app.is_login() && s.login_drag).unwrap_or(false);
    if dragging && wp.0 & MK_LBUTTON != 0 {
        let hdc = unsafe { GetDC(Some(hwnd)) };
        if !hdc.is_invalid() {
            let painter = Painter::new(hdc);
            let changed = State::with(|s| {
                let fonts = &s.fonts;
                let Some(form) = s.app.login.as_mut() else {
                    return false;
                };
                painter
                    .as_ref()
                    .is_some_and(|p| login::place_caret(p, fonts, form, m, x, y, false))
            });
            drop(painter);
            unsafe {
                let _ = ReleaseDC(Some(hwnd), hdc);
            }
            if changed == Some(true) {
                actions.push(Action::Redraw);
            }
        }
        return;
    }

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

    if State::with(|s| !s.app.is_login()).unwrap_or(false)
        && panel::settings_button(m).contains(x, y)
    {
        actions.push(Action::ShowSettings(popup::click_point(hwnd, lp)));
        return;
    }

    let mut redraw = false;

    // A GDI+ session to measure text with, so the caret can be placed at the
    // character under the pointer. One DC and one graphics object per click.
    let hdc = unsafe { GetDC(Some(hwnd)) };
    let painter = (!hdc.is_invalid())
        .then(|| Painter::new(hdc))
        .flatten();

    State::with(|s| {
        if s.app.is_login() {
            let action = s.app.login.as_ref().and_then(|f| login::hit(f, m, x, y));
            redraw = true;
            match action {
                Some(LoginAction::Back) => actions.push(Action::ReturnToSettings),
                Some(LoginAction::Submit) => actions.push(Action::SignIn),
                Some(LoginAction::CycleMode) => {
                    if let Some(f) = s.app.login.as_mut() {
                        // Saving a password belongs to the hidden password
                        // method, so the selector only walks the two modes
                        // that mean something here. A config left in password
                        // mode falls back to not storing anything.
                        let next = match f.mode {
                            CredentialMode::None => CredentialMode::Token,
                            _ => CredentialMode::None,
                        };
                        f.set_mode(next);
                    }
                }
                Some(LoginAction::SetMethod(method)) => {
                    if let Some(f) = s.app.login.as_mut() {
                        f.method = method;
                        f.focus_field(match method {
                            Method::Password => Field::Email,
                            Method::Token => Field::Token,
                        });
                        f.error = None;
                    }
                }
                Some(LoginAction::Focus(field)) => {
                    // The press both focuses the field and drops the caret
                    // where it landed; a drag that follows extends the
                    // selection from there.
                    if let Some(f) = s.app.login.as_mut() {
                        if let Some(p) = painter.as_ref() {
                            login::place_caret(p, &s.fonts, f, m, x, y, true);
                        } else {
                            f.focus_field(field);
                        }
                    }
                    // The button is down on a field: whatever moves next is a
                    // drag, until it is released.
                    s.login_drag = true;
                }
                None => {}
            }
            return;
        }

        // Filter chips take the press; card selection only happens on the list
        // proper. Pinned mode is display-only: a click must not select a
        // card or switch filters, only the right-click menu opens the detail
        // page.
        if let Some(f) = panel::hit_filter(m, &list_view(s), x, y) {
            s.app.click_filter(f);
            s.app.scroll = 0.0;
            redraw = true;
        } else if !s.app.config.pin_position
            && let Some(index) = panel::hit_row(m, &list_view(s), x, y)
        {
            s.app.selected = Some(index);
        }
    });

    drop(painter);
    if !hdc.is_invalid() {
        unsafe {
            let _ = ReleaseDC(Some(hwnd), hdc);
        }
    }

    if redraw {
        actions.push(Action::Redraw);
    }
    let _ = hwnd;
}

fn on_left_up(hwnd: HWND, lp: LPARAM, actions: &mut Vec<Action>) {
    let x = (lp.0 & 0xFFFF) as i16 as f32;
    let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32;
    let m = metrics_for(hwnd);

    // End a drag on the sign-in form: a press that never moved leaves a
    // zero-width anchor behind, which has to go.
    let ended = State::with(|s| {
        if !s.login_drag {
            return false;
        }
        s.login_drag = false;
        if let Some(form) = s.app.login.as_mut() {
            form.collapse_selection();
        }
        true
    });
    if ended == Some(true) {
        actions.push(Action::Redraw);
    }

    // Only act if the pointer is still over the card it went down on, so
    // dragging off cancels the action; pinned mode is display-only. Only a card
    // with something to explain can get here — `hit_test` leaves every other
    // one as the window's drag handle.
    let index = State::with(|s| {
        if s.app.is_login() || s.app.config.pin_position {
            return None;
        }
        let view = list_view(s);
        let index = panel::hit_error_row(m, &view, x, y)?;
        (s.app.selected == Some(index)).then_some(index)
    })
    .flatten();

    if let Some(index) = index {
        actions.push(Action::ShowDetail(index, popup::click_point(hwnd, lp)));
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
        accounts: &s.app.accounts,
        pinned: s.app.config.pin_position,
        filter: s.app.filter,
        hidden_fields: &s.app.config.hidden_fields,
    }
}

/// The worker's poll list: one target per saved account, with whatever token
/// the encrypted blob holds for it.
fn targets_from(config: &Config, stored: &Stored) -> Vec<Target> {
    config
        .accounts
        .iter()
        .map(|account| Target {
            id: account.id.clone(),
            name: account.name.clone(),
            endpoint: account.endpoint.clone(),
            project_id: account.project_id.clone(),
            token: stored
                .token_for(&account.id)
                .filter(|token| !token.is_empty()),
        })
        .collect()
}

/// Hand the worker the current set of accounts and tokens.
fn send_targets(s: &mut State) {
    let targets = targets_from(&s.app.config, &s.app.stored);
    s.worker.send(Command::SetTargets(targets));
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

    // Clipboard and selection shortcuts. Handled here because this window
    // draws its own controls: there is no edit control to implement them.
    if ctrl_down() && State::with(|s| s.app.is_login() == true).unwrap_or(false) {
        // A plain 'v' arrives as WM_CHAR too, so swallow that one; see below.
        match vk {
            v if v == VK_V.0 => {
                actions.push(Action::Paste);
                return;
            }
            v if v == VK_A.0 => {
                State::with(|s| {
                    if let Some(form) = s.app.login.as_mut() {
                        form.select_all();
                    }
                });
                actions.push(Action::Redraw);
                return;
            }
            v if v == VK_C.0 => {
                actions.push(Action::CopySelection);
                return;
            }
            v if v == VK_X.0 => {
                actions.push(Action::CutSelection);
                return;
            }
            _ => {}
        }
    }

    // Arrow keys move the caret; with Shift they drag the selection anchor,
    // which is what makes the fields behave like edit controls.
    let extend = shift_down();

    let mut quit = false;
    let mut submit = false;
    let handled = State::with(|s| {
        if s.app.is_login() {
            let Some(form) = s.app.login.as_mut() else {
                return false;
            };
            match vk {
                v if v == VK_BACK.0 => form.backspace(),
                v if v == VK_DELETE.0 => form.delete_forward(),
                v if v == VK_LEFT.0 => form.move_caret(-1, extend),
                v if v == VK_RIGHT.0 => form.move_caret(1, extend),
                v if v == VK_HOME.0 => form.caret_to_edge(false, extend),
                v if v == VK_END.0 => form.caret_to_edge(true, extend),
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
        if State::with(|s| s.settings_login.is_some()).unwrap_or(false) {
            actions.push(Action::ReturnToSettings);
        } else {
            actions.push(Action::Quit);
        }
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
        // The mode chosen in the form is what governs persistence, so it has to
        // reach the config before `store` consults it.
        s.app.config.credential_mode = form.mode;

        let submission = match form.method {
            Method::Password => Submission::Password(form.credentials()),
            Method::Token => Submission::Token(form.trimmed_token()),
        };
        Some(submission)
    });

    let Some(submission) = prepared.flatten() else {
        invalidate(hwnd);
        return;
    };

    match submission {
        Submission::Password(creds) => {
            // Remember what to persist once a token comes back; the password is
            // only written when the mode allows it.
            State::with(|s| {
                let Some((id, _name)) = upsert_form_account(s) else {
                    return;
                };
                s.app.stored.email = creds.email.clone();
                s.app.stored.password = Some(creds.password.clone());
                s.app.save();
                // The account has to exist in the poll list before the answer
                // lands, or the issued token would have nowhere to go.
                send_targets(s);
                s.worker.send(Command::SignIn {
                    account: id,
                    creds,
                });
            });
        }
        Submission::Token(token) => {
            // A pasted token is used directly: no sign-in round trip, and the
            // password never enters the process.
            State::with(|s| {
                let Some((id, name)) = upsert_form_account(s) else {
                    return;
                };
                let mut stored = s.app.stored.clone();
                // Keep live credentials in memory; Config::store alone decides
                // what is allowed onto disk in “do not save” mode.
                stored.set_token(&id, &token);
                s.app.config.store(&stored);
                s.app.stored = stored;
                s.app.view = View::List;
                s.app.status = Some((format!("账号 {name} 已登录"), false));
                s.app.save();
                send_targets(s);
            });
        }
    }
    let done = State::with(|s| s.settings_login.is_some() && !s.app.is_login()).unwrap_or(false);
    if done {
        settings_window::finish_login(hwnd, false);
    }
    let mut actions = Vec::new();
    sync_activation(&mut actions);
    run_actions(hwnd, actions);
    invalidate(hwnd);
}

/// Create or update the account the sign-in form addresses and return its id
/// and name. The panel's own endpoint and credential mode follow the form, so
/// the next "add account" starts where this one left off.
fn upsert_form_account(s: &mut State) -> Option<(String, String)> {
    let form = s.app.login.as_ref()?;
    let name = form.account_name.trim().to_string();
    let endpoint = form.endpoint.trim().trim_end_matches('/').to_string();
    let mode = form.mode;
    let id = match form.editing.clone() {
        Some(id) => id,
        None => s.app.config.fresh_account_id(),
    };
    // An existing account keeps the project it resolved last time; a new one
    // starts from the panel's own, which is the common single-project case.
    let project_id = s
        .app
        .config
        .account(&id)
        .map(|a| a.project_id.clone())
        .unwrap_or_else(|| s.app.config.project_id.clone());
    s.app.config.upsert_account(Account {
        id: id.clone(),
        name: name.clone(),
        endpoint: endpoint.clone(),
        project_id,
    });
    s.app.config.endpoint = endpoint;
    s.app.config.credential_mode = mode;
    s.app.accounts = s.app.config.accounts.clone();
    Some((id, name))
}

fn on_command(wp: WPARAM, actions: &mut Vec<Action>) {
    let id = (wp.0 & 0xFFFF) as usize;
    State::with(|s| match id {
        MENU_REFRESH => s.worker.send(Command::RefreshNow),
        MENU_TOPMOST => {
            let on = !s.app.config.always_on_top;
            s.app.config.always_on_top = on;
            if on {
                s.app.config.always_on_bottom = false;
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
        MENU_SINGLE_LINE => {
            let on = !s.app.config.single_line;
            actions.push(Action::SetSingleLine(on));
        }
        MENU_OPEN => {
            let url = format!(
                "{}/project/requests",
                s.app.config.endpoint.trim_end_matches('/')
            );
            actions.push(Action::OpenUrl(url));
        }
        MENU_SETTINGS => actions.push(Action::ShowSettings(popup::cursor())),
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
        add("设置…", MENU_SETTINGS, false, true);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
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
        // One-line card layout: same row count, shorter cards, shorter panel.
        let single = State::with(|s| s.app.config.single_line).unwrap_or(false);
        add("单行模式", MENU_SINGLE_LINE, single, true);

        add("刷新", MENU_REFRESH, false, true);
        add("打开请求页", MENU_OPEN, false, true);

        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        add("固定窗口位置", MENU_PIN_POSITION, pinned, true);
        let top = State::with(|s| s.app.config.always_on_top).unwrap_or(false);
        add("置顶", MENU_TOPMOST, top, true);
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

// --- error-detail popup ------------------------------------------------------

/// Metrics for the popup's client area. The scale is the panel's rather than
/// the popup's own monitor: the popup draws with the panel's fonts, so a
/// different scale would size the boxes for text that is not being drawn.
fn detail_metrics(hwnd: HWND) -> detail::Popup {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    let scale = State::with(|s| s.scale).unwrap_or_else(|| window_scale(hwnd));
    detail::Popup::new(
        (rc.right - rc.left) as f32,
        (rc.bottom - rc.top) as f32,
        scale,
    )
}

/// Open at the clicked card, keeping the whole popup inside that monitor.
fn detail_geometry(point: POINT, scale: f32, depth: detail::Depth) -> WindowState {
    let width = ui::layout::device_px(detail::POPUP_W as i32, scale);
    let height = ui::layout::device_px(detail::default_height(depth) as i32, scale);
    popup::at_point(point, width, height)
}

/// Give the popup the height its form is authored for, keeping its top-left
/// corner and staying on screen. Called when the form changes, so the full
/// document is not read through a letterbox and the compact one is not shown in
/// a half-empty window.
fn fit_detail_height(hwnd: HWND, scale: f32, depth: detail::Depth) {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rc);
    }
    let work = work_area_for(hwnd);
    let height =
        ui::layout::device_px(detail::default_height(depth) as i32, scale).min(work.3 - work.1);
    let placed = config::clamp_to_virtual_screen(
        WindowState {
            x: rc.left,
            y: rc.top,
            width: rc.right - rc.left,
            height,
        },
        work,
        scale,
        false,
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
}

/// Create the popup window. It is owned by the panel, so it always stays above
/// it and goes away with it.
fn create_detail_window(panel: HWND, geom: WindowState) -> Option<HWND> {
    let instance = unsafe { GetModuleHandleW(None) }.ok()?;
    let class: Vec<u16> = DETAIL_CLASS
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let title: Vec<u16> = "请求详情"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut ex_style = WS_EX_TOOLWINDOW;
    if State::with(|s| s.app.config.always_on_top).unwrap_or(false) {
        ex_style |= WS_EX_TOPMOST;
    }
    let hwnd = unsafe {
        CreateWindowExW(
            ex_style,
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_POPUP,
            geom.x,
            geom.y,
            geom.width,
            geom.height,
            Some(panel),
            None,
            Some(instance.into()),
            None,
        )
    }
    .ok()?;
    apply_window_chrome(hwnd);
    Some(hwnd)
}

/// The form the popup opens in: whichever is already on screen, so a reader who
/// asked for the full document keeps it while walking through several failures.
/// A fresh popup starts compact.
fn current_depth() -> detail::Depth {
    State::with(|s| s.detail.as_ref().map(|open| open.depth))
        .flatten()
        .unwrap_or(detail::Depth::Compact)
}

/// Open — or refocus — the detail popup for a row of the list.
fn open_detail(panel: HWND, index: usize, point: POINT) {
    enum Next {
        /// This request is already on screen; just raise the window.
        Focus(HWND),
        /// Fetch the executions; `None` means the window still has to be built.
        Load(Option<HWND>, Box<model::Row>, String, f32),
    }

    // Read before borrowing the state below: `current_depth` uses the same cell.
    let depth = current_depth();
    let prepared = State::with(|s| {
        let row = s.app.rows.get(index)?.clone();
        if let Some(open) = s.detail.as_ref().filter(|p| p.row.id == row.id) {
            // Clicking the same card again must not throw the answer away.
            return Some(Next::Focus(open.hwnd));
        }
        // The console link belongs to the server the request came from, which
        // is not necessarily the panel's own endpoint once accounts differ.
        let endpoint = s
            .app
            .config
            .account(&row.account_id)
            .map(|a| a.endpoint.clone())
            .unwrap_or_else(|| s.app.config.endpoint.clone());
        let url = model::request_url(&endpoint, &row.id);
        let existing = s.detail.as_ref().map(|p| p.hwnd);
        Some(Next::Load(existing, Box::new(row), url, s.scale))
    })
    .flatten();

    let Some(next) = prepared else {
        return;
    };
    match next {
        Next::Focus(hwnd) => unsafe {
            place_detail_at(hwnd, point);
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        },
        Next::Load(existing, row, url, scale) => {
            let hwnd = match existing {
                Some(hwnd) => hwnd,
                None => match create_detail_window(
                    panel,
                    detail_geometry(point, scale, current_depth()),
                ) {
                    Some(hwnd) => hwnd,
                    None => return,
                },
            };
            let id = row.id.clone();
            let account = row.account_id.clone();
            State::with(|s| {
                s.detail = Some(DetailPopup {
                    hwnd,
                    row: *row,
                    url,
                    executions: None,
                    doc: None,
                    error: None,
                    depth,
                    scroll: 0.0,
                    content_h: 0.0,
                    hover: None,
                });
                // The request has to be fetched from the account that listed
                // it: another account on another server cannot see it.
                s.worker.send(Command::FetchDetail {
                    account,
                    id,
                });
            });
            if existing.is_some() {
                place_detail_at(hwnd, point);
            }
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetForegroundWindow(hwnd);
                let _ = SetFocus(Some(hwnd));
            }
            invalidate(hwnd);
        }
    }
}

fn place_detail_at(hwnd: HWND, point: POINT) {
    let mut rc = RECT::default();
    unsafe { let _ = GetWindowRect(hwnd, &mut rc); }
    let placed = popup::at_point(point, rc.right - rc.left, rc.bottom - rc.top);
    unsafe {
        let _ = SetWindowPos(hwnd, None, placed.x, placed.y, placed.width, placed.height,
            SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// Put the whole detail document on the clipboard.
fn copy_detail_text() {
    let text = State::with(|s| {
        s.detail
            .as_ref()
            .and_then(|p| p.doc.as_ref())
            .map(detail::plain_text)
    })
    .flatten();
    if let Some(text) = text {
        set_clipboard_text(&text);
    }
}

unsafe extern "system" fn detail_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let mut actions: Vec<Action> = Vec::new();

    match msg {
        WM_ACTIVATE => {
            if wp.0 as u32 & 0xFFFF == WA_INACTIVE {
                popup::dismiss_later(hwnd);
            }
            return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
        }
        popup::WM_DISMISS => popup::dismiss_if_inactive(hwnd),
        WM_PAINT => paint_detail(hwnd),
        WM_ERASEBKGND => {}
        WM_NCHITTEST => {
            let mut rc = RECT::default();
            unsafe {
                let _ = GetWindowRect(hwnd, &mut rc);
            }
            let x = (lp.0 & 0xFFFF) as i16 as f32 - rc.left as f32;
            let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32 - rc.top as f32;
            let popup = detail_metrics(hwnd);
            if detail::hit_button(&popup, x, y).is_some() {
                return LRESULT(HTCLIENT as isize);
            }
            let m = Metrics::new(popup.width, popup.height, popup.scale);
            if let Some(edge) = edge_at(m, x, y) {
                return LRESULT(edge.hit_test() as isize);
            }
            // Everything else is the window's drag handle.
            return LRESULT(HTCAPTION as isize);
        }
        WM_SETCURSOR => {
            let mut cursor = POINT::default();
            let mut rc = RECT::default();
            unsafe {
                let _ = GetCursorPos(&mut cursor);
                let _ = GetWindowRect(hwnd, &mut rc);
            }
            let popup = detail_metrics(hwnd);
            let x = (cursor.x - rc.left) as f32;
            let y = (cursor.y - rc.top) as f32;
            let m = Metrics::new(popup.width, popup.height, popup.scale);
            let shape = match edge_at(m, x, y) {
                Some(edge) => edge.cursor_id(),
                None if detail::hit_button(&popup, x, y).is_some() => IDC_HAND,
                None => IDC_ARROW,
            };
            if let Ok(handle) = unsafe { LoadCursorW(None, shape) } {
                unsafe {
                    SetCursor(Some(handle));
                }
                return LRESULT(1);
            }
        }
        WM_MOUSEMOVE => detail_mouse_move(hwnd, lp, &mut actions),
        WM_MOUSELEAVE => {
            let changed =
                State::with(|s| s.detail.as_mut().is_some_and(|p| p.hover.take().is_some()));
            if changed == Some(true) {
                actions.push(Action::Redraw);
            }
        }
        WM_LBUTTONUP => {
            let x = (lp.0 & 0xFFFF) as i16 as f32;
            let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32;
            let popup = detail_metrics(hwnd);
            match detail::hit_button(&popup, x, y) {
                Some(Button::Close) => actions.push(Action::CloseWindow),
                Some(Button::Toggle) => actions.push(Action::ToggleDetail),
                Some(Button::Copy) => actions.push(Action::CopyDetail),
                Some(Button::Open) => {
                    let url = State::with(|s| s.detail.as_ref().map(|p| p.url.clone())).flatten();
                    if let Some(url) = url {
                        actions.push(Action::OpenUrl(url));
                    }
                }
                None => {}
            }
        }
        WM_MOUSEWHEEL => detail_wheel(hwnd, wp, &mut actions),
        WM_KEYDOWN => detail_key(hwnd, wp, &mut actions),
        WM_GETMINMAXINFO => {
            let scale = window_scale(hwnd);
            let info = unsafe { &mut *(lp.0 as *mut MINMAXINFO) };
            info.ptMinTrackSize = POINT {
                x: (detail::MIN_W * scale) as i32,
                y: (detail::MIN_H * scale) as i32,
            };
            return LRESULT(0);
        }
        WM_SIZING => {
            if lp.0 != 0 {
                let rect = unsafe { &mut *(lp.0 as *mut RECT) };
                let proposed = *rect;
                let placed = clamp_window_rect(proposed, work_area_of_rect(&proposed), hwnd);
                *rect = RECT {
                    left: placed.x,
                    top: placed.y,
                    right: placed.x + placed.width,
                    bottom: placed.y + placed.height,
                };
            }
        }
        WM_MOVING => {
            if lp.0 != 0 {
                let mut cursor = POINT::default();
                unsafe {
                    let _ = GetCursorPos(&mut cursor);
                }
                let rect = unsafe { &mut *(lp.0 as *mut RECT) };
                let placed = clamp_window_rect(*rect, work_area_at(cursor.x, cursor.y), hwnd);
                *rect = RECT {
                    left: placed.x,
                    top: placed.y,
                    right: placed.x + placed.width,
                    bottom: placed.y + placed.height,
                };
            }
        }
        WM_WINDOWPOSCHANGING => {
            let window_pos = lp.0 as *mut WINDOWPOS;
            if !window_pos.is_null() {
                unsafe {
                    constrain_to_work_area(hwnd, &mut *window_pos);
                }
            }
        }
        WM_SIZE => actions.push(Action::Redraw),
        WM_DPICHANGED => {
            // The popup keeps the panel's scale, but it still has to take the
            // rectangle Windows suggests for the monitor it landed on.
            if lp.0 != 0 {
                let suggested = unsafe { &*(lp.0 as *const RECT) };
                unsafe {
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        suggested.left,
                        suggested.top,
                        suggested.right - suggested.left,
                        suggested.bottom - suggested.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
            actions.push(Action::Redraw);
        }
        WM_DISPLAYCHANGE => {
            fit_window_to_work_area(hwnd);
            actions.push(Action::Redraw);
        }
        WM_CLOSE => actions.push(Action::CloseWindow),
        WM_DESTROY => {
            State::with(|s| s.detail = None);
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }

    run_actions(hwnd, actions);
    LRESULT(0)
}

fn detail_mouse_move(hwnd: HWND, lp: LPARAM, actions: &mut Vec<Action>) {
    let x = (lp.0 & 0xFFFF) as i16 as f32;
    let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32;
    let popup = detail_metrics(hwnd);
    let hover = detail::hit_button(&popup, x, y);

    let changed = State::with(|s| {
        let Some(open) = s.detail.as_mut() else {
            return false;
        };
        if open.hover == hover {
            return false;
        }
        open.hover = hover;
        true
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
    if changed == Some(true) {
        actions.push(Action::Redraw);
    }
}

fn detail_wheel(hwnd: HWND, wp: WPARAM, actions: &mut Vec<Action>) {
    let delta = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
    let m = detail_metrics(hwnd);
    let step = m.row_h() * 3.0;
    let view_h = m.content_view_h();

    let changed = State::with(|s| {
        let Some(open) = s.detail.as_mut() else {
            return false;
        };
        let max = (open.content_h - view_h).max(0.0);
        let next = (open.scroll - delta * step).clamp(0.0, max);
        if (next - open.scroll).abs() < 0.01 {
            return false;
        }
        open.scroll = next;
        true
    });
    if changed == Some(true) {
        actions.push(Action::Redraw);
    }
}

fn detail_key(hwnd: HWND, wp: WPARAM, actions: &mut Vec<Action>) {
    let vk = wp.0 as u16;
    if ctrl_down() && vk == VK_C.0 {
        actions.push(Action::CopyDetail);
        return;
    }
    if vk == VK_ESCAPE.0 {
        actions.push(Action::CloseWindow);
        return;
    }

    let m = detail_metrics(hwnd);
    let row = m.row_h();
    let page = (m.content_view_h() - row).max(row);
    let delta = match vk {
        v if v == VK_DOWN.0 => row,
        v if v == VK_UP.0 => -row,
        v if v == VK_NEXT.0 => page,
        v if v == VK_PRIOR.0 => -page,
        v if v == VK_HOME.0 => f32::MIN,
        v if v == VK_END.0 => f32::MAX,
        _ => return,
    };
    let view_h = m.content_view_h();

    let changed = State::with(|s| {
        let Some(open) = s.detail.as_mut() else {
            return false;
        };
        let max = (open.content_h - view_h).max(0.0);
        let next = (open.scroll + delta).clamp(0.0, max);
        if (next - open.scroll).abs() < 0.01 {
            return false;
        }
        open.scroll = next;
        true
    });
    if changed == Some(true) {
        actions.push(Action::Redraw);
    }
}

fn paint_detail(hwnd: HWND) {
    let popup = detail_metrics(hwnd);
    let view_h = popup.content_view_h();
    let content_h;

    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);

        // Clamp with the height the previous frame measured, so a document that
        // just got shorter cannot paint past its end while this frame renders.
        State::with(|s| {
            if let Some(open) = s.detail.as_mut() {
                open.scroll = open.scroll.clamp(0.0, (open.content_h - view_h).max(0.0));
            }
        });

        content_h = with_double_buffer(hdc, rc, |painter| {
            State::with(|s| {
                let Some(open) = s.detail.as_ref() else {
                    return 0.0;
                };
                let view = detail::View {
                    row: &open.row,
                    doc: open.doc.as_ref(),
                    error: open.error.as_deref(),
                    depth: open.depth,
                    scroll: open.scroll,
                    hover: open.hover,
                };
                detail::draw(&painter, &s.fonts, &popup, &view)
            })
            .unwrap_or(0.0)
        })
        .unwrap_or(0.0);

        let _ = EndPaint(hwnd, &ps);
    }

    State::with(|s| {
        if let Some(open) = s.detail.as_mut() {
            open.content_h = content_h;
        }
    });
}
