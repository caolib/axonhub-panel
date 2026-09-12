//! Pure geometry: where every card and chip lands for a given window size.
//!
//! Kept free of Win32 and GDI+ so hit-testing and painting share one source of
//! truth and the layout stays testable by inspection.
//!
//! Every constant below is authored for a 96-DPI display and multiplied by the
//! monitor scale when a `Metrics` is built. Fonts are scaled the same way, so
//! text and the boxes holding it stay in proportion on a scaled monitor.

pub const PAD: f32 = 8.0;
pub const CARD_H: f32 = 44.0;
pub const CARD_GAP: f32 = 4.0;
pub const HEADER_H: f32 = 30.0;

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

#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    pub width: f32,
    pub height: f32,
    /// Device-pixel scale of the monitor the window is on (1.0 at 96 DPI).
    pub scale: f32,
    pub header_h: f32,
    pub card_h: f32,
    pub card_gap: f32,
}

impl Metrics {
    /// Build metrics for a client area of `width` x `height` device pixels on a
    /// monitor scaled by `scale`.
    pub fn new(width: f32, height: f32, scale: f32) -> Self {
        let s = scale.max(0.5);
        Metrics {
            width,
            height,
            scale: s,
            header_h: HEADER_H * s,
            card_h: CARD_H * s,
            card_gap: CARD_GAP * s,
        }
    }

    pub fn pad(&self) -> f32 {
        PAD * self.scale
    }

    /// Vertical distance between the tops of adjacent cards.
    pub fn pitch(&self) -> f32 {
        self.card_h + self.card_gap
    }

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

/// How many whole request cards fit at `scale`.
pub fn rows_in_height(height: f32, scale: f32) -> usize {
    let m = Metrics::new(0.0, height, scale);
    let viewport = m.rows_viewport_h();
    if viewport <= 0.0 {
        return 1;
    }
    ((viewport + m.card_gap) / m.pitch()).floor().max(1.0) as usize
}

/// Window height that shows exactly `rows` cards with no partial card left over,
/// at `scale`. Callers clamp the result to the monitor work area: a tall panel
/// on a short screen is expected to scroll rather than overflow.
pub fn height_for_rows(rows: usize, scale: f32) -> i32 {
    let m = Metrics::new(0.0, 0.0, scale);
    let rows = rows.max(1);
    (m.header_h + rows as f32 * m.pitch() - m.card_gap).ceil() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_scales_with_the_monitor() {
        let at_100 = Metrics::new(400.0, 800.0, 1.0);
        let at_150 = Metrics::new(600.0, 1200.0, 1.5);
        assert_eq!(at_100.card_h, 44.0);
        assert_eq!(at_150.card_h, 66.0);
        assert_eq!(at_150.pitch(), 72.0);
        assert_eq!(at_150.pad(), 12.0);
    }

    #[test]
    fn height_for_rows_round_trips_at_each_scale() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for rows in [1usize, 5, 12, 30] {
                let h = height_for_rows(rows, scale) as f32;
                assert_eq!(
                    rows_in_height(h, scale),
                    rows,
                    "scale {scale} rows {rows}: height {h} reported a different count"
                );
            }
        }
    }

    #[test]
    fn card_rects_do_not_overlap() {
        let m = Metrics::new(400.0, 400.0, 1.5);
        let a = m.card_rect(0, 0.0);
        let b = m.card_rect(1, 0.0);
        assert_eq!(b.y, a.y + m.pitch());
        assert!(b.y >= a.y + a.h);
    }

    #[test]
    fn row_at_finds_the_card_under_a_point() {
        let m = Metrics::new(400.0, 800.0, 1.0);
        let first = m.rows_top();
        assert_eq!(m.row_at(first + 1.0, 0.0, 5), Some(0));
        assert_eq!(m.row_at(first + m.pitch() + 1.0, 0.0, 5), Some(1));
        // Inside the inter-card gap, and above the list, nothing is hovered.
        assert_eq!(m.row_at(first + m.card_h + 2.0, 0.0, 5), None);
        assert_eq!(m.row_at(first - 4.0, 0.0, 5), None);
    }

    #[test]
    fn scrolling_is_clamped_to_the_content() {
        let m = Metrics::new(400.0, 200.0, 1.0);
        assert_eq!(m.max_scroll(0), 0.0);
        assert!(m.max_scroll(50) > 0.0);
        // A scroll offset beyond the end must still resolve to a real row.
        let scroll = m.max_scroll(50);
        let last_visible = m.row_at(m.rows_bottom() - 2.0, scroll, 50);
        assert!(last_visible.is_some());
    }
}
