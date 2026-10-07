//! Pure geometry: where every card and chip lands for a given window size.
//!
//! Kept free of Win32 and GDI+ so hit-testing and painting share one source of
//! truth and the layout stays testable by inspection.
//!
//! Every constant below is authored for a 96-DPI display and multiplied by the
//! monitor scale when a `Metrics` is built. Fonts are scaled the same way, so
//! text and the boxes holding it stay in proportion on a scaled monitor.

pub const PAD: f32 = 8.0;
/// Three content lines: marks, model, then the usage metrics.
pub const CARD_H: f32 = 68.0;
/// Card height in the single-line layout: one body line with the model chip
/// centred in it.
pub const CARD_H_SINGLE: f32 = 26.0;
pub const CARD_GAP: f32 = 4.0;
pub const HEADER_H: f32 = 30.0;
/// Height of one table row. Rows sit flush against each other (no gap), so the
/// pitch is exactly this.
pub const TABLE_ROW_H: f32 = 22.0;

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// How the list area renders the requests. Every mode keeps one entry per
/// request; they differ in the row they draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    /// Two-line cards: the roomy default.
    Cards,
    /// One-line cards: the same information compressed onto a single row.
    SingleLine,
    /// One table row per request: aligned columns, no card chrome.
    Table,
}

#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    pub width: f32,
    pub height: f32,
    /// Device-pixel scale of the monitor the window is on (1.0 at 96 DPI).
    pub scale: f32,
    /// Which layout the list area draws.
    pub mode: LayoutMode,
    pub header_h: f32,
    pub card_h: f32,
    pub card_gap: f32,
}

impl Metrics {
    /// Build metrics for a client area of `width` x `height` device pixels on a
    /// monitor scaled by `scale`, in the default two-line layout.
    pub fn new(width: f32, height: f32, scale: f32) -> Self {
        let s = scale.max(0.5);
        Metrics {
            width,
            height,
            scale: s,
            mode: LayoutMode::Cards,
            header_h: HEADER_H * s,
            card_h: CARD_H * s,
            card_gap: CARD_GAP * s,
        }
    }

    /// Adopt one layout mode's row height. Everything derived from `card_h` —
    /// the pitch, row rectangles, hit areas and scroll range — follows, so the
    /// caller only has to swap the mode.
    pub fn with_mode(mut self, mode: LayoutMode) -> Self {
        let (height, gap) = match mode {
            LayoutMode::Cards => (CARD_H, CARD_GAP),
            LayoutMode::SingleLine => (CARD_H_SINGLE, CARD_GAP),
            LayoutMode::Table => (TABLE_ROW_H, 0.0),
        };
        self.mode = mode;
        self.card_h = height * self.scale;
        self.card_gap = gap * self.scale;
        self
    }

    pub fn pad(&self) -> f32 {
        PAD * self.scale
    }

    /// Vertical distance between the tops of adjacent rows.
    pub fn pitch(&self) -> f32 {
        self.card_h + self.card_gap
    }

    /// Where the scrollable rows start: right below the panel header.
    pub fn rows_top(&self) -> f32 {
        self.header_h
    }

    /// Cards run all the way to the bottom edge now that there is no footer.
    pub fn rows_bottom(&self) -> f32 {
        self.height.max(self.header_h)
    }

    pub fn rows_viewport_h(&self) -> f32 {
        (self.rows_bottom() - self.rows_top()).max(0.0)
    }

    pub fn card_w(&self) -> f32 {
        (self.width - self.pad() * 2.0).max(0.0)
    }

    pub fn content_h(&self, row_count: usize) -> f32 {
        if row_count == 0 {
            return 0.0;
        }
        row_count as f32 * self.pitch() - self.card_gap
    }

    pub fn max_scroll(&self, row_count: usize) -> f32 {
        (self.content_h(row_count) - self.rows_viewport_h()).max(0.0)
    }

    /// Card rectangle for `index`, already offset by the scroll position.
    pub fn card_rect(&self, index: usize, scroll: f32) -> Rect {
        Rect {
            x: self.pad(),
            y: self.rows_top() + index as f32 * self.pitch() - scroll,
            w: self.card_w(),
            h: self.card_h,
        }
    }

    /// Which card sits under a viewport-space point, if any.
    pub fn row_at(&self, py: f32, scroll: f32, row_count: usize) -> Option<usize> {
        if py < self.rows_top() || py >= self.rows_bottom() {
            return None;
        }
        let local = py - self.rows_top() + scroll;
        if local < 0.0 {
            return None;
        }
        let index = (local / self.pitch()).floor();
        if index < 0.0 {
            return None;
        }
        let index = index as usize;
        if index >= row_count {
            return None;
        }
        // Reject the gap between cards so hover does not flicker.
        let within = local - index as f32 * self.pitch();
        if within > self.card_h {
            return None;
        }
        Some(index)
    }

    pub fn rows_area(&self) -> Rect {
        Rect {
            x: 0.0,
            y: self.rows_top(),
            w: self.width,
            h: self.rows_viewport_h(),
        }
    }
}

/// How many whole request rows fit at `scale`.
pub fn rows_in_height(height: f32, scale: f32, mode: LayoutMode) -> usize {
    let m = Metrics::new(0.0, height, scale).with_mode(mode);
    let viewport = m.rows_viewport_h();
    if viewport <= 0.0 {
        return 1;
    }
    ((viewport + m.card_gap) / m.pitch()).floor().max(1.0) as usize
}

/// Window height that shows exactly `rows` rows with no partial row left over,
/// at `scale`. Callers clamp the result to the monitor work area: a tall panel
/// on a short screen is expected to scroll rather than overflow.
pub fn height_for_rows(rows: usize, scale: f32, mode: LayoutMode) -> i32 {
    let m = Metrics::new(0.0, 0.0, scale).with_mode(mode);
    let rows = rows.max(1);
    (m.rows_top() + rows as f32 * m.pitch() - m.card_gap).ceil() as i32
}

/// Device pixels for a length authored at 96 DPI. The saved window size is kept
/// in logical units so the panel is the same apparent size on every monitor,
/// and only converted to device pixels for the monitor it currently sits on.
pub fn device_px(logical: i32, scale: f32) -> i32 {
    (logical as f32 * scale).round() as i32
}

/// The inverse of `device_px`, for persisting a window size that was measured in
/// device pixels. Never returns zero, so a degenerate window still round-trips.
pub fn logical_px(device: i32, scale: f32) -> i32 {
    (device as f32 / scale.max(0.01)).round().max(1.0) as i32
}

/// Scale that keeps a length the same *physical* size on the monitor described
/// by its device-pixel resolution (`px_w`/`px_h`) and EDID-reported glass size
/// in millimetres (`mm_w`/`mm_h`), relative to the 96-DPI baseline the layout
/// is authored against.
///
/// Windows' per-monitor DPI setting is the system's *guess* at density; it does
/// not match when two monitors share a scaling value but differ in pixels-per
/// inch, so the panel ends up too small on the denser screen. The physical
/// density is exact, so the scale is derived from it directly. Returns `None`
/// when the reported dimensions are missing or absurd (a driver without EDID
/// returns zeroes), so the caller can fall back to the Windows DPI guess.
pub fn physical_scale(px_w: i32, px_h: i32, mm_w: f32, mm_h: f32) -> Option<f32> {
    if px_w <= 0 || px_h <= 0 || mm_w <= 0.0 || mm_h <= 0.0 {
        return None;
    }
    let diag_px = ((px_w as f32).powi(2) + (px_h as f32).powi(2)).sqrt();
    let diag_in = ((mm_w * mm_w + mm_h * mm_h).sqrt()) / 25.4;
    // 5–80 inches covers every real panel and rules out a driver returning
    // the desktop extents, or a single stray value, by mistake.
    if !(5.0..=80.0).contains(&diag_in) {
        return None;
    }
    let ppi = diag_px / diag_in;
    // 60–500 PPI covers every desktop and laptop panel; outside it the EDID
    // is not to be trusted.
    if !(60.0..=500.0).contains(&ppi) {
        return None;
    }
    Some((ppi / 96.0).clamp(0.75, 4.0))
}

#[cfg(test)]
#[path = "../tests/ui/layout.rs"]
mod tests;
