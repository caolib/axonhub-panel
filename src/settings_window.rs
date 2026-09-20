//! Modeless settings window. Win32 effects always run after releasing panel state.

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, DeleteObject, EndPaint, HDC, HGDIOBJ, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_DOWN, VK_END, VK_ESCAPE,
    VK_F, VK_HOME, VK_NEXT, VK_PRIOR, VK_RETURN, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

use crate::app::{LoginTarget, View};
use crate::config::WindowState;
use crate::theme::Fonts;
use crate::ui::font_search::{self, SearchBox};
use crate::ui::login::LoginForm;
use crate::ui::settings::{self, Action, Layout};
use crate::{Action as PanelAction, State as PanelState, invalidate};

pub struct Popup {
    pub hwnd: HWND,
    owner: HWND,
    fonts: Fonts,
    scale: f32,
    ui: settings::State,
    anchor: POINT,
    search: Option<SearchBox>,
}

/// Restore the panel after editing an account, including a previously open form.
pub struct LoginReturn {
    pub geometry: WindowState,
    view: View,
    form: Option<LoginForm>,
    anchor: POINT,
}

fn scale_for(hwnd: HWND) -> f32 {
    crate::monitor_density(hwnd)
        .unwrap_or_else(|| crate::windows_dpi_scale(hwnd))
        .clamp(0.5, 4.0)
}

pub fn open(owner: HWND, point: POINT) {
    let existing = PanelState::with(|s| s.settings.as_ref().map(|p| p.hwnd)).flatten();
    if let Some(hwnd) = existing {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetWindowRect(hwnd, &mut rc);
        }
        let placed = crate::popup::at_point(point, rc.right - rc.left, rc.bottom - rc.top);
        PanelState::with(|s| {
            if let Some(p) = s.settings.as_mut() {
                p.anchor = point;
            }
        });
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
        focus(hwnd);
        invalidate(hwnd);
        return;
    }
    let scale = scale_for(owner);
    let width = (settings::WIDTH * scale).round() as i32;
    let height = (settings::HEIGHT * scale).round() as i32;
    let placed = crate::popup::at_point(point, width, height);
    let hwnd = unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("AHSettingsWindow"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("AHSettingsWindow"),
            w!("AH Panel · 设置"),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_CLIPCHILDREN,
            placed.x,
            placed.y,
            placed.width,
            placed.height,
            Some(owner),
            None,
            Some(instance.into()),
            None,
        ) else {
            return;
        };
        hwnd
    };
    let scale = scale_for(hwnd);
    let mut ui = settings::State {
        font_names: crate::font_catalog::installed(),
        ..Default::default()
    };
    ui.set_font_query(String::new());
    PanelState::with(|s| {
        s.settings = Some(Popup {
            hwnd,
            owner,
            scale,
            fonts: Fonts::load(scale, &s.app.config.font_family),
            ui,
            anchor: point,
            search: None,
        })
    });
    crate::apply_window_chrome(hwnd);
    focus(hwnd);
}

fn focus(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
    }
}

pub fn redraw() {
    if let Some(hwnd) = PanelState::with(|s| s.settings.as_ref().map(|p| p.hwnd)).flatten() {
        invalidate(hwnd);
    }
}

pub fn is_visible() -> bool {
    PanelState::with(|s| s.settings.as_ref().map(|p| p.hwnd))
        .flatten()
        .is_some_and(|hwnd| unsafe { IsWindowVisible(hwnd).as_bool() })
}

pub fn reload_fonts() {
    PanelState::with(|s| {
        if let Some(p) = s.settings.as_mut() {
            p.fonts = Fonts::load(p.scale, &s.app.config.font_family);
        }
    });
    redraw();
}

fn sync_search_box(hwnd: HWND) {
    let Some(layout) = layout(hwnd) else {
        return;
    };
    let state = PanelState::with(|s| {
        s.settings.as_ref().map(|p| {
            (
                p.search.as_ref().map(|search| search.hwnd),
                p.scale,
                p.ui.font_query.clone(),
            )
        })
    })
    .flatten();
    let Some((existing, scale, query)) = state else {
        return;
    };
    let Some(rect) = layout.search_rect() else {
        if let Some(child) = existing {
            unsafe {
                if GetFocus() == child {
                    let _ = SetFocus(Some(hwnd));
                }
                let _ = ShowWindow(child, SW_HIDE);
            }
        }
        return;
    };
    if existing.is_none() {
        let Some(search) = SearchBox::new(hwnd, scale, &query) else {
            return;
        };
        PanelState::with(|s| {
            if let Some(p) = s.settings.as_mut() {
                p.search = Some(search);
            }
        });
    }
    let ready = PanelState::with(|s| {
        let search = s.settings.as_mut()?.search.as_mut()?;
        let old = if (search.scale - scale).abs() > 0.01 {
            search.scale = scale;
            Some(std::mem::replace(
                &mut search.font,
                font_search::font(scale),
            ))
        } else {
            None
        };
        Some((search.hwnd, search.font, old))
    })
    .flatten();
    if let Some((child, font, old)) = ready {
        unsafe {
            if let Some(old) = old {
                SendMessageW(
                    child,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                );
                let _ = DeleteObject(HGDIOBJ(old.0));
            }
            let _ = SetWindowPos(
                child,
                None,
                (rect.x * scale).round() as i32,
                (rect.y * scale).round() as i32,
                (rect.w * scale).round() as i32,
                (rect.h * scale).round() as i32,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }
}

fn focus_control(hwnd: HWND, select_all: bool) {
    let search = PanelState::with(|s| {
        let p = s.settings.as_ref()?;
        (p.ui.focus == Some(Action::FontSearch))
            .then(|| p.search.as_ref().map(|s| s.hwnd))
            .flatten()
    })
    .flatten();
    font_search::focus(search.unwrap_or(hwnd), select_all && search.is_some());
}

fn layout(hwnd: HWND) -> Option<Layout> {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    PanelState::with(|s| {
        let popup = s.settings.as_ref()?;
        Some(Layout::new(
            &s.app,
            &popup.ui,
            rc.right as f32 / popup.scale,
            rc.bottom as f32 / popup.scale,
        ))
    })
    .flatten()
}

fn hit(hwnd: HWND, lp: LPARAM) -> Option<Action> {
    let layout = layout(hwnd)?;
    PanelState::with(|s| {
        let p = s.settings.as_ref()?;
        layout.hit(
            &p.ui,
            (lp.0 & 0xFFFF) as i16 as f32 / p.scale,
            ((lp.0 >> 16) & 0xFFFF) as i16 as f32 / p.scale,
        )
    })
    .flatten()
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ACTIVATE => {
            if wp.0 as u32 & 0xFFFF == WA_INACTIVE {
                crate::popup::dismiss_later(hwnd);
            }
            return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
        }
        crate::popup::WM_DISMISS => crate::popup::dismiss_if_inactive(hwnd),
        WM_COMMAND if wp.0 & 0xFFFF == font_search::ID => {
            if (wp.0 >> 16) as u32 == EN_CHANGE {
                let query = font_search::query(HWND(lp.0 as *mut _));
                PanelState::with(|s| {
                    if let Some(p) = s.settings.as_mut() {
                        p.ui.set_font_query(query);
                        p.ui.focus = Some(Action::FontSearch);
                    }
                });
                invalidate(hwnd);
            }
        }
        font_search::WM_SEARCH_FOCUS => {
            let focused = unsafe { GetFocus() };
            PanelState::with(|s| {
                if let Some(p) = s.settings.as_mut()
                    && p.ui.tab == settings::Tab::Fonts
                    && p.search
                        .as_ref()
                        .is_some_and(|search| search.hwnd == focused)
                {
                    p.ui.focus = Some(Action::FontSearch);
                }
            });
            invalidate(hwnd);
        }
        WM_CTLCOLOREDIT => {
            if let Some(color) = PanelState::with(|s| {
                let search = s.settings.as_ref()?.search.as_ref()?;
                (search.hwnd.0 as isize == lp.0).then(|| search.color_dc(HDC(wp.0 as *mut _)))
            })
            .flatten()
            {
                return color;
            }
            return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
        }
        WM_PAINT => paint(hwnd),
        WM_ERASEBKGND => return LRESULT(1),
        WM_MOUSEMOVE => {
            let hover = hit(hwnd, lp);
            let changed = PanelState::with(|s| {
                let p = s.settings.as_mut()?;
                let changed = p.ui.hover != hover;
                p.ui.hover = hover;
                Some(changed)
            })
            .flatten()
            .unwrap_or(false);
            unsafe {
                let mut tracking = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tracking);
            }
            if changed {
                invalidate(hwnd);
            }
        }
        crate::WM_MOUSELEAVE => {
            PanelState::with(|s| {
                if let Some(p) = s.settings.as_mut() {
                    p.ui.hover = None;
                    p.ui.pressed = None;
                }
            });
            invalidate(hwnd);
        }
        WM_LBUTTONDOWN => {
            unsafe {
                let _ = SetFocus(Some(hwnd));
            }
            let action = hit(hwnd, lp);
            PanelState::with(|s| {
                if let Some(p) = s.settings.as_mut() {
                    p.ui.focus = action.clone();
                    p.ui.pressed = action;
                }
            });
            invalidate(hwnd);
        }
        WM_LBUTTONUP => {
            let released = hit(hwnd, lp);
            let pressed =
                PanelState::with(|s| s.settings.as_mut().and_then(|p| p.ui.pressed.take()))
                    .flatten();
            if pressed == released
                && let Some(action) = released
            {
                dispatch(hwnd, action);
            }
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
            scroll(hwnd, -delta * 48.0);
        }
        WM_KEYDOWN => key(hwnd, wp.0 as u16),
        WM_GETMINMAXINFO => {
            let scale = scale_for(hwnd);
            let work = crate::work_area_for(hwnd);
            let info = unsafe { &mut *(lp.0 as *mut MINMAXINFO) };
            info.ptMinTrackSize.x = ((settings::MIN_WIDTH * scale) as i32).min(work.2 - work.0);
            info.ptMinTrackSize.y = ((settings::MIN_HEIGHT * scale) as i32).min(work.3 - work.1);
        }
        WM_WINDOWPOSCHANGING => {
            if lp.0 != 0 {
                let pos = unsafe { &mut *(lp.0 as *mut WINDOWPOS) };
                if !pos.flags.contains(SWP_NOMOVE) || !pos.flags.contains(SWP_NOSIZE) {
                    let mut rc = RECT::default();
                    unsafe {
                        let _ = GetWindowRect(hwnd, &mut rc);
                    }
                    let (x, y) = if pos.flags.contains(SWP_NOMOVE) {
                        (rc.left, rc.top)
                    } else {
                        (pos.x, pos.y)
                    };
                    let (w, h) = if pos.flags.contains(SWP_NOSIZE) {
                        (rc.right - rc.left, rc.bottom - rc.top)
                    } else {
                        (pos.cx, pos.cy)
                    };
                    let proposed = RECT {
                        left: x,
                        top: y,
                        right: x + w,
                        bottom: y + h,
                    };
                    let fitted = fit(proposed, crate::work_area_of_rect(&proposed));
                    pos.x = fitted.left;
                    pos.y = fitted.top;
                    pos.cx = fitted.right - fitted.left;
                    pos.cy = fitted.bottom - fitted.top;
                    pos.flags &= !(SWP_NOMOVE | SWP_NOSIZE);
                }
            }
        }
        WM_DPICHANGED | WM_MOVE | WM_DISPLAYCHANGE => sync_scale(hwnd),
        WM_SIZE => {
            scroll(hwnd, 0.0);
            sync_search_box(hwnd);
            invalidate(hwnd);
        }
        WM_CLOSE => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        WM_NCDESTROY => {
            PanelState::with(|s| s.settings = None);
            return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
    LRESULT(0)
}

fn fit(rect: RECT, work: (i32, i32, i32, i32)) -> RECT {
    let width = (rect.right - rect.left).max(1).min(work.2 - work.0);
    let height = (rect.bottom - rect.top).max(1).min(work.3 - work.1);
    let x = rect.left.clamp(work.0, work.2 - width);
    let y = rect.top.clamp(work.1, work.3 - height);
    RECT {
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
    }
}

fn sync_scale(hwnd: HWND) {
    let scale = scale_for(hwnd);
    let ratio = PanelState::with(|s| {
        let p = s.settings.as_mut()?;
        if (scale - p.scale).abs() < 0.01 {
            return None;
        }
        let ratio = scale / p.scale;
        p.scale = scale;
        p.fonts = Fonts::load(scale, &s.app.config.font_family);
        Some(ratio)
    })
    .flatten()
    .unwrap_or(1.0);
    let mut rect = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rect);
    }
    let proposed = RECT {
        right: rect.left + ((rect.right - rect.left) as f32 * ratio).round() as i32,
        bottom: rect.top + ((rect.bottom - rect.top) as f32 * ratio).round() as i32,
        ..rect
    };
    let fitted = fit(proposed, crate::work_area_for(hwnd));
    if fitted != rect {
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                fitted.left,
                fitted.top,
                fitted.right - fitted.left,
                fitted.bottom - fitted.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    sync_search_box(hwnd);
    invalidate(hwnd);
}

fn scroll(hwnd: HWND, delta: f32) {
    let Some(layout) = layout(hwnd) else {
        return;
    };
    PanelState::with(|s| {
        if let Some(p) = s.settings.as_mut() {
            p.ui.scroll = (p.ui.scroll + delta).clamp(0.0, layout.max_scroll());
            p.ui.hover = None;
            p.ui.pressed = None;
        }
    });
    invalidate(hwnd);
}

fn key(hwnd: HWND, key: u16) {
    let confirming =
        PanelState::with(|s| s.settings.as_ref().is_some_and(|p| p.ui.pending.is_some()))
            .unwrap_or(false);
    if key == VK_F.0 && crate::ctrl_down() && !confirming {
        dispatch(hwnd, Action::Tab(settings::Tab::Fonts));
        focus_control(hwnd, true);
        return;
    }
    let on_fonts = PanelState::with(|s| {
        s.settings
            .as_ref()
            .is_some_and(|p| p.ui.tab == settings::Tab::Fonts)
    })
    .unwrap_or(false);
    if on_fonts && (key == VK_DOWN.0 || key == VK_UP.0) {
        if let Some(layout) = layout(hwnd) {
            PanelState::with(|s| {
                if let Some(p) = s.settings.as_mut() {
                    layout.move_font_focus(&mut p.ui, key == VK_UP.0);
                }
            });
            focus_control(hwnd, false);
            invalidate(hwnd);
        }
        return;
    }
    if key == VK_ESCAPE.0 {
        let pending =
            PanelState::with(|s| s.settings.as_ref().is_some_and(|p| p.ui.pending.is_some()))
                .unwrap_or(false);
        dispatch(
            hwnd,
            if pending {
                Action::Cancel
            } else {
                Action::Close
            },
        );
    } else if key == VK_TAB.0 {
        let Some(layout) = layout(hwnd) else {
            return;
        };
        let backwards = crate::shift_down();
        PanelState::with(|s| {
            if let Some(p) = s.settings.as_mut() {
                layout.focus_next(&mut p.ui, backwards);
            }
        });
        focus_control(hwnd, false);
        invalidate(hwnd);
    } else if key == VK_RETURN.0 || key == VK_SPACE.0 {
        let focused =
            PanelState::with(|s| s.settings.as_ref().and_then(|p| p.ui.focus.clone())).flatten();
        if focused == Some(Action::FontSearch) && key == VK_RETURN.0 {
            let first = PanelState::with(|s| {
                let p = s.settings.as_ref()?;
                let index = *p.ui.font_matches.first()?;
                Some(p.ui.font_names[index].name.clone())
            })
            .flatten();
            if let Some(name) = first {
                dispatch(hwnd, Action::FontFamily(name));
            }
        } else if let Some(action) = focused {
            dispatch(hwnd, action);
        }
    } else {
        let page = layout(hwnd).map(|l| l.view_height()).unwrap_or(200.0);
        let delta = match key {
            v if v == VK_DOWN.0 => 36.0,
            v if v == VK_UP.0 => -36.0,
            v if v == VK_NEXT.0 => page,
            v if v == VK_PRIOR.0 => -page,
            v if v == VK_HOME.0 => f32::MIN,
            v if v == VK_END.0 => f32::MAX,
            _ => return,
        };
        scroll(hwnd, delta);
    }
}

fn dispatch(hwnd: HWND, action: Action) {
    let owner = PanelState::with(|s| s.settings.as_ref().map(|p| p.owner)).flatten();
    let Some(owner) = owner else {
        return;
    };
    let mut actions = Vec::new();
    let restore_focus = matches!(action, Action::Topmost | Action::Bottommost);
    let keep_search_focus = matches!(action, Action::FontFamily(_))
        && PanelState::with(|s| {
            s.settings
                .as_ref()
                .is_some_and(|p| p.ui.focus == Some(Action::FontSearch))
        })
        .unwrap_or(false);
    match action {
        Action::Close => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        Action::Quit => actions.push(PanelAction::Quit),
        Action::Tab(tab) => {
            PanelState::with(|s| {
                if let Some(p) = s.settings.as_mut() {
                    p.ui.tab = tab;
                    p.ui.scroll = 0.0;
                    p.ui.hover = None;
                    p.ui.focus = Some(if tab == settings::Tab::Fonts {
                        Action::FontSearch
                    } else {
                        Action::Tab(tab)
                    });
                }
            });
            sync_search_box(hwnd);
            focus_control(hwnd, false);
        }
        Action::Rows(rows) => actions.push(PanelAction::ResizeToRows(rows)),
        Action::Font(size) => actions.push(PanelAction::SetFontSize(size)),
        Action::FontFamily(name) => actions.push(PanelAction::SetFontFamily(name)),
        Action::FontSearch => focus_control(hwnd, false),
        Action::SingleLine => crate::on_command(WPARAM(crate::MENU_SINGLE_LINE), &mut actions),
        Action::Pin => crate::on_command(WPARAM(crate::MENU_PIN_POSITION), &mut actions),
        Action::Topmost => crate::on_command(WPARAM(crate::MENU_TOPMOST), &mut actions),
        Action::Bottommost => crate::on_command(WPARAM(crate::MENU_BOTTOMMOST), &mut actions),
        Action::Refresh => crate::on_command(WPARAM(crate::MENU_REFRESH), &mut actions),
        Action::OpenRequests => crate::on_command(WPARAM(crate::MENU_OPEN), &mut actions),
        Action::AddAccount => begin_login(owner, LoginTarget::Add),
        Action::Login(id) => begin_login(owner, LoginTarget::Account(id)),
        Action::RemoveAccount(_) | Action::ClearCredentials => {
            PanelState::with(|s| {
                if let Some(p) = s.settings.as_mut() {
                    p.ui.pending = Some(action);
                    p.ui.focus = Some(Action::Cancel);
                    p.ui.hover = None;
                }
            });
        }
        Action::Cancel => {
            PanelState::with(|s| {
                if let Some(p) = s.settings.as_mut() {
                    p.ui.pending = None;
                    p.ui.focus = None;
                }
            });
        }
        Action::Confirm => {
            let pending = PanelState::with(|s| {
                s.settings.as_mut().and_then(|p| {
                    p.ui.focus = None;
                    p.ui.pending.take()
                })
            })
            .flatten();
            match pending {
                Some(Action::RemoveAccount(id)) => actions.push(PanelAction::RemoveAccount(id)),
                Some(Action::ClearCredentials) => actions.push(PanelAction::ClearCredentials),
                _ => {}
            }
        }
        Action::CredentialMode(mode) => {
            PanelState::with(|s| {
                s.app.config.credential_mode = mode;
                if let Some(form) = s.app.login.as_mut() {
                    form.mode = mode;
                }
                s.app.config.store(&s.app.stored);
                s.app.save();
            });
        }
        Action::ClearChannels => {
            PanelState::with(|s| {
                s.app.config.hidden_channels.clear();
                s.app.scroll = 0.0;
                s.app.rebuild();
                s.app.save();
                s.app.status = Some(("已清除渠道过滤".into(), false));
            });
        }
        Action::Channel(entry) => {
            PanelState::with(|s| {
                s.app
                    .config
                    .toggle_hidden_channel(&entry.account_id, &entry.channel);
                s.app.scroll = 0.0;
                s.app.rebuild();
                s.app.save();
            });
        }
    }
    crate::run_actions(owner, actions);
    // Changing the owner's Z order can lower its owned settings window too.
    // Restore focus only for that internal transition. Opening a browser should
    // genuinely deactivate and dismiss settings instead of stealing focus back.
    if restore_focus && unsafe { IsWindowVisible(hwnd).as_bool() } {
        focus(hwnd);
    } else if keep_search_focus {
        focus_control(hwnd, false);
    }
    invalidate(owner);
    redraw();
}

fn paint(hwnd: HWND) {
    let Some(layout) = layout(hwnd) else {
        unsafe {
            let _ = DefWindowProcW(hwnd, WM_PAINT, WPARAM(0), LPARAM(0));
        }
        return;
    };
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        crate::with_double_buffer(hdc, rc, |painter| {
            PanelState::with(|s| {
                if let Some(p) = s.settings.as_ref() {
                    layout.draw(&painter, &p.fonts, &p.ui, p.scale);
                }
            });
        });
        let _ = EndPaint(hwnd, &ps);
    }
}

fn begin_login(owner: HWND, target: LoginTarget) {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetWindowRect(owner, &mut rc);
    }
    let popup = PanelState::with(|s| {
        s.settings_login = Some(LoginReturn {
            geometry: WindowState {
                x: rc.left,
                y: rc.top,
                width: rc.right - rc.left,
                height: rc.bottom - rc.top,
            },
            view: s.app.view,
            form: s.app.login.take(),
            anchor: s.settings.as_ref().map(|p| p.anchor).unwrap_or_default(),
        });
        s.app.open_login(None, target);
        if let Some(form) = s.app.login.as_mut() {
            form.return_to_settings = true;
        }
        s.settings.as_ref().map(|p| p.hwnd)
    })
    .flatten();
    if let Some(hwnd) = popup {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
    let scale = crate::window_scale(owner);
    let work = crate::work_area_for(owner);
    let fitted = fit(
        RECT {
            right: rc.left + (452.0 * scale) as i32,
            bottom: rc.top + (440.0 * scale) as i32,
            ..rc
        },
        work,
    );
    unsafe {
        let _ = SetWindowPos(
            owner,
            Some(HWND_TOP),
            fitted.left,
            fitted.top,
            fitted.right - fitted.left,
            fitted.bottom - fitted.top,
            SWP_NOACTIVATE,
        );
    }
    crate::apply_activation(owner, true);
    PanelState::with(|s| s.activatable = true);
    invalidate(owner);
}

pub fn finish_login(owner: HWND, cancel: bool) {
    let previous = PanelState::with(|s| {
        let previous = s.settings_login.take()?;
        if cancel {
            s.app.view = previous.view;
            s.app.login = previous.form;
        } else {
            s.app.login = None;
        }
        Some((previous.geometry, previous.anchor))
    })
    .flatten();
    let Some((rect, anchor)) = previous else {
        return;
    };
    unsafe {
        let _ = SetWindowPos(
            owner,
            None,
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    let mut actions = Vec::new();
    crate::sync_activation(&mut actions);
    if PanelState::with(|s| !s.app.is_login() && s.app.config.always_on_bottom).unwrap_or(false) {
        actions.push(PanelAction::SetBottom(true));
    }
    crate::run_actions(owner, actions);
    invalidate(owner);
    open(owner, anchor);
    PanelState::with(|s| {
        if let Some(p) = s.settings.as_mut() {
            p.ui.tab = settings::Tab::Accounts;
        }
    });
    redraw();
}
