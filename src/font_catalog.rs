//! Installed font names and glyph coverage, using the current user's font set.

use windows::Win32::Foundation::LPARAM;
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DEFAULT_CHARSET, DeleteObject, ENUMLOGFONTEXW, EnumFontFamiliesExW,
    GGI_MARK_NONEXISTING_GLYPHS, GetDC, GetGlyphIndicesW, HGDIOBJ, LOGFONTW, ReleaseDC,
    SelectObject, TEXTMETRICW,
};
use windows::core::w;

#[derive(Clone)]
pub struct InstalledFont {
    pub name: String,
    search: String,
}

impl InstalledFont {
    pub fn matches(&self, query: &str) -> bool {
        query
            .split_whitespace()
            .all(|part| self.search.contains(part))
    }
}

fn text(wide: &[u16]) -> String {
    let end = wide.iter().position(|c| *c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end])
}

unsafe extern "system" fn collect(
    font: *const LOGFONTW,
    _: *const TEXTMETRICW,
    _: u32,
    data: LPARAM,
) -> i32 {
    // EnumFontFamiliesExW passes ENUMLOGFONTEXW with LOGFONTW as its first field.
    let font = unsafe { &*(font as *const ENUMLOGFONTEXW) };
    let fonts = unsafe { &mut *(data.0 as *mut Vec<InstalledFont>) };
    let name = text(&font.elfLogFont.lfFaceName);
    if !name.is_empty() && !name.starts_with('@') && !fonts.iter().any(|f| f.name == name) {
        fonts.push(InstalledFont {
            search: format!("{} {}", name, text(&font.elfFullName)).to_lowercase(),
            name,
        });
    }
    1
}

pub fn installed() -> Vec<InstalledFont> {
    let mut fonts = Vec::new();
    unsafe {
        let dc = GetDC(None);
        if dc.is_invalid() {
            return fonts;
        }
        let query = LOGFONTW {
            lfCharSet: DEFAULT_CHARSET,
            ..Default::default()
        };
        EnumFontFamiliesExW(
            dc,
            &query,
            Some(collect),
            LPARAM(&mut fonts as *mut _ as isize),
            0,
        );
        let _ = ReleaseDC(None, dc);
    }
    // Raster/device-only faces cannot be rendered by the panel's GDI+ painter.
    fonts.retain(|font| crate::theme::font_available(&font.name));
    fonts.sort_by_cached_key(|font| font.name.to_lowercase());
    fonts
}

pub fn supports_chinese(name: &str) -> bool {
    let mut logical = LOGFONTW {
        lfHeight: -16,
        lfCharSet: DEFAULT_CHARSET,
        ..Default::default()
    };
    for (to, from) in logical
        .lfFaceName
        .iter_mut()
        .take(31)
        .zip(name.encode_utf16())
    {
        *to = from;
    }
    unsafe {
        let dc = GetDC(None);
        if dc.is_invalid() {
            return false;
        }
        let font = CreateFontIndirectW(&logical);
        if font.is_invalid() {
            let _ = ReleaseDC(None, dc);
            return false;
        }
        let previous = SelectObject(dc, HGDIOBJ(font.0));
        let mut glyphs = [0u16; 2];
        let result = GetGlyphIndicesW(
            dc,
            w!("中文"),
            2,
            glyphs.as_mut_ptr(),
            GGI_MARK_NONEXISTING_GLYPHS,
        );
        SelectObject(dc, previous);
        let _ = DeleteObject(HGDIOBJ(font.0));
        let _ = ReleaseDC(None, dc);
        result != u32::MAX && glyphs.iter().all(|g| *g != 0 && *g != 0xFFFF)
    }
}
