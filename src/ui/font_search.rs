//! Native font-search edit control: IME, selection and clipboard stay native.

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, CreateSolidBrush, DEFAULT_CHARSET, DeleteObject, HBRUSH, HDC, HFONT,
    HGDIOBJ, LOGFONTW, SetBkColor, SetTextColor,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SetFocus, VK_A, VK_DOWN, VK_ESCAPE, VK_F, VK_RETURN, VK_TAB, VK_UP,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use crate::theme;

pub const ID: usize = 2100;
pub const WM_SEARCH_FOCUS: u32 = WM_APP + 42;
const EM_LIMITTEXT: u32 = 0x00C5;
const EM_SETSEL: u32 = 0x00B1;

pub struct SearchBox {
    pub hwnd: HWND,
    pub brush: HBRUSH,
    pub font: HFONT,
    pub scale: f32,
}

pub fn color(argb: u32) -> COLORREF {
    COLORREF(((argb >> 16) & 0xFF) | (argb & 0xFF00) | ((argb & 0xFF) << 16))
}

pub fn font(scale: f32) -> HFONT {
    let mut logical = LOGFONTW {
        lfHeight: -(13.0 * scale).round() as i32,
        lfCharSet: DEFAULT_CHARSET,
        ..Default::default()
    };
    for (to, from) in logical
        .lfFaceName
        .iter_mut()
        .zip("Microsoft YaHei UI".encode_utf16())
    {
        *to = from;
    }
    unsafe { CreateFontIndirectW(&logical) }
}

impl SearchBox {
    pub fn new(parent: HWND, scale: f32, query: &str) -> Option<Self> {
        let query: Vec<u16> = query.encode_utf16().chain(Some(0)).collect();
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("EDIT"),
                PCWSTR(query.as_ptr()),
                WS_CHILD | WS_TABSTOP | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                0,
                0,
                1,
                1,
                Some(parent),
                Some(HMENU(ID as *mut _)),
                None,
                None,
            )
        }
        .ok()?;
        let field = Self {
            hwnd,
            brush: unsafe { CreateSolidBrush(color(theme::INPUT_BG)) },
            font: font(scale),
            scale,
        };
        unsafe {
            let _ = SetWindowSubclass(hwnd, Some(edit_proc), ID, parent.0 as usize);
            SendMessageW(hwnd, EM_LIMITTEXT, Some(WPARAM(128)), None);
            SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(field.font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        Some(field)
    }

    pub fn color_dc(&self, dc: HDC) -> LRESULT {
        unsafe {
            SetBkColor(dc, color(theme::INPUT_BG));
            SetTextColor(dc, color(theme::TEXT));
        }
        LRESULT(self.brush.0 as isize)
    }
}

impl Drop for SearchBox {
    fn drop(&mut self) {
        // The parent destroys the child HWND before dropping this at NCDESTROY.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.font.0));
            let _ = DeleteObject(HGDIOBJ(self.brush.0));
        }
    }
}

pub fn query(hwnd: HWND) -> String {
    let mut text = [0u16; 129];
    let count = unsafe { GetWindowTextW(hwnd, &mut text) }.max(0) as usize;
    String::from_utf16_lossy(&text[..count])
}

pub fn focus(hwnd: HWND, select_all: bool) {
    unsafe {
        let _ = SetFocus(Some(hwnd));
        if select_all {
            SendMessageW(hwnd, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
        }
    }
}

unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    parent: usize,
) -> LRESULT {
    let parent = HWND(parent as *mut _);
    match msg {
        WM_SETFOCUS => unsafe {
            let _ = PostMessageW(Some(parent), WM_SEARCH_FOCUS, WPARAM(0), LPARAM(0));
        },
        WM_KEYDOWN => {
            let key = wp.0 as u16;
            if key == VK_A.0 && crate::ctrl_down() {
                unsafe {
                    SendMessageW(hwnd, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
                }
                return LRESULT(0);
            }
            if [VK_TAB.0, VK_ESCAPE.0, VK_RETURN.0, VK_DOWN.0, VK_UP.0].contains(&key)
                || (key == VK_F.0 && crate::ctrl_down())
            {
                unsafe {
                    let _ = SetFocus(Some(parent));
                    SendMessageW(parent, WM_KEYDOWN, Some(wp), Some(lp));
                }
                return LRESULT(0);
            }
        }
        WM_CHAR if [9, 13, 27].contains(&wp.0) => return LRESULT(0),
        WM_MOUSEWHEEL => return unsafe { SendMessageW(parent, msg, Some(wp), Some(lp)) },
        WM_NCDESTROY => unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(edit_proc), id);
        },
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}
