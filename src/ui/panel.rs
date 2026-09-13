//! Request card rendering.
//!
//! Mirrors the AxonHub requests page information hierarchy — identity/status,
//! model and routing, then channel, tokens, cache, latency and cost — but laid
//! out as cards rather than a table so it reads well at panel width. Everything
//! the console puts behind a tooltip is shown inline: the panel has room
//! because it is not trying to be a dense, sortable, filterable grid.

use windows::Win32::Graphics::GdiPlus::RectF;

use crate::format as fmt;
use crate::model::Row;
use crate::theme::{self, Painter};
use crate::ui::layout::{Metrics, Rect};

/// Authored at 96 DPI; multiplied by `Metrics::scale` at draw time.
const INNER_PAD: f32 = 7.0;
const LINE1_H: f32 = 17.0;
const LINE2_H: f32 = 14.0;
const CARD_RADIUS: f32 = 5.0;
const ACCENT_W: f32 = 4.0;
const ACCENT_INSET: f32 = 5.0;

/// The card's internal measurements for one draw, already scaled to the monitor.
#[derive(Clone, Copy)]
struct CardMetrics {
    inner_pad: f32,
    line1: f32,
    line2: f32,
    radius: f32,
    accent_w: f32,
    accent_inset: f32,
    /// The gap between the two text lines.
    gap: f32,
}

impl CardMetrics {
    fn new(m: Metrics) -> Self {
        let s = m.scale;
        CardMetrics {
            inner_pad: INNER_PAD * s,
            line1: LINE1_H * s,
            line2: LINE2_H * s,
            radius: CARD_RADIUS * s,
            accent_w: ACCENT_W * s,
            accent_inset: ACCENT_INSET * s,
            gap: 3.0 * s,
        }
    }
}

/// Everything the list needs besides the rows themselves.
pub struct ListView<'a> {
    pub rows: &'a [Row],
    pub total: i64,
    /// Unix seconds, used to render relative ages.
    pub now: i64,
    pub scroll: f32,
    pub hover: Option<usize>,
    pub selected: Option<usize>,
    pub status_text: Option<(String, bool)>,
    pub user: Option<&'a str>,
    pub pinned: bool,
}

pub fn draw(p: &Painter, fonts: &theme::Fonts, m: Metrics, v: &ListView) {
    p.fill_rect(0.0, 0.0, m.width, m.height, theme::BG);
    draw_header(p, fonts, m, v);
    draw_rows(p, fonts, m, v);
    draw_edge(p, m);
}

/// One-pixel outline so the panel separates from whatever is behind it.
fn draw_edge(p: &Painter, m: Metrics) {
    let c = theme::with_alpha(theme::BORDER, 0xE0);
    let t = m.scale.max(1.0);
    p.fill_rect(0.0, 0.0, m.width, t, c);
    p.fill_rect(0.0, m.height - t, m.width, t, c);
    p.fill_rect(0.0, 0.0, t, m.height, c);
    p.fill_rect(m.width - t, 0.0, t, m.height, c);
}

fn draw_header(p: &Painter, fonts: &theme::Fonts, m: Metrics, v: &ListView) {
    let s = m.scale;
    let y = 6.0 * s;

    // Right side: active count and total, split by a separator.
    let active = v.rows.iter().filter(|r| r.status.is_active()).count();
    let right = m.width - m.pad() - 2.0;

    let active_str = active.to_string();
    let gray_text = if active > 0 {
        format!(" / {}", v.total)
    } else {
        v.total.to_string()
    };
    let aw = p.dual_measure(&fonts.small, &active_str);
    let gw = p.dual_measure(&fonts.small, &gray_text);
    let right_w = if active > 0 { aw + gw } else { gw };

    // Lock icon when window position is pinned.
    let sub_x = if v.pinned {
        let lx = m.pad() + 2.0;
        let ly = y + 4.0 * s;
        let c = theme::TEXT_DIM;
        let bw = 7.0 * s;
        // Shackle (outlined arc on top).
        p.round_rect(lx + 1.5 * s, ly, 4.0 * s, 5.0 * s, 2.0 * s, c, false);
        // Body (filled rounded rect).
        p.round_rect(lx, ly + 3.0 * s, bw, 5.0 * s, 1.5 * s, c, true);
        m.pad() + 2.0 + bw + 6.0 * s
    } else {
        m.pad() + 2.0
    };
    let sub_right = right - right_w - 8.0 * s;
    let sub_w = (sub_right - sub_x).max(0.0);

    let mut parts: Vec<String> = Vec::new();
    if let Some(user) = v.user {
        parts.push(user.to_string());
    }
    parts.push(v.rows.len().to_string());
    // Without a footer, the status line shares the subtitle. A failure takes
    // the line over entirely, since it matters more than the usual detail.
    let (text, color) = match &v.status_text {
        Some((message, true)) => (message.clone(), theme::RED),
        Some((message, false)) => (
            format!("{}  ·  {}", parts.join("  ·  "), message),
            theme::GREEN,
        ),
        None => (parts.join("  ·  "), theme::TEXT_FAINT),
    };
    p.dual_text(
        &fonts.small,
        &text,
        sub_x,
        y,
        sub_w,
        18.0 * s,
        color,
        theme::ALIGN_NEAR,
    );

    if active > 0 {
        p.dual_text(
            &fonts.small,
            &active_str,
            right - right_w,
            y,
            aw,
            18.0 * s,
            theme::BLUE,
            theme::ALIGN_NEAR,
        );
    }
    p.dual_text(
        &fonts.small,
        &gray_text,
        right - gw,
        y,
        gw,
        18.0 * s,
        theme::TEXT_DIM,
        theme::ALIGN_NEAR,
    );

    p.fill_rect(0.0, m.header_h - 1.0, m.width, 1.0, theme::SEPARATOR);
}

fn draw_rows(p: &Painter, fonts: &theme::Fonts, m: Metrics, v: &ListView) {
    if v.rows.is_empty() {
        p.dual_text(
            &fonts.body,
            "等待数据…",
            0.0,
            m.rows_top() + m.rows_viewport_h() / 2.0 - 10.0 * m.scale,
            m.width,
            20.0 * m.scale,
            theme::TEXT_FAINT,
            theme::ALIGN_CENTER,
        );
        return;
    }

    // Clip so cards scroll inside the list area rather than over the chrome.
    let area = m.rows_area();
    p.clip(RectF {
        X: 0.0,
        Y: area.y,
        Width: area.w,
        Height: area.h,
    });

    for index in 0..v.rows.len() {
        let rect = m.card_rect(index, v.scroll);
        if rect.y + rect.h < area.y || rect.y > area.y + area.h {
            continue;
        }
        draw_card(p, fonts, &v.rows[index], rect, v, index, m);
    }
    p.reset_clip();
}

fn draw_card(
    p: &Painter,
    fonts: &theme::Fonts,
    row: &Row,
    rect: Rect,
    v: &ListView,
    index: usize,
    m: Metrics,
) {
    let c = CardMetrics::new(m);
    let highlighted = v.hover == Some(index) || v.selected == Some(index);
    let fill = if highlighted {
        theme::CARD_HOVER
    } else {
        theme::CARD
    };
    p.round_rect(rect.x, rect.y, rect.w, rect.h, c.radius, fill, true);

    // Status is encoded by colour alone: this bar is the only state indicator,
    // so it is wide enough to read at a glance and inset from the card edge.
    p.round_rect(
        rect.x + m.scale,
        rect.y + c.accent_inset,
        c.accent_w,
        rect.h - c.accent_inset * 2.0,
        c.accent_w / 2.0,
        theme::status_color(row.status),
        true,
    );

    let text_x = rect.x + c.inner_pad;
    let mut y = rect.y + c.accent_inset;

    // --- Line 1: marks, caller, channel, time(right) ---
    let line1_y = y;
    let line1_gap = 6.0 * m.scale;
    let mut x = text_x;
    let mark_gap = 2.0 * m.scale;

    {
        let w = p.dual_measure(&fonts.body, "流");
        p.dual_text(
            &fonts.body,
            "流",
            x,
            line1_y,
            w,
            c.line1,
            if row.stream {
                theme::WHITE
            } else {
                theme::TEXT_FAINT
            },
            theme::ALIGN_NEAR,
        );
        x += w + mark_gap;
    }

    // Protocol conversion mark: 转 highlighted when no conversion.
    {
        let w = p.dual_measure(&fonts.body, "转");
        p.dual_text(
            &fonts.body,
            "转",
            x,
            line1_y,
            w,
            c.line1,
            if row.protocol().is_converted() {
                theme::TEXT_FAINT
            } else {
                theme::WHITE
            },
            theme::ALIGN_NEAR,
        );
        x += w + mark_gap;
    }

    // Pass-through mark: 透 highlighted when applied.
    {
        let w = p.dual_measure(&fonts.body, "透");
        p.dual_text(
            &fonts.body,
            "透",
            x,
            line1_y,
            w,
            c.line1,
            if row.pass_through {
                theme::WHITE
            } else {
                theme::TEXT_FAINT
            },
            theme::ALIGN_NEAR,
        );
        x += w + line1_gap;
    }

    // Time: right-aligned on line 1.
    let time_text = fmt::relative_time(row.created_at.as_deref(), v.now);
    let time_w = p.dual_measure(&fonts.body, &time_text) + 4.0;
    p.dual_text(
        &fonts.body,
        &time_text,
        rect.x + rect.w - c.inner_pad - time_w,
        line1_y,
        time_w,
        c.line1,
        theme::TEXT_FAINT,
        theme::ALIGN_FAR,
    );

    // Remaining space between marks and time.
    let badge_limit = rect.x + rect.w - c.inner_pad - time_w - 4.0 * m.scale;

    // Caller (API key name).
    if let Some(caller) = &row.caller {
        let w = p.dual_measure(&fonts.small, caller);
        if x + w < badge_limit {
            p.dual_text(
                &fonts.small,
                caller,
                x,
                line1_y,
                w + 2.0 * m.scale,
                c.line1,
                theme::TEXT_FAINT,
                theme::ALIGN_NEAR,
            );
            x += w + 5.0 * m.scale;
        }
    }

    // Channel.
    if let Some(channel) = &row.channel {
        let w = p.dual_measure(&fonts.small, channel);
        if x + w < badge_limit {
            p.dual_text(
                &fonts.small,
                channel,
                x,
                line1_y,
                w + 2.0 * m.scale,
                c.line1,
                theme::TEXT_DIM,
                theme::ALIGN_NEAR,
            );
            x += w + 5.0 * m.scale;
        }
    }

    // Tokens.
    if row.total_tokens > 0 {
        let tokens = fmt::tokens_compact(row.total_tokens);
        let w = p.dual_measure(&fonts.small, &tokens);
        if x + w < badge_limit {
            p.dual_text(
                &fonts.small,
                &tokens,
                x,
                line1_y,
                w,
                c.line1,
                theme::TEXT_DIM,
                theme::ALIGN_NEAR,
            );
        }
    }

    y += c.line1 + c.gap;

    // --- Line 2: model, cache, tokens, TPS, retry ---
    // Right-aligned items are placed first so the left cluster can be clipped
    // to whatever space remains.
    let mut right_edge = rect.x + rect.w - c.inner_pad;

    // TPS (tokens per second).
    if let Some(tps) = row.tps() {
        let label = format!("{tps:.0} tok/s");
        let w = p.dual_measure(&fonts.small, &label);
        p.dual_text(
            &fonts.small,
            &label,
            right_edge - w - 2.0 * m.scale,
            y,
            w + 4.0 * m.scale,
            c.line2,
            theme::TEXT_DIM,
            theme::ALIGN_FAR,
        );
        right_edge -= w + 10.0 * m.scale;
    }

    // Retry badge.
    if row.attempt_count > 1 {
        let label = format!("{}×", row.attempt_count - 1);
        let w = p.dual_measure(&fonts.small, &label) + 6.0;
        p.dual_text(
            &fonts.small,
            &label,
            right_edge - w,
            y,
            w,
            c.line2,
            theme::RED,
            theme::ALIGN_FAR,
        );
        right_edge -= w + 10.0 * m.scale;
    }

    // Left-aligned items: model first, then metrics advancing past each cell.
    let line2_gap = 6.0 * m.scale;
    let mut x = text_x;

    // Model name with reasoning effort, e.g. "glm-5.2(max)". Drawn on a soft
    // chip so the served model stands apart from the surrounding metrics.
    let model_label = row.served_model();
    let display = match &row.reasoning_effort {
        Some(effort) => format!("{}({})", model_label, effort),
        None => model_label.to_string(),
    };
    let chip_hpad = 4.0 * m.scale;
    let chip_vpad = 2.0 * m.scale;
    let chip_w =
        (p.dual_measure(&fonts.small, &display) + chip_hpad * 2.0).min((right_edge - x).max(0.0));
    let text_w = (chip_w - chip_hpad * 2.0).max(0.0);
    let chip_h = c.line2 + chip_vpad * 2.0;
    p.round_rect(
        x,
        y - chip_vpad,
        chip_w,
        chip_h,
        3.0 * m.scale,
        theme::BORDER,
        true,
    );
    p.dual_text(
        &fonts.small,
        &display,
        x + chip_hpad,
        y,
        text_w,
        c.line2,
        if row.is_routed() {
            theme::GOLD
        } else {
            theme::TEXT
        },
        theme::ALIGN_NEAR,
    );
    x += chip_w + line2_gap;

    // Cache hit rate.
    if let Some(rate) = row.cache_hit_rate() {
        let label = format!("{rate:.2}%");
        let w = p.dual_measure(&fonts.small, &label);
        if x + w < right_edge {
            p.dual_text(
                &fonts.small,
                &label,
                x,
                y,
                w,
                c.line2,
                if row.cache_hit_is_low() {
                    theme::RED
                } else {
                    theme::GREEN
                },
                theme::ALIGN_NEAR,
            );
        }
    }
}

/// Hit-test a point in viewport space.
pub fn hit_row(m: Metrics, v: &ListView, x: f32, y: f32) -> Option<usize> {
    if x < m.pad() || x > m.width - m.pad() {
        return None;
    }
    m.row_at(y, v.scroll, v.rows.len())
}
