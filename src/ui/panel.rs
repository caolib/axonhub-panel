//! Request card rendering.
//!
//! Mirrors the AxonHub requests page information hierarchy — identity/status,
//! model and routing, then channel, tokens, cache, latency and cost — but laid
//! out as cards rather than a table so it reads well at panel width. Everything
//! the console puts behind a tooltip is shown inline: the panel has room
//! because it is not trying to be a dense, sortable, filterable grid.

use windows::Win32::Graphics::GdiPlus::RectF;

use crate::format as fmt;
use crate::model::{Row, Status};
use crate::theme::{self, Painter};
use crate::ui::layout::{Metrics, Rect};

/// Authored at 96 DPI; multiplied by `Metrics::scale` at draw time.
const INNER_PAD: f32 = 11.0;
const LINE1_H: f32 = 18.0;
const LINE2_H: f32 = 17.0;
const LINE3_H: f32 = 15.0;
const CARD_RADIUS: f32 = 7.0;
const ACCENT_W: f32 = 4.0;
const ACCENT_INSET: f32 = 7.0;

/// Column x-offsets for the metric row, measured from the card's inner edge.
///
/// Fixed offsets rather than "advance past the previous cell": with variable
/// width neighbours (a long channel name, a card with no cache figure), packing
/// text end-to-end makes every column start at a different x on every card.
/// Reserving a slot per column keeps them aligned down the list.
#[derive(Clone, Copy)]
struct Columns {
    channel: f32,
    cache: f32,
    tokens: f32,
}

impl Columns {
    /// Slot boundaries in characters, converted to pixels using the measured
    /// advance width of the row's font. Since every family used here is
    /// monospaced, a character grid is the natural unit: it makes the slots
    /// exact and keeps the gaps even regardless of DPI.
    fn new(p: &Painter, font: &theme::DualFont) -> Self {
        // Advance width of one character, measured from the actual Latin face
        // rather than assumed, so the grid matches the rendered font exactly.
        let advance = p.dual_measure(font, "0000") / 4.0;
        let tab = |chars: f32| chars * advance;
        Columns {
            // Channel takes 17 characters (enough for `ag-caolib-cc`), then a
            // two-character gap before the cache slot.
            channel: 0.0,
            cache: tab(18.0),
            tokens: tab(28.0),
        }
    }
}

/// The card's internal measurements for one draw, already scaled to the monitor.
#[derive(Clone, Copy)]
struct CardMetrics {
    inner_pad: f32,
    line1: f32,
    line2: f32,
    line3: f32,
    radius: f32,
    accent_w: f32,
    accent_inset: f32,
    /// Heights of small pill-shaped chips (reasoning effort).
    chip_h: f32,
    /// The status accent bar's vertical inset.
    gap: f32,
}

impl CardMetrics {
    fn new(m: Metrics) -> Self {
        let s = m.scale;
        CardMetrics {
            inner_pad: INNER_PAD * s,
            line1: LINE1_H * s,
            line2: LINE2_H * s,
            line3: LINE3_H * s,
            radius: CARD_RADIUS * s,
            accent_w: ACCENT_W * s,
            accent_inset: ACCENT_INSET * s,
            chip_h: 14.0 * s,
            gap: 2.0 * s,
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
    let title_y = 8.0 * s;
    p.dual_text(
        &fonts.bold,
        "AxonHub",
        m.pad() + 2.0,
        title_y,
        150.0 * s,
        22.0 * s,
        theme::TEXT,
        theme::ALIGN_NEAR,
    );

    // Right side answers "what is happening right now".
    let active = v.rows.iter().filter(|r| r.status.is_active()).count();
    let right = m.width - m.pad() - 2.0;

    let total_text = format!("共 {}", v.total);
    p.dual_text(
        &fonts.small,
        &total_text,
        right - 90.0 * s,
        title_y + 3.0 * s,
        90.0 * s,
        18.0 * s,
        theme::TEXT_DIM,
        theme::ALIGN_FAR,
    );

    if active > 0 {
        let dot_x = right - 158.0 * s;
        round_dot(p, dot_x, title_y + 9.0 * s, 6.0 * s, theme::BLUE);
        p.dual_text(
            &fonts.small,
            &format!("{active} 进行中"),
            dot_x + 11.0 * s,
            title_y + 1.0 * s,
            76.0 * s,
            18.0 * s,
            theme::BLUE,
            theme::ALIGN_NEAR,
        );
    }

    // Second line: identity and freshness.
    let mut parts: Vec<String> = Vec::new();
    if let Some(user) = v.user {
        parts.push(user.to_string());
    }
    if let Some(t) = v.last_refresh {
        parts.push(format!("更新于 {t}"));
    }
    if v.paused {
        parts.push("已暂停".into());
    }
    parts.push(format!("显示前 {} 条", v.rows.len()));
    // Without a footer, the status line shares the subtitle row. A failure takes
    // the row over entirely, since it matters more than the usual detail.
    let (text, color) = match &v.status_text {
        Some((message, true)) => (message.clone(), theme::RED),
        Some((message, false)) => (format!("{}  ·  {}", parts.join("  ·  "), message), theme::GREEN),
        None => (parts.join("  ·  "), theme::TEXT_FAINT),
    };
    p.dual_text(
        &fonts.small,
        &text,
        m.pad() + 2.0,
        30.0 * m.scale,
        m.width - (m.pad() + 2.0) * 2.0,
        18.0 * m.scale,
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
    let text_w = rect.w - c.inner_pad * 2.0;
    let mut y = rect.y + c.accent_inset;

    // --- line 1: request number, mark, time, cost ---
    // Every cell on this line uses one font and the same rectangle, so their
    // baselines agree; per-cell fonts and offsets made them sit at heights that
    // differed by a pixel or two.
    let line1_y = y;
    let number_text = format!("#{}", row.number);
    // Fixed slot: the clock must not shift with the width of the id.
    let number_slot = 74.0 * m.scale;
    p.dual_text(
        &fonts.body,
        &number_text,
        text_x,
        line1_y,
        number_slot,
        c.line1,
        theme::TEXT_FAINT,
        theme::ALIGN_NEAR,
    );

    // `流式` is a status marker, not a metric, so it keeps the id's baseline.
    if row.stream {
        p.dual_text(
            &fonts.body,
            "流式",
            text_x + number_slot,
            line1_y,
            34.0 * m.scale,
            c.line1,
            theme::BROWN,
            theme::ALIGN_NEAR,
        );
    }

    p.dual_text(
        &fonts.body,
        &fmt::relative_time(row.created_at.as_deref(), v.now),
        text_x + number_slot + 34.0 * m.scale,
        line1_y,
        92.0 * m.scale,
        c.line1,
        theme::TEXT_FAINT,
        theme::ALIGN_NEAR,
    );

    if let Some(cost) = row.cost {
        let cost_text = fmt::cost(Some(cost));
        let w = p.dual_measure(&fonts.body, &cost_text) + 4.0;
        p.dual_text(
            &fonts.body,
            &cost_text,
            rect.x + rect.w - c.inner_pad - w,
            y,
            w,
            c.line1,
            if cost >= 0.1 { theme::YELLOW } else { theme::TEXT_DIM },
            theme::ALIGN_FAR,
        );
    }
    y += c.line1 + c.gap;

    // --- line 2: model, reasoning effort, protocol ---
    let mut x = text_x;
    // Show only the model that served the request; gold marks the case where
    // that differs from what was asked for.
    let model_label = row.served_model();
    let model_w = p.dual_measure(&fonts.body, model_label).min(text_w * 0.62);
    p.dual_text(
        &fonts.body,
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
    x += model_w + 7.0 * m.scale;

    if let Some(effort) = &row.reasoning_effort {
        let w = p.dual_measure(&fonts.small, effort) + 14.0 * m.scale;
        if x + w < rect.x + rect.w - c.inner_pad {
            p.round_rect(x, y + 1.5 * m.scale, w, c.chip_h, c.chip_h / 2.0, 0xFF1D2635, true);
            p.dual_text(
                &fonts.small,
                effort,
                x,
                y + 1.5 * m.scale,
                w,
                c.chip_h,
                0xFF7FB2FF,
                theme::ALIGN_CENTER,
            );
            x += w + 7.0 * m.scale;
        }
    }

    // Protocol cell. Amber marks an in-flight conversion, the only case worth
    // drawing attention to; a match stays muted.
    let protocol = row.protocol();
    if let Some(label) = protocol.label() {
        let w = p.dual_measure(&fonts.small, &label);
        if x + w < rect.x + rect.w - c.inner_pad {
            p.dual_text(
                &fonts.small,
                &label,
                x,
                y + 1.0,
                w + 2.0 * m.scale,
                c.line2,
                if protocol.is_converted() {
                    theme::GOLD
                } else {
                    theme::TEXT_FAINT
                },
                theme::ALIGN_NEAR,
            );
            x += w + 7.0 * m.scale;
        }
    }

    // Pass-through badge: only drawn when it is on, since "not applied" is the
    // normal case and a marker for it would be noise.
    if row.pass_through && x + 42.0 * m.scale < rect.x + rect.w - c.inner_pad {
        let label = "透传";
        let w = p.dual_measure(&fonts.small, label) + 12.0 * m.scale;
        p.round_rect(
            x,
            y + 1.5 * m.scale,
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
            y + 1.5 * m.scale,
            w,
            c.chip_h,
            theme::GOLD,
            theme::ALIGN_CENTER,
        );
    }

    // Retry badge right-aligned when AxonHub retried upstream.
    if row.attempt_count > 1 {
        let label = format!("{} 次重试", row.attempt_count - 1);
        let w = p.dual_measure(&fonts.small, &label) + 6.0;
        p.dual_text(
            &fonts.small,
            &label,
            rect.x + rect.w - c.inner_pad - w,
            y + 1.0,
            w,
            c.line2,
            theme::RED,
            theme::ALIGN_FAR,
        );
    }
    y += c.line2;

    // --- line 3: channel, cache, tokens, latency, caller ---
    // Right-aligned items are placed first so the left cluster can be clipped
    // to whatever space remains. Every left-hand cell starts at a fixed column
    // so the row reads as a table rather than as drifting text.
    let mut right_edge = rect.x + rect.w - c.inner_pad;

    // Caller sits at the far right: it identifies the source rather than
    // measuring the request, so it is kept out of the metrics cluster.
    if let Some(caller) = &row.caller {
        let w = p.dual_measure(&fonts.small, caller).min(140.0 * m.scale);
        p.dual_text(
            &fonts.small,
            caller,
            right_edge - w,
            y,
            w + 2.0 * m.scale,
            c.line3,
            theme::TEXT_FAINT,
            theme::ALIGN_FAR,
        );
        right_edge -= w + 10.0 * m.scale;
    }
    if row.status == Status::Completed && row.latency_ms.is_some() {
        let latency = format!(
            "{} / {}",
            fmt::duration(row.latency_ms),
            fmt::duration(row.first_token_ms)
        );
        let w = p.dual_measure(&fonts.small, &latency);
        p.dual_text(
            &fonts.small,
            &latency,
            right_edge - w - 2.0 * m.scale,
            y,
            w + 4.0 * m.scale,
            c.line3,
            theme::TEXT_DIM,
            theme::ALIGN_FAR,
        );
        right_edge -= w + 12.0 * m.scale;
    }

    let cols = Columns::new(p, &fonts.small);
    let slot = |offset: f32| text_x + offset;
    // One character of slack between slots, so neighbouring cells never touch.
    let gap = p.dual_measure(&fonts.small, "0");

    // Channel: a fixed slot, truncated rather than allowed to push its
    // neighbours along.
    if let Some(channel) = &row.channel {
        p.dual_text(
            &fonts.small,
            channel,
            slot(cols.channel),
            y,
            (cols.cache - cols.channel) - gap,
            c.line3,
            theme::TEXT_DIM,
            theme::ALIGN_NEAR,
        );
    }

    // Cache hit rate is only meaningful, and only shown, when caching occurred.
    if let Some(rate) = row.cache_hit_rate() {
        let label = format!("缓存 {rate:.2}%");
        p.dual_text(
            &fonts.small,
            &label,
            slot(cols.cache),
            y,
            (cols.tokens - cols.cache) - gap,
            c.line3,
            if row.cache_hit_is_low() {
                theme::RED
            } else {
                theme::GREEN
            },
            theme::ALIGN_NEAR,
        );
    }

    if row.total_tokens > 0 {
        let tokens = fmt::tokens_compact(row.total_tokens);
        p.dual_text(
            &fonts.small,
            &tokens,
            slot(cols.tokens),
            y,
            (right_edge - slot(cols.tokens)).max(0.0),
            c.line3,
            theme::TEXT_DIM,
            theme::ALIGN_NEAR,
        );
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
