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
    /// Heights of small pill-shaped chips (reasoning effort).
    chip_h: f32,
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
            chip_h: 13.0 * s,
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
    pub last_refresh: Option<&'a str>,
    pub paused: bool,
}

pub fn draw(p: &Painter, fonts: &theme::Fonts, m: Metrics, v: &ListView) {
    p.fill_rect(0.0, 0.0, m.width, m.height, theme::BG);
    draw_header(p, fonts, m, v);
    draw_rows(p, fonts, m, v);
    draw_scrollbar(p, m, v);
    draw_edge(p, m);
}

/// Thin scroll indicator: the panel is taller than a screen can show whole, so
/// the position within the full result set needs to be visible.
fn draw_scrollbar(p: &Painter, m: Metrics, v: &ListView) {
    let max = m.max_scroll(v.rows.len());
    if max <= 0.0 {
        return;
    }
    let s = m.scale;
    let track_y = m.rows_top() + 2.0 * s;
    let track_h = m.rows_viewport_h() - 4.0 * s;
    if track_h <= 8.0 * s {
        return;
    }
    let total = m.content_h(v.rows.len());
    let thumb_h = (track_h * m.rows_viewport_h() / total).max(24.0 * s);
    let progress = (v.scroll / max).clamp(0.0, 1.0);
    let thumb_y = track_y + (track_h - thumb_h) * progress;
    p.round_rect(
        m.width - 5.0 * s,
        thumb_y,
        3.0 * s,
        thumb_h,
        1.5 * s,
        theme::with_alpha(theme::TEXT_DIM, 0x9A),
        true,
    );
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

    // Right side: active count and total.
    let active = v.rows.iter().filter(|r| r.status.is_active()).count();
    let right = m.width - m.pad() - 2.0;

    let total_text = format!("共 {}", v.total);
    p.dual_text(
        &fonts.small,
        &total_text,
        right - 56.0 * s,
        y,
        56.0 * s,
        18.0 * s,
        theme::TEXT_DIM,
        theme::ALIGN_FAR,
    );

    if active > 0 {
        let dot_x = right - 118.0 * s;
        round_dot(p, dot_x, y + 6.5 * s, 5.0 * s, theme::BLUE);
        p.dual_text(
            &fonts.small,
            &format!("{active} 进行中"),
            dot_x + 9.0 * s,
            y,
            52.0 * s,
            18.0 * s,
            theme::BLUE,
            theme::ALIGN_NEAR,
        );
    }

    // Left side: identity and freshness.
    let sub_x = m.pad() + 2.0;
    let sub_right = if active > 0 {
        right - 128.0 * s
    } else {
        right - 62.0 * s
    };
    let sub_w = (sub_right - sub_x).max(0.0);

    let mut parts: Vec<String> = Vec::new();
    if let Some(user) = v.user {
        parts.push(user.to_string());
    }
    if v.paused {
        parts.push("已暂停".into());
    }
    parts.push(format!("显示前 {} 条", v.rows.len()));
    // Without a footer, the status line shares the subtitle. A failure takes
    // the line over entirely, since it matters more than the usual detail.
    let (text, color) = match &v.status_text {
        Some((message, true)) => (message.clone(), theme::RED),
        Some((message, false)) => (format!("{}  ·  {}", parts.join("  ·  "), message), theme::GREEN),
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

    // --- Line 1: number, stream mark, badges, time(right) ---
    let line1_y = y;
    let line1_gap = 6.0 * m.scale;
    let mut x = text_x;

    let number_text = format!("#{}", row.number);
    let number_w = p.dual_measure(&fonts.body, &number_text);
    p.dual_text(
        &fonts.body,
        &number_text,
        x,
        line1_y,
        number_w,
        c.line1,
        theme::TEXT_FAINT,
        theme::ALIGN_NEAR,
    );
    x += number_w + line1_gap;

    if row.stream {
        let w = p.dual_measure(&fonts.body, "流");
        p.dual_text(
            &fonts.body,
            "流",
            x,
            line1_y,
            w,
            c.line1,
            theme::BROWN,
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

    // Badges between the stream marker and the time.
    let badge_limit = rect.x + rect.w - c.inner_pad - time_w - 4.0 * m.scale;

    // Reasoning effort chip.
    if let Some(effort) = &row.reasoning_effort {
        let w = p.dual_measure(&fonts.small, effort) + 12.0 * m.scale;
        if x + w < badge_limit {
            p.round_rect(x, line1_y + 2.0 * m.scale, w, c.chip_h, c.chip_h / 2.0, 0xFF1D2635, true);
            p.dual_text(
                &fonts.small,
                effort,
                x,
                line1_y + 2.0 * m.scale,
                w,
                c.chip_h,
                0xFF7FB2FF,
                theme::ALIGN_CENTER,
            );
            x += w + 5.0 * m.scale;
        }
    }

    // Protocol cell. Amber marks an in-flight conversion.
    let protocol = row.protocol();
    if let Some(label) = protocol.label() {
        let w = p.dual_measure(&fonts.small, &label);
        if x + w < badge_limit {
            p.dual_text(
                &fonts.small,
                &label,
                x,
                line1_y,
                w + 2.0 * m.scale,
                c.line1,
                if protocol.is_converted() {
                    theme::GOLD
                } else {
                    theme::TEXT_FAINT
                },
                theme::ALIGN_NEAR,
            );
            x += w + 5.0 * m.scale;
        }
    }

    // Pass-through badge: only drawn when it is on.
    if row.pass_through && x + 42.0 * m.scale < badge_limit {
        let label = "透传";
        let w = p.dual_measure(&fonts.small, label) + 12.0 * m.scale;
        p.round_rect(
            x,
            line1_y + 2.0 * m.scale,
            w,
            c.chip_h,
            c.chip_h / 2.0,
            0xFF2C2418,
            true,
        );
        p.dual_text(
            &fonts.small,
            label,
            x,
            line1_y + 2.0 * m.scale,
            w,
            c.chip_h,
            theme::GOLD,
            theme::ALIGN_CENTER,
        );
    }

    y += c.line1 + c.gap;

    // --- Line 2: model, channel, cache, tokens, TPS, retry, caller ---
    // Right-aligned items are placed first so the left cluster can be clipped
    // to whatever space remains.
    let mut right_edge = rect.x + rect.w - c.inner_pad;

    // Caller (far right).
    if let Some(caller) = &row.caller {
        let w = p.dual_measure(&fonts.small, caller).min(130.0 * m.scale);
        p.dual_text(
            &fonts.small,
            caller,
            right_edge - w,
            y,
            w + 2.0 * m.scale,
            c.line2,
            theme::TEXT_FAINT,
            theme::ALIGN_FAR,
        );
        right_edge -= w + 10.0 * m.scale;
    }

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
        let label = format!("{} 次重试", row.attempt_count - 1);
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

    // Model name.
    let model_label = row.served_model();
    let model_w = p.dual_measure(&fonts.small, model_label).min((right_edge - x).max(0.0));
    p.dual_text(
        &fonts.small,
        model_label,
        x,
        y,
        model_w,
        c.line2,
        if row.is_routed() {
            theme::GOLD
        } else {
            theme::TEXT
        },
        theme::ALIGN_NEAR,
    );
    x += model_w + line2_gap;

    // Channel.
    if let Some(channel) = &row.channel {
        let w = p.dual_measure(&fonts.small, channel);
        if x + w < right_edge {
            p.dual_text(
                &fonts.small,
                channel,
                x,
                y,
                w,
                c.line2,
                theme::TEXT_DIM,
                theme::ALIGN_NEAR,
            );
            x += w + line2_gap;
        }
    }

    // Cache hit rate.
    if let Some(rate) = row.cache_hit_rate() {
        let label = format!("缓 {rate:.2}%");
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
            x += w + line2_gap;
        }
    }

    // Tokens.
    if row.total_tokens > 0 {
        let tokens = fmt::tokens_compact(row.total_tokens);
        let w = p.dual_measure(&fonts.small, &tokens);
        if x + w < right_edge {
            p.dual_text(
                &fonts.small,
                &tokens,
                x,
                y,
                w,
                c.line2,
                theme::TEXT_DIM,
                theme::ALIGN_NEAR,
            );
        }
    }
}

fn round_dot(p: &Painter, x: f32, y: f32, d: f32, color: u32) {
    p.round_rect(x, y, d, d, d / 2.0, color, true);
}

/// Hit-test a point in viewport space.
pub fn hit_row(m: Metrics, v: &ListView, x: f32, y: f32) -> Option<usize> {
    if x < m.pad() || x > m.width - m.pad() {
        return None;
    }
    m.row_at(y, v.scroll, v.rows.len())
}
