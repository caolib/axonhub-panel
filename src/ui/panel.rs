//! Request card rendering.
//!
//! Mirrors the AxonHub requests page information hierarchy — identity/status,
//! model and routing, then channel, tokens, cache, latency and cost — but laid
//! out as cards rather than a table so it reads well at panel width. Everything
//! the console puts behind a tooltip is shown inline: the panel has room
//! because it is not trying to be a dense, sortable, filterable grid.

use windows::Win32::Graphics::GdiPlus::RectF;

use crate::config::{Account, DisplayField};
use crate::format as fmt;
use crate::model::{self, Filter, FilterMask, Row};
use crate::theme::{self, Painter};
use crate::ui::layout::{Metrics, Rect};

/// Colours that mark which account a card came from, chosen by position in the
/// account list so an account keeps its colour across repaints. Deliberately
/// none of the status colours, which carry meaning of their own.
const ACCOUNT_COLORS: [u32; 4] = [theme::MAUVE, theme::ORANGE, theme::CYAN, 0xFFB0_B8C4];

/// The colour of one account's tag.
fn account_color(accounts: &[Account], id: &str) -> u32 {
    accounts
        .iter()
        .position(|a| a.id == id)
        .map(|i| ACCOUNT_COLORS[i % ACCOUNT_COLORS.len()])
        .unwrap_or(theme::TEXT_DIM)
}

/// Authored at 96 DPI; multiplied by `Metrics::scale` at draw time.
const INNER_PAD: f32 = 7.0;
const LINE1_H: f32 = 17.0;
const LINE2_H: f32 = 14.0;
const CARD_RADIUS: f32 = 5.0;
const ACCENT_W: f32 = 4.0;
const ACCENT_INSET: f32 = 5.0;
/// Header filter chips. The width is fixed (widest label 进行中 plus padding),
/// so click hit-testing needs no font measurement.
const FILTER_W: f32 = 46.0;
const FILTER_H: f32 = 20.0;
const FILTER_GAP: f32 = 4.0;
/// Chip order on the header, left to right.
const FILTERS: [Filter; 4] = [
    Filter::All,
    Filter::Completed,
    Filter::Failed,
    Filter::Active,
];

/// Colour palette for channel and model names, shared by both so the two kinds
/// of label are drawn from the same set of colours. The first entry is the
/// default light blue; later entries distinguish other names on the page.
const NAME_PALETTE: [u32; 9] = [
    theme::CYAN,   // 0: light blue (default)
    theme::ORANGE, // 1: orange
    0xFFFF_6BC4,   // 2: hot pink
    0xFF4F_E0C0,   // 3: turquoise
    0xFFB9_8CFF,   // 4: purple
    0xFFFF_B454,   // 5: amber
    0xFF8D_E85B,   // 6: lime
    0xFFE0_6BFF,   // 7: magenta
    0xFFFF_6B6B,   // 8: coral
];

/// Hand the palette out to `names`, which are listed newest first, so the
/// assignment runs the other way: the oldest name on the page keeps the default
/// light blue. New rows arrive at the top, and a name already on the page keeps
/// its colour when one appears above it.
fn rank_palette(names: &mut [(&str, u32)]) {
    let count = names.len();
    for (i, (_, color)) in names.iter_mut().enumerate() {
        *color = NAME_PALETTE[(count - 1 - i) % NAME_PALETTE.len()];
    }
}

/// Map each distinct channel in `rows` (newest first) to a palette colour.
fn channel_palette(rows: &[Row]) -> Vec<(&str, u32)> {
    let mut map: Vec<(&str, u32)> = Vec::new();
    for row in rows {
        if let Some(ch) = &row.channel {
            if !map.iter().any(|(n, _)| *n == ch.as_str()) {
                map.push((ch.as_str(), 0));
            }
        }
    }
    rank_palette(&mut map);
    map
}

/// Map each distinct served model in `rows` (newest first) to a palette colour.
fn model_palette(rows: &[Row]) -> Vec<(&str, u32)> {
    let mut map: Vec<(&str, u32)> = Vec::new();
    for row in rows {
        let model = row.served_model();
        if !map.iter().any(|(n, _)| *n == model) {
            map.push((model, 0));
        }
    }
    rank_palette(&mut map);
    map
}

/// The card's internal measurements for one draw, already scaled to the monitor.
#[derive(Clone, Copy)]
struct CardMetrics {
    inner_pad: f32,
    line1: f32,
    line2: f32,
    radius: f32,
    accent_w: f32,
    accent_inset: f32,
    /// The gap between the two text lines (two-line layout only).
    gap: f32,
    /// Horizontal padding inside the model chip.
    chip_hpad: f32,
    /// Vertical padding of the model chip around its text line.
    chip_vpad: f32,
    /// Single-line layout: one content line with the chip defining its height.
    single: bool,
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
            // A single-line card gives the accent bar less inset, or the bar
            // would be shorter than the text it flanks.
            accent_inset: (if m.single_line { 4.0 } else { ACCENT_INSET }) * s,
            gap: 3.0 * s,
            chip_hpad: 4.0 * s,
            chip_vpad: 2.0 * s,
            single: m.single_line,
        }
    }

    /// Total height of the model chip.
    fn chip_h(&self) -> f32 {
        self.line2 + self.chip_vpad * 2.0
    }

    /// The vertical band a card's content must fit into.
    fn band_h(&self, card_h: f32) -> f32 {
        card_h - self.accent_inset * 2.0
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
    /// Saved accounts; more than one means every card is tagged with its own.
    pub accounts: &'a [Account],
    /// An account is waiting on a fresh sign-in: the settings button turns red,
    /// since re-signing in is what fixes it.
    pub settings_alert: bool,
    pub pinned: bool,
    /// Selected status-filter bits; the header chips highlight against this.
    pub filter: FilterMask,
    /// Card cells the user switched off; everything else is drawn.
    pub hidden_fields: &'a [DisplayField],
}

impl ListView<'_> {
    /// Whether one card cell should be drawn.
    pub fn shows(&self, field: DisplayField) -> bool {
        !self.hidden_fields.contains(&field)
    }
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

pub fn settings_button(m: Metrics) -> Rect {
    Rect {
        x: m.width - m.pad() - 40.0 * m.scale,
        y: (m.header_h - 20.0 * m.scale) / 2.0,
        w: 40.0 * m.scale,
        h: 20.0 * m.scale,
    }
}

fn draw_header(p: &Painter, fonts: &theme::Fonts, m: Metrics, v: &ListView) {
    let s = m.scale;
    let y = 6.0 * s;

    // Right side: active count and total, split by a separator. While a
    // status filter is active the left number is the visible subset instead.
    let active = v.rows.iter().filter(|r| r.status.is_active()).count();
    let blue_num = if v.filter == model::FILTER_NONE {
        active
    } else {
        v.rows.len()
    };
    let settings = settings_button(m);
    // A red pill is the panel's only hint that an account needs attention, so
    // it stays red until the account signs in again.
    let (settings_fill, settings_text) = if v.settings_alert {
        (theme::with_alpha(theme::RED, 0x38), theme::RED)
    } else {
        (theme::CARD_HOVER, theme::TEXT_DIM)
    };
    p.round_rect(
        settings.x,
        settings.y,
        settings.w,
        settings.h,
        3.0 * s,
        settings_fill,
        true,
    );
    p.dual_text(
        &fonts.small,
        "设置",
        settings.x,
        settings.y,
        settings.w,
        settings.h,
        settings_text,
        theme::ALIGN_CENTER,
    );
    let right = settings.x - 8.0 * s;

    let blue_str = blue_num.to_string();
    let gray_text = if blue_num > 0 {
        format!(" / {}", v.total)
    } else {
        v.total.to_string()
    };
    let aw = p.dual_measure(&fonts.small, &blue_str);
    let gw = p.dual_measure(&fonts.small, &gray_text);
    let right_w = if blue_num > 0 { aw + gw } else { gw };

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
    // Filter chips run left-to-right from the header's left edge; the
    // subtitle follows them. `chip_rect`/`hit_filter` mirror this geometry.
    let chip_y = (m.header_h - FILTER_H * s) / 2.0;
    let chip_text_h = 14.0 * s;
    let mut x = sub_x;
    for f in FILTERS {
        // 全部 is lit when nothing is selected; status chips light when
        // their own bit is set, so several can be on at once.
        let selected = if f == Filter::All {
            v.filter == model::FILTER_NONE
        } else {
            v.filter & f.mask() != 0
        };
        if selected {
            p.round_rect(
                x,
                chip_y,
                FILTER_W * s,
                FILTER_H * s,
                3.0 * s,
                theme::CARD_HOVER,
                true,
            );
        }
        let label = f.label();
        let tw = p.dual_measure(&fonts.small, label);
        let color = if selected {
            match f {
                Filter::All => theme::WHITE,
                Filter::Completed => theme::GREEN,
                Filter::Failed => theme::RED,
                Filter::Active => theme::BLUE,
            }
        } else {
            theme::TEXT_DIM
        };
        p.dual_text(
            &fonts.small,
            label,
            x + (FILTER_W * s - tw) / 2.0,
            chip_y + (FILTER_H * s - chip_text_h) / 2.0,
            tw,
            chip_text_h,
            color,
            theme::ALIGN_NEAR,
        );
        x += FILTER_W * s + FILTER_GAP * s;
    }

    let sub_right = right - right_w - 8.0 * s;
    let sub_w = (sub_right - x).max(0.0);

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
        x,
        y,
        sub_w,
        18.0 * s,
        color,
        theme::ALIGN_NEAR,
    );

    if blue_num > 0 {
        p.dual_text(
            &fonts.small,
            &blue_str,
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

    let ch_palette = channel_palette(v.rows);
    let mdl_palette = model_palette(v.rows);

    for index in 0..v.rows.len() {
        let rect = m.card_rect(index, v.scroll);
        if rect.y + rect.h < area.y || rect.y > area.y + area.h {
            continue;
        }
        draw_card(
            p,
            fonts,
            &v.rows[index],
            rect,
            v,
            index,
            m,
            &ch_palette,
            &mdl_palette,
        );
    }
    p.reset_clip();
}

/// Card background and the status accent bar, shared by both card layouts.
fn draw_card_chrome(
    p: &Painter,
    row: &Row,
    rect: Rect,
    c: &CardMetrics,
    m: Metrics,
    highlighted: bool,
) {
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
        c.band_h(rect.h),
        c.accent_w / 2.0,
        theme::status_color(row.status),
        true,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_card(
    p: &Painter,
    fonts: &theme::Fonts,
    row: &Row,
    rect: Rect,
    v: &ListView,
    index: usize,
    m: Metrics,
    ch_palette: &[(&str, u32)],
    mdl_palette: &[(&str, u32)],
) {
    let c = CardMetrics::new(m);
    let highlighted = v.hover == Some(index) || v.selected == Some(index);
    draw_card_chrome(p, row, rect, &c, m, highlighted);

    if c.single {
        draw_card_single(p, fonts, row, rect, v, m, &c, ch_palette, mdl_palette);
        return;
    }

    let text_x = rect.x + c.inner_pad;
    let mut y = rect.y + c.accent_inset;

    // --- Line 1: marks, caller, channel + time(right) ---
    let line1_y = y;
    let line1_gap = 6.0 * m.scale;
    let mut x = text_x;
    let mark_gap = 2.0 * m.scale;

    // 流 (streaming): bright when the client streamed, faint when it did not.
    if v.shows(DisplayField::Stream) {
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

    // Protocol conversion mark: 转 yellow when conversion happened, green otherwise.
    if v.shows(DisplayField::Conversion) {
        let w = p.dual_measure(&fonts.body, "转");
        p.dual_text(
            &fonts.body,
            "转",
            x,
            line1_y,
            w,
            c.line1,
            if row.protocol().is_converted() {
                theme::YELLOW
            } else {
                theme::GREEN
            },
            theme::ALIGN_NEAR,
        );
        x += w + mark_gap;
    }

    // Pass-through mark: 透 green when applied, gray otherwise.
    if v.shows(DisplayField::PassThrough) {
        let w = p.dual_measure(&fonts.body, "透");
        p.dual_text(
            &fonts.body,
            "透",
            x,
            line1_y,
            w,
            c.line1,
            if row.pass_through {
                theme::GREEN
            } else {
                theme::GRAY
            },
            theme::ALIGN_NEAR,
        );
        x += w + line1_gap;
    }

    // Right-aligned cells are placed first so the left cluster knows what room
    // is left: the time, then the account tag.
    let mut cells_limit = rect.x + rect.w - c.inner_pad;

    if v.shows(DisplayField::CreatedAt) {
        let time_text = fmt::relative_time(row.created_at.as_deref(), v.now);
        let time_w = p.dual_measure(&fonts.body, &time_text) + 4.0;
        p.dual_text(
            &fonts.body,
            &time_text,
            cells_limit - time_w,
            line1_y,
            time_w,
            c.line1,
            theme::TEXT_FAINT,
            theme::ALIGN_FAR,
        );
        cells_limit -= time_w + 4.0 * m.scale;
    }

    // Account tag, immediately left of the time. Only when there is more than
    // one account whose requests are merged here — with a single account it
    // would say nothing.
    if v.shows(DisplayField::Account) && v.accounts.len() > 1 && !row.account_name.is_empty() {
        let color = account_color(v.accounts, &row.account_id);
        let w = p.dual_measure(&fonts.small, &row.account_name);
        let tag_x = cells_limit - w - 6.0 * m.scale;
        if tag_x > x {
            p.dual_text(
                &fonts.small,
                &row.account_name,
                tag_x,
                line1_y,
                w + 2.0 * m.scale,
                c.line1,
                color,
                theme::ALIGN_NEAR,
            );
            cells_limit = tag_x - 4.0 * m.scale;
        }
    }

    // Protocol name (chat/responses/messages): the interface type the client
    // spoke, placed right after the marks so it sits beside its indicators.
    if v.shows(DisplayField::Protocol)
        && let Some(name) = &row.format
    {
        let w = p.dual_measure(&fonts.small, name);
        if x + w < cells_limit {
            p.dual_text(
                &fonts.small,
                name,
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

    // Caller (API key name).
    if v.shows(DisplayField::Caller)
        && let Some(caller) = &row.caller
    {
        let w = p.dual_measure(&fonts.small, caller);
        if x + w < cells_limit {
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
    if v.shows(DisplayField::Channel)
        && let Some(channel) = &row.channel
    {
        let color = ch_palette
            .iter()
            .find(|(n, _)| *n == channel.as_str())
            .map(|(_, c)| *c)
            .unwrap_or(theme::CYAN);
        let w = p.dual_measure(&fonts.small, channel);
        if x + w < cells_limit {
            p.dual_text(
                &fonts.small,
                channel,
                x,
                line1_y,
                w + 2.0 * m.scale,
                c.line1,
                color,
                theme::ALIGN_NEAR,
            );
            x += w + 5.0 * m.scale;
        }
    }

    // Tokens.
    if v.shows(DisplayField::Tokens) && row.total_tokens > 0 {
        let tokens = fmt::tokens_compact(row.total_tokens);
        let w = p.dual_measure(&fonts.small, &tokens);
        if x + w < cells_limit {
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
    if v.shows(DisplayField::Speed)
        && let Some(tps) = row.tps()
    {
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
    if v.shows(DisplayField::Retry) && row.attempt_count > 1 {
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
    if v.shows(DisplayField::Model) {
        let display = row.display_model();
        let model_color = mdl_palette
            .iter()
            .find(|(n, _)| *n == row.served_model())
            .map(|(_, c)| *c)
            .unwrap_or(theme::TEXT);
        let chip_w = (p.dual_measure(&fonts.small, &display) + c.chip_hpad * 2.0)
            .min((right_edge - x).max(0.0));
        let text_w = (chip_w - c.chip_hpad * 2.0).max(0.0);
        p.round_rect(
            x,
            y - c.chip_vpad,
            chip_w,
            c.chip_h(),
            3.0 * m.scale,
            theme::BORDER,
            true,
        );
        p.dual_text(
            &fonts.small,
            &display,
            x + c.chip_hpad,
            y,
            text_w,
            c.line2,
            model_color,
            theme::ALIGN_NEAR,
        );
        x += chip_w + line2_gap;
    }

    // Cache hit rate.
    if v.shows(DisplayField::Cache)
        && let Some(rate) = row.cache_hit_rate()
    {
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

/// Compact card: every field on a single line, with the model chip as the only
/// framed element.
///
/// The busiest cards cannot fit at panel width, so the right-hand cluster is
/// placed first and the left-hand cells are drawn in priority order, each only
/// if it still fits in what remains. A dropped cell is not an anomaly — it
/// simply does not fit — which is why the panel degrades this way rather than
/// shrinking the text.
#[allow(clippy::too_many_arguments)]
fn draw_card_single(
    p: &Painter,
    fonts: &theme::Fonts,
    row: &Row,
    rect: Rect,
    v: &ListView,
    m: Metrics,
    c: &CardMetrics,
    ch_palette: &[(&str, u32)],
    mdl_palette: &[(&str, u32)],
) {
    let s = m.scale;
    let gap = 4.0 * s;
    // The chip defines the content band; every text line is centred against it.
    let chip_h = c.chip_h();
    let chip_y = rect.y + (rect.h - chip_h) / 2.0;
    let small_y = chip_y + c.chip_vpad;
    let body_y = rect.y + (rect.h - c.line1) / 2.0;

    // --- Right cluster: time, TPS, retry. Placed first so the left cluster
    // knows exactly how much room it has.
    let mut right = rect.x + rect.w - c.inner_pad;

    if v.shows(DisplayField::CreatedAt) {
        let time_text = fmt::relative_time(row.created_at.as_deref(), v.now);
        let time_w = p.dual_measure(&fonts.body, &time_text) + 4.0;
        let time_x = right - time_w;
        p.dual_text(
            &fonts.body,
            &time_text,
            time_x,
            body_y,
            time_w,
            c.line1,
            theme::TEXT_FAINT,
            theme::ALIGN_FAR,
        );
        right = time_x - 8.0 * s;
    }

    // Account tag, immediately left of the time; only when more than one
    // account is merged into this list.
    if v.shows(DisplayField::Account) && v.accounts.len() > 1 && !row.account_name.is_empty() {
        let color = account_color(v.accounts, &row.account_id);
        let w = p.dual_measure(&fonts.small, &row.account_name);
        p.dual_text(
            &fonts.small,
            &row.account_name,
            right - w - 2.0 * s,
            small_y,
            w + 4.0 * s,
            c.line2,
            color,
            theme::ALIGN_FAR,
        );
        right -= w + 10.0 * s;
    }

    if v.shows(DisplayField::Speed)
        && let Some(tps) = row.tps()
    {
        let label = format!("{tps:.0} tok/s");
        let w = p.dual_measure(&fonts.small, &label);
        p.dual_text(
            &fonts.small,
            &label,
            right - w - 2.0 * s,
            small_y,
            w + 4.0 * s,
            c.line2,
            theme::TEXT_DIM,
            theme::ALIGN_FAR,
        );
        right -= w + 10.0 * s;
    }

    if v.shows(DisplayField::Retry) && row.attempt_count > 1 {
        let label = format!("{}×", row.attempt_count - 1);
        let w = p.dual_measure(&fonts.small, &label) + 6.0;
        p.dual_text(
            &fonts.small,
            &label,
            right - w,
            small_y,
            w,
            c.line2,
            theme::RED,
            theme::ALIGN_FAR,
        );
        right -= w + 10.0 * s;
    }

    // Every left-hand cell must stop short of the right cluster.
    let limit = right - 4.0 * s;

    // --- Left cluster. The status marks (流/转/透) and interface type are
    // drawn first, at a fixed offset from the card edge, so they never jitter
    // as the variable-width model name changes between rows. The model chip —
    // the row's identity — always follows, filling whatever room remains.
    let mut x = rect.x + c.inner_pad;

    // Status marks. They are drawn as one group so they stay aligned with each
    // other, but each mark can be switched off in settings; the group is only
    // dropped whole when it does not fit the room left on the line.
    let mut marks: Vec<(&str, u32)> = Vec::with_capacity(3);
    if v.shows(DisplayField::Stream) {
        marks.push((
            "流",
            if row.stream {
                theme::WHITE
            } else {
                theme::TEXT_FAINT
            },
        ));
    }
    if v.shows(DisplayField::Conversion) {
        marks.push((
            "转",
            if row.protocol().is_converted() {
                theme::YELLOW
            } else {
                theme::GREEN
            },
        ));
    }
    if v.shows(DisplayField::PassThrough) {
        marks.push((
            "透",
            if row.pass_through {
                theme::GREEN
            } else {
                theme::GRAY
            },
        ));
    }
    let mark_gap = 1.0 * s;
    let marks_w: f32 = marks
        .iter()
        .map(|(mark, _)| p.dual_measure(&fonts.body, mark) + mark_gap)
        .sum::<f32>()
        - mark_gap;
    if !marks.is_empty() && x + marks_w <= limit {
        for (mark, color) in &marks {
            let w = p.dual_measure(&fonts.body, mark);
            p.dual_text(
                &fonts.body,
                mark,
                x,
                body_y,
                w,
                c.line1,
                *color,
                theme::ALIGN_NEAR,
            );
            x += w + mark_gap;
        }
        x += gap - mark_gap;
    }

    // Interface type (chat/responses/messages): the protocol the client spoke.
    // Placed right after the marks, before the model name, so its position is
    // also fixed rather than trailing a variable-width chip.
    if v.shows(DisplayField::Protocol)
        && let Some(name) = &row.format
    {
        let w = p.dual_measure(&fonts.small, name);
        if x + w < limit {
            p.dual_text(
                &fonts.small,
                name,
                x,
                small_y,
                w + 2.0 * s,
                c.line2,
                theme::TEXT_DIM,
                theme::ALIGN_NEAR,
            );
            x += w + gap;
        }
    }

    // Channel: drawn before the model name so its position is fixed too, right
    // after the interface type, rather than trailing the variable-width chip.
    if v.shows(DisplayField::Channel)
        && let Some(channel) = &row.channel
    {
        let color = ch_palette
            .iter()
            .find(|(n, _)| *n == channel.as_str())
            .map(|(_, c)| *c)
            .unwrap_or(theme::CYAN);
        let w = p.dual_measure(&fonts.small, channel);
        if x + w < limit {
            p.dual_text(
                &fonts.small,
                channel,
                x,
                small_y,
                w + 2.0 * s,
                c.line2,
                color,
                theme::ALIGN_NEAR,
            );
            x += w + gap;
        }
    }

    // Model chip: the row's identity, drawn last so it is compressed to
    // whatever room the fixed-width prefix above leaves for it.
    if v.shows(DisplayField::Model) {
        let display = row.display_model();
        let model_color = mdl_palette
            .iter()
            .find(|(n, _)| *n == row.served_model())
            .map(|(_, c)| *c)
            .unwrap_or(theme::TEXT);
        let chip_w =
            (p.dual_measure(&fonts.small, &display) + c.chip_hpad * 2.0).min((limit - x).max(0.0));
        p.round_rect(x, chip_y, chip_w, chip_h, 3.0 * s, theme::BORDER, true);
        p.dual_text(
            &fonts.small,
            &display,
            x + c.chip_hpad,
            small_y,
            (chip_w - c.chip_hpad * 2.0).max(0.0),
            c.line2,
            model_color,
            theme::ALIGN_NEAR,
        );
        x += chip_w + gap;
    }

    // Caller (API key name).
    if v.shows(DisplayField::Caller)
        && let Some(caller) = &row.caller
    {
        let w = p.dual_measure(&fonts.small, caller);
        if x + w < limit {
            p.dual_text(
                &fonts.small,
                caller,
                x,
                small_y,
                w + 2.0 * s,
                c.line2,
                theme::TEXT_FAINT,
                theme::ALIGN_NEAR,
            );
            x += w + gap;
        }
    }

    // Tokens.
    if v.shows(DisplayField::Tokens) && row.total_tokens > 0 {
        let tokens = fmt::tokens_compact(row.total_tokens);
        let w = p.dual_measure(&fonts.small, &tokens);
        if x + w < limit {
            p.dual_text(
                &fonts.small,
                &tokens,
                x,
                small_y,
                w,
                c.line2,
                theme::TEXT_DIM,
                theme::ALIGN_NEAR,
            );
            x += w + gap;
        }
    }

    // Cache hit rate.
    if v.shows(DisplayField::Cache)
        && let Some(rate) = row.cache_hit_rate()
    {
        let label = format!("{rate:.2}%");
        let w = p.dual_measure(&fonts.small, &label);
        if x + w < limit {
            p.dual_text(
                &fonts.small,
                &label,
                x,
                small_y,
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

/// The header chip for `f`, laid out exactly as `draw_header` draws them.
/// The pinned lock icon sits ahead of the chips, so its width is folded in.
fn chip_rect(m: Metrics, v: &ListView, f: Filter) -> Rect {
    let s = m.scale;
    let mut x = if v.pinned {
        m.pad() + 2.0 + 7.0 * s + 6.0 * s
    } else {
        m.pad() + 2.0
    };
    for g in FILTERS {
        if g == f {
            return Rect {
                x,
                y: (m.header_h - FILTER_H * s) / 2.0,
                w: FILTER_W * s,
                h: FILTER_H * s,
            };
        }
        x += FILTER_W * s + FILTER_GAP * s;
    }
    unreachable!("FILTERS covers every Filter variant")
}

/// Which filter chip a point in viewport space is over. Pinned mode is
/// display-only: chips are drawn but never clickable.
pub fn hit_filter(m: Metrics, v: &ListView, x: f32, y: f32) -> Option<Filter> {
    if v.pinned || y < 0.0 || y > m.header_h {
        return None;
    }
    for f in FILTERS {
        let r = chip_rect(m, v, f);
        if x >= r.x && x <= r.x + r.w {
            return Some(f);
        }
    }
    None
}

/// Hit-test a point in viewport space.
pub fn hit_row(m: Metrics, v: &ListView, x: f32, y: f32) -> Option<usize> {
    if x < m.pad() || x > m.width - m.pad() {
        return None;
    }
    m.row_at(y, v.scroll, v.rows.len())
}

/// The card a point is over whose detail can be opened, if any. Those are the
/// only cards that take a click: everything else stays a drag handle, so a
/// stray press can never open anything. Pinned mode is display-only.
pub fn hit_error_row(m: Metrics, v: &ListView, x: f32, y: f32) -> Option<usize> {
    if v.pinned {
        return None;
    }
    let index = hit_row(m, v, x, y)?;
    v.rows.get(index).filter(|r| r.is_error()).map(|_| index)
}

#[cfg(test)]
#[path = "../tests/ui/panel.rs"]
mod tests;
