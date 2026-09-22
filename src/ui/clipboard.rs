//! Clipboard read/write helpers.
//!
//! Kept out of `main.rs` (the window-procedure god module) so the panel's
//! message handling stays focused on window concerns; the desktop clipboard is
//! an OS facility with its own lifetime rules.

use tracing::warn;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::System::Ole::CF_UNICODETEXT;

/// Read Unicode text from the clipboard. Returns `None` when the clipboard is
/// empty, holds no text, or is momentarily locked by another process.
pub fn clipboard_text() -> Option<String> {
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_err() {
            return None;
        }
        OpenClipboard(None).ok()?;
        // Every exit below must close the clipboard, or the rest of the desktop
        // is left unable to use copy/paste.
        let result = (|| {
            let handle = GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?;
            let hglobal = HGLOBAL(handle.0);
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

/// Put `text` on the clipboard as `CF_UNICODETEXT`, replacing its contents. A
/// clipboard another process is holding is left alone: copying is a nicety.
pub fn set_clipboard_text(text: &str) {
    unsafe {
        if OpenClipboard(None).is_err() {
            warn!("无法打开剪贴板,复制已跳过");
            return;
        }
        let _ = EmptyClipboard();
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        if let Ok(handle) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * std::mem::size_of::<u16>()) {
            let ptr = GlobalLock(handle) as *mut u16;
            if ptr.is_null() {
                let _ = GlobalFree(Some(handle));
            } else {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
                let _ = GlobalUnlock(handle);
                // Ownership passes to the clipboard; only a failed hand-off
                // leaves us holding the block.
                if SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(handle.0))).is_err() {
                    warn!("写入剪贴板失败,文本未复制");
                    let _ = GlobalFree(Some(handle));
                }
            }
        } else {
            warn!("分配剪贴板内存失败,文本未复制");
        }
        let _ = CloseClipboard();
    }
}
