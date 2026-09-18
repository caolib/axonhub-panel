//! Palette and GDI+ drawing primitives.
//!
//! Font objects and family handles are created once and reused for the whole
//! process lifetime: a full repaint draws a few hundred strings, and building
//! GDI+ objects per call would be pure waste at any cadence.

use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::Graphics::GdiPlus::*;
pub use windows::Win32::Graphics::GdiPlus::{
    GpBrush, GpFont, GpFontFamily, GpGraphics, GpPath, GpPen, GpSolidFill, GpStringFormat, RectF,
};
use windows::core::PCWSTR;

// --- palette (dark theme, aligned with the AxonHub console) ---
pub const BG: u32 = 0xFF12_141A;
pub const CARD: u32 = 0xFF19_1C22;
pub const CARD_HOVER: u32 = 0xFF22_272F;
pub const BORDER: u32 = 0xFF26_2B33;
pub const TEXT: u32 = 0xFFE6_E8EE;
pub const TEXT_DIM: u32 = 0xFF8B_94A0;
pub const TEXT_FAINT: u32 = 0xFF5F_6773;
pub const SEPARATOR: u32 = 0xFF20_242B;
pub const MAUVE: u32 = 0xFFC4_A0E0;
/// Default colour for model names in the card palette.
pub const ORANGE: u32 = 0xFFD9_7757;

// --- status colours ---
pub const GREEN: u32 = 0xFF3F_B950;
pub const BLUE: u32 = 0xFF4C_9AFF;
pub const CYAN: u32 = 0xFF79_C0FF;
pub const RED: u32 = 0xFFF8_5149;
pub const GRAY: u32 = 0xFF8B_949E;
pub const YELLOW: u32 = 0xFFD2_9922;
/// Bright, unambiguous highlight for the 流 / 转 / 透 marks.
pub const WHITE: u32 = 0xFF_FF_FF_FF;

/// The single colour that encodes a request's state in the list.
pub fn status_color(status: crate::model::Status) -> u32 {
    use crate::model::Status;
    match status {
        Status::Completed => GREEN,
        Status::Failed => RED,
        Status::Processing => BLUE,
        Status::Pending => YELLOW,
        Status::Canceled => GRAY,
    }
}

// --- enum values (GDI+ declares these as newtypes, not Rust enums) ---
const UNIT_PIXEL: Unit = Unit(2);
const FILL_WINDING: FillMode = FillMode(0);
const SMOOTH_ANTIALIAS: SmoothingMode = SmoothingMode(4);
const TEXT_CLEARTYPE: TextRenderingHint = TextRenderingHint(5);
const OFFSET_HALF: PixelOffsetMode = PixelOffsetMode(4);
const TRIM_ELLIPSIS: StringTrimming = StringTrimming(3);
const LINE_ALIGN_TOP: StringAlignment = StringAlignment(0);
const NO_WRAP: i32 = 0x1000;

pub const ALIGN_NEAR: u32 = 0;
pub const ALIGN_CENTER: u32 = 1;
pub const ALIGN_FAR: u32 = 2;

/// The font sizes the panel uses. Sizes are in device pixels (the process is
/// per-monitor DPI aware, so these scale with `ctx.scale`).
/// A font paired with the CJK face used for its Chinese glyphs.
///
/// GDI+ has no per-run fallback: whatever family is passed to `DrawString`
/// supplies every glyph, and a missing one is substituted by the system at an
/// unknown size. So Chinese and non-Chinese runs are measured and drawn
/// separately, each with its own face, sharing one baseline.
#[derive(Clone, Copy)]
pub struct DualFont {
    /// Latin, digits and punctuation. Monospaced, which is what aligns columns.
    pub latin: *mut GpFont,
    /// Chinese and other CJK.
    pub cjk: *mut GpFont,
}

impl DualFont {
    fn is_null(&self) -> bool {
        self.latin.is_null()
    }
}

pub struct Fonts {
    /// Row metrics and chips.
    pub small: DualFont,
    /// Card titles and values.
    pub body: DualFont,
    /// Panel title and emphasis.
    pub bold: DualFont,
    latin_family: *mut GpFontFamily,
    cjk_family: *mut GpFontFamily,
}

// GDI+ objects are only ever touched from the UI thread.
unsafe impl Send for Fonts {}

/// First family in `names` that the system can resolve, or null if none can.
fn pick(names: &[&str]) -> *mut GpFontFamily {
    for name in names {
        let f = family(name);
        if !f.is_null() {
            return f;
        }
    }
    std::ptr::null_mut()
}

fn family(name: &str) -> *mut GpFontFamily {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut out: *mut GpFontFamily = std::ptr::null_mut();
    unsafe {
        if GdipCreateFontFamilyFromName(PCWSTR(wide.as_ptr()), std::ptr::null_mut(), &mut out).0
            != 0
        {
            return std::ptr::null_mut();
        }
    }
    out
}

fn make_font(fam: *mut GpFontFamily, size: f32, bold: bool) -> *mut GpFont {
    if fam.is_null() {
        return std::ptr::null_mut();
    }
    let mut font: *mut GpFont = std::ptr::null_mut();
    unsafe {
        GdipCreateFont(fam, size, if bold { 1 } else { 0 }, UNIT_PIXEL, &mut font);
    }
    font
}

impl Fonts {
    pub fn load(scale: f32) -> Self {
        // Single font family for both Latin and CJK — JetBrainsLxgwNerdMono
        // covers both with 2:1 metrics, so run splitting uses the same metrics
        // everywhere.
        let family = pick(&[
            "JetBrainsLxgwNerdMono",
            "Maple Mono NF CN",
            "Microsoft YaHei UI",
            "Segoe UI",
        ]);
        let latin_family = family;
        let cjk_family = family;

        let s = |v: f32| v * scale;
        let dual = |size: f32, bold: bool| DualFont {
            latin: make_font(latin_family, s(size), bold),
            cjk: make_font(cjk_family, s(size), bold),
        };

        Fonts {
            small: dual(11.0, false),
            body: dual(12.5, false),
            bold: dual(12.5, true),
            latin_family,
            cjk_family,
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            for f in [self.small, self.body, self.bold] {
                for font in [f.latin, f.cjk] {
                    if !font.is_null() {
                        GdipDeleteFont(font);
                    }
                }
            }
            for fam in [self.latin_family, self.cjk_family] {
                if !fam.is_null() {
                    GdipDeleteFontFamily(fam);
                }
            }
        }
    }
}

/// A GDI+ drawing session bound to one `WM_PAINT`.
pub struct Painter {
    g: *mut GpGraphics,
}

impl Painter {
    pub fn new(hdc: HDC) -> Option<Self> {
        let mut g: *mut GpGraphics = std::ptr::null_mut();
        unsafe {
            if GdipCreateFromHDC(hdc, &mut g).0 != 0 || g.is_null() {
                return None;
            }
            GdipSetSmoothingMode(g, SMOOTH_ANTIALIAS);
            GdipSetTextRenderingHint(g, TEXT_CLEARTYPE);
            GdipSetPixelOffsetMode(g, OFFSET_HALF);
        }
        Some(Painter { g })
    }

    /// Restrict drawing to a rectangle so clipped text cannot bleed.
    pub fn clip(&self, r: RectF) {
        unsafe {
            let _ = GdipSetClipRect(self.g, r.X, r.Y, r.Width, r.Height, CombineModeIntersect);
        }
    }

    pub fn reset_clip(&self) {
        unsafe {
            let _ = GdipResetClip(self.g);
        }
    }

    pub fn fill_rect(&self, x: f32, y: f32, w: f32, h: f32, color: u32) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        unsafe {
            let mut brush: *mut GpSolidFill = std::ptr::null_mut();
            if GdipCreateSolidFill(color, &mut brush).0 != 0 {
                return;
            }
            GdipFillRectangle(self.g, brush as *mut GpBrush, x, y, w, h);
            GdipDeleteBrush(brush as *mut GpBrush);
        }
    }

    pub fn round_rect(&self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: u32, fill: bool) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let r = radius.min(h / 2.0).min(w / 2.0).max(0.0);
        unsafe {
            let mut path: *mut GpPath = std::ptr::null_mut();
            if GdipCreatePath(FILL_WINDING, &mut path).0 != 0 {
                return;
            }
            let d = r * 2.0;
            let (xi, yi, di) = (x as i32, y as i32, d as i32);
            let (ri, bi) = ((x + w - d) as i32, (y + h - d) as i32);
            GdipAddPathArcI(path, xi, yi, di, di, 180.0, 90.0);
            GdipAddPathArcI(path, ri, yi, di, di, 270.0, 90.0);
            GdipAddPathArcI(path, ri, bi, di, di, 0.0, 90.0);
            GdipAddPathArcI(path, xi, bi, di, di, 90.0, 90.0);
            GdipClosePathFigure(path);

            if fill {
                let mut brush: *mut GpSolidFill = std::ptr::null_mut();
                if GdipCreateSolidFill(color, &mut brush).0 == 0 {
                    GdipFillPath(self.g, brush as *mut GpBrush, path);
                    GdipDeleteBrush(brush as *mut GpBrush);
                }
            } else {
                let mut pen: *mut GpPen = std::ptr::null_mut();
                if GdipCreatePen1(color, 1.0, UNIT_PIXEL, &mut pen).0 == 0 {
                    GdipDrawPath(self.g, pen, path);
                    GdipDeletePen(pen);
                }
            }
            GdipDeletePath(path);
        }
    }

    /// Draw `s` with the right face per run: Chinese gets `font.cjk`, the rest
    /// `font.latin`.
    #[allow(clippy::too_many_arguments)]
    pub fn dual_text(
        &self,
        font: &DualFont,
        s: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: u32,
        align: u32,
    ) {
        if font.is_null() || s.is_empty() || w <= 0.0 {
            return;
        }
        let runs = split_runs(s);
        if runs.len() == 1 {
            // The common case: nothing to align, draw straight through.
            let (text, cjk) = runs[0];
            self.text(
                if cjk { font.cjk } else { font.latin },
                text,
                x,
                y,
                w,
                h,
                color,
                align,
            );
            return;
        }

        let total: f32 = runs
            .iter()
            .map(|(t, cjk)| self.measure(if *cjk { font.cjk } else { font.latin }, t))
            .sum();
        let mut cursor = match align {
            ALIGN_CENTER => x + (w - total) / 2.0,
            ALIGN_FAR => x + w - total,
            _ => x,
        };
        // Clip so a run cannot spill past the cell when the text is too long.
        let clip = RectF {
            X: x,
            Y: y,
            Width: w,
            Height: h,
        };
        self.clip(clip);
        for (text, cjk) in runs {
            let face = if cjk { font.cjk } else { font.latin };
            let run_w = self.measure(face, text);
            // Each run is drawn left-aligned at its own offset; the shared
            // rectangle height keeps all runs on one baseline.
            self.text(face, text, cursor, y, run_w.max(1.0), h, color, ALIGN_NEAR);
            cursor += run_w;
        }
        self.reset_clip();
    }

    /// Width of `s` when drawn with `font`, mixing faces per run.
    pub fn dual_measure(&self, font: &DualFont, s: &str) -> f32 {
        if font.is_null() || s.is_empty() {
            return 0.0;
        }
        split_runs(s)
            .iter()
            .map(|(t, cjk)| self.measure(if *cjk { font.cjk } else { font.latin }, t))
            .sum()
    }

    /// Break `text` into lines that fit `max_w`, using this painter's real font
    /// metrics. Newlines in the input always start a new line.
    pub fn wrap(&self, font: &DualFont, text: &str, max_w: f32) -> Vec<String> {
        wrap_by(text, max_w, |s| self.dual_measure(font, s))
    }

    pub fn text(
        &self,
        font: *mut GpFont,
        s: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: u32,
        align: u32,
    ) {
        if font.is_null() || s.is_empty() || w <= 0.0 {
            return;
        }
        unsafe {
            let mut brush: *mut GpSolidFill = std::ptr::null_mut();
            if GdipCreateSolidFill(color, &mut brush).0 != 0 {
                return;
            }
            let mut fmt: *mut GpStringFormat = std::ptr::null_mut();
            GdipCreateStringFormat(0, 0, &mut fmt);
            GdipSetStringFormatAlign(fmt, StringAlignment(align as i32));
            GdipSetStringFormatLineAlign(fmt, LINE_ALIGN_TOP);
            GdipSetStringFormatTrimming(fmt, TRIM_ELLIPSIS);
            GdipSetStringFormatFlags(fmt, NO_WRAP);

            let wide: Vec<u16> = s.encode_utf16().collect();
            let rect = RectF {
                X: x,
                Y: y,
                Width: w,
                Height: h,
            };
            GdipDrawString(
                self.g,
                PCWSTR(wide.as_ptr()),
                wide.len() as i32,
                font,
                &rect,
                fmt,
                brush as *mut GpBrush,
            );
            GdipDeleteStringFormat(fmt);
            GdipDeleteBrush(brush as *mut GpBrush);
        }
    }

    /// Width of `s` when laid out with `font`, for right-aligned compositions.
    pub fn measure(&self, font: *mut GpFont, s: &str) -> f32 {
        if font.is_null() || s.is_empty() {
            return 0.0;
        }
        unsafe {
            let wide: Vec<u16> = s.encode_utf16().collect();
            let layout = RectF {
                X: 0.0,
                Y: 0.0,
                Width: 10_000.0,
                Height: 200.0,
            };
            let mut bounds = RectF::default();
            let mut fmt: *mut GpStringFormat = std::ptr::null_mut();
            GdipCreateStringFormat(0, 0, &mut fmt);
            GdipSetStringFormatFlags(fmt, NO_WRAP);
            let status = GdipMeasureString(
                self.g,
                PCWSTR(wide.as_ptr()),
                wide.len() as i32,
                font,
                &layout,
                fmt,
                &mut bounds,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            GdipDeleteStringFormat(fmt);
            if status.0 == 0 { bounds.Width } else { 0.0 }
        }
    }
}

impl Drop for Painter {
    fn drop(&mut self) {
        unsafe {
            if !self.g.is_null() {
                GdipDeleteGraphics(self.g);
            }
        }
    }
}

/// Split `s` into maximal runs of CJK and non-CJK, in order.
///
/// CJK detection is by code point range rather than by font query: it avoids a
/// per-glyph API call on every paint, and the ranges below cover what this
/// panel actually renders (Chinese labels and full-width punctuation).
pub fn split_runs(s: &str) -> Vec<(&str, bool)> {
    let mut runs: Vec<(&str, bool)> = Vec::new();
    let mut start = 0usize;
    let mut current: Option<bool> = None;
    for (idx, c) in s.char_indices() {
        let cjk = is_cjk(c);
        match current {
            None => current = Some(cjk),
            Some(prev) if prev != cjk => {
                runs.push((&s[start..idx], prev));
                start = idx;
                current = Some(cjk);
            }
            _ => {}
        }
    }
    if let Some(kind) = current {
        runs.push((&s[start..], kind));
    }
    runs
}

/// Whether `c` is drawn with the CJK face. See `split_runs` for why the ranges
/// are enumerated rather than queried from the font.
pub fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3000..=0x303F |   // CJK punctuation
        0x3400..=0x4DBF |   // extension A
        0x4E00..=0x9FFF |   // unified ideographs
        0xF900..=0xFAFF |   // compatibility ideographs
        0xFF00..=0xFFEF |   // full-width forms
        0x20000..=0x2FA1F
    )
}

/// Break `text` into lines that fit `max_w` according to `measure`.
///
/// Pure so it can be exercised without a device context (`Painter::wrap` binds
/// it to real font metrics). Latin breaks fall between words and CJK breaks
/// between characters, which is how the two scripts are read; a single token
/// wider than the whole column — a long URL, a hash — is broken by character so
/// nothing is dropped.
pub fn wrap_by(text: &str, max_w: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let max_w = max_w.max(1.0);
    let mut out: Vec<String> = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for token in tokens(para) {
            if token.trim().is_empty() {
                // A run of spaces only matters between two words; a leading one
                // is dropped so wrapped lines start flush.
                if !line.is_empty() {
                    line.push_str(token);
                }
                continue;
            }
            if line.trim().is_empty() {
                line.clear();
                push_token(&mut line, &mut out, token, max_w, &measure);
                continue;
            }
            let joined = format!("{line}{token}");
            if measure(joined.trim_end()) <= max_w {
                line = joined;
            } else {
                out.push(line.trim_end().to_string());
                line = String::new();
                push_token(&mut line, &mut out, token, max_w, &measure);
            }
        }
        out.push(line.trim_end().to_string());
    }
    out
}

/// Append `token` to `line`, emitting whole lines while it does not fit.
fn push_token(
    line: &mut String,
    out: &mut Vec<String>,
    token: &str,
    max_w: f32,
    measure: &impl Fn(&str) -> f32,
) {
    let mut rest = token;
    loop {
        if measure(rest) <= max_w {
            line.push_str(rest);
            return;
        }
        let cut = fitting_prefix(rest, max_w, measure);
        if cut == 0 {
            // A single character wider than the column: keep it rather than
            // spin forever on a token that can never fit.
            line.push_str(rest);
            return;
        }
        out.push(rest[..cut].to_string());
        rest = &rest[cut..];
    }
}

/// Byte length of the longest prefix of `s` whose width stays within `max_w`.
fn fitting_prefix(s: &str, max_w: f32, measure: &impl Fn(&str) -> f32) -> usize {
    let mut width = 0.0;
    for (i, c) in s.char_indices() {
        width += measure(&s[i..i + c.len_utf8()]);
        if width > max_w {
            return i;
        }
    }
    s.len()
}

/// Break `s` into the smallest pieces a line break may fall between. Every
/// slice together reproduces `s` exactly.
fn tokens(s: &str) -> Vec<&str> {
    #[derive(PartialEq, Clone, Copy)]
    enum Kind {
        Space,
        Word,
        Cjk,
    }
    fn kind_of(c: char) -> Kind {
        if c.is_whitespace() {
            Kind::Space
        } else if is_cjk(c) {
            Kind::Cjk
        } else {
            Kind::Word
        }
    }

    let mut out: Vec<&str> = Vec::new();
    let mut start = 0usize;
    let mut kind: Option<Kind> = None;
    for (i, c) in s.char_indices() {
        let k = kind_of(c);
        match kind {
            None => kind = Some(k),
            // CJK characters each stand alone; other runs stay together.
            Some(prev) if prev != k || k == Kind::Cjk => {
                out.push(&s[start..i]);
                start = i;
                kind = Some(k);
            }
            _ => {}
        }
    }
    if !s.is_empty() {
        out.push(&s[start..]);
    }
    out
}

/// Replace a colour's alpha channel.
pub fn with_alpha(color: u32, alpha: u32) -> u32 {
    (color & 0x00FF_FFFF) | ((alpha & 0xFF) << 24)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(s: &str) -> Vec<(&str, bool)> {
        split_runs(s)
    }

    #[test]
    fn ascii_only_is_one_run() {
        assert_eq!(runs("claude-opus-5"), vec![("claude-opus-5", false)]);
        assert_eq!(runs("3.4s"), vec![("3.4s", false)]);
    }

    #[test]
    fn chinese_only_is_one_run() {
        assert_eq!(runs("已完成"), vec![("已完成", true)]);
    }

    #[test]
    fn mixed_text_alternates_runs() {
        // The label shape the panel actually draws.
        assert_eq!(runs("缓存 99.5%"), vec![("缓存", true), (" 99.5%", false)]);
        // The space after a digit stays with the Latin run; it is only the
        // run boundaries that matter for the face, not which side the space
        // lands on.
        assert_eq!(runs("5 分钟前"), vec![("5 ", false), ("分钟前", true)]);
    }

    #[test]
    fn preserved_text_is_byte_exact() {
        // Concatenating the runs must reproduce the input, or text would be
        // silently mangled when drawn piecewise.
        for s in ["缓存 99.5%", "5 分钟前", "cc · caolib", "透传", "abc", ""] {
            let joined: String = runs(s).into_iter().map(|(t, _)| t).collect();
            assert_eq!(joined, s, "round trip failed for {s:?}");
        }
    }

    #[test]
    fn full_width_punctuation_counts_as_cjk() {
        // `·` is U+00B7 (Latin), but `：` is full-width and needs the CJK face.
        assert_eq!(runs("凭据:token"), vec![("凭据", true), (":token", false)]);
        assert_eq!(runs("凭据："), vec![("凭据：", true)]);
    }

    #[test]
    fn leading_and_trailing_spaces_stay_with_their_run() {
        assert_eq!(
            runs(" 完成 "),
            vec![(" ", false), ("完成", true), (" ", false)]
        );
    }

    /// Stand-in for the monospaced face: one unit per character, so a test
    /// budget of N * 10 is exactly N characters.
    fn mono(s: &str) -> f32 {
        s.chars().count() as f32 * 10.0
    }

    #[test]
    fn wraps_latin_between_words() {
        assert_eq!(
            wrap_by("alpha beta gamma", 100.0, mono),
            vec!["alpha beta", "gamma"]
        );
    }

    #[test]
    fn wraps_cjk_between_characters() {
        assert_eq!(wrap_by("中文中文", 20.0, mono), vec!["中文", "中文"]);
    }

    #[test]
    fn breaks_a_token_wider_than_the_column() {
        let url = "https://example.com/very/long/path";
        let lines = wrap_by(url, 40.0, mono);
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|l| mono(l) <= 40.0), "{lines:?}");
        assert_eq!(lines.concat(), url);
    }

    #[test]
    fn newlines_start_a_new_line() {
        assert_eq!(wrap_by("a\nb", 100.0, mono), vec!["a", "b"]);
        // An empty paragraph survives as an empty line.
        assert_eq!(wrap_by("a\n\nb", 100.0, mono), vec!["a", "", "b"]);
    }

    #[test]
    fn wrapping_keeps_every_character() {
        for text in [
            "failed to do request: HTTP request failed: Post \"https://api.example.com/v1/chat/completions?beta=true\": context canceled",
            "Concurrency limit exceeded for account, please retry later",
            "模型 glm-5.3-flash 在渠道 aliyun 上返回 429",
        ] {
            let joined: String = wrap_by(text, 120.0, mono).join(" ");
            let strip = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
            assert_eq!(strip(&joined), strip(text), "text was mangled: {text}");
        }
    }
}
