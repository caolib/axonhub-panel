//! Table layout: one row per request, one column per visible cell.
//!
//! The card layouts drop a cell that does not fit; the table instead sizes every
//! column to its widest text, so nothing is ever truncated, and reports the
//! width it needs back to the caller, which widens the window to match. Colours
//! and formatting mirror the cards, so the same request reads the same in
//! either layout.

use windows::Win32::Graphics::GdiPlus::RectF;

use crate::config::DisplayField;
use crate::format as fmt;
use crate::model::Row;
use crate::theme::{self, Painter};
use crate::ui::layout::Metrics;
use crate::ui::panel::{self, ListView};

/// Horizontal padding inside a cell, on each side. Adjacent cells therefore sit
/// two paddings apart, which is what separates the columns.
const CELL_HPAD: f32 = 6.0;

/// The status cell is a dot, not a word: the colour carries the state and the
/// column stays one character wide. Drawn with the CJK face — see `is_cjk` —
/// which is where a full-width `●` is guaranteed to live.
const STATUS_DOT: &str = "●";

/// One table column: the status the cards encode in their accent bar, followed
/// by one column per visible cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Status,
    Field(DisplayField),
}

impl Column {
    /// Every column of the table, in draw order. A retry column with nothing
    /// but dashes under it is dropped — retries are the exception, so on most
    /// pages the column would only ever be noise.
    fn all(v: &ListView) -> Vec<Column> {
        let mut out = vec![Column::Status];
        for field in DisplayField::ALL {
            if !v.shows(field) {
                continue;
            }
            if field == DisplayField::Retry && !v.rows.iter().any(has_retry) {
                continue;
            }
            out.push(Column::Field(field));
        }
        out
    }

    /// The cell's text. The table shows every column, so a value the row does
    /// not have still has to read as "none" rather than vanish.
    fn text(self, row: &Row, now: i64) -> String {
        match self {
            Column::Status => STATUS_DOT.into(),
            Column::Field(field) => match field {
                DisplayField::Stream => yes_no(row.stream),
                DisplayField::Conversion => yes_no(row.protocol().is_converted()),
                DisplayField::PassThrough => yes_no(row.pass_through),
                DisplayField::Model => row.display_model(),
                DisplayField::Protocol => text_or_dash(row.format.as_deref()),
                DisplayField::Caller => text_or_dash(row.caller.as_deref()),
                DisplayField::Channel => text_or_dash(row.channel.as_deref()),
                DisplayField::Tokens => {
                    if row.total_tokens > 0 {
                        fmt::tokens_compact(row.total_tokens)
                    } else {
                        fmt::DASH.into()
                    }
                }
                DisplayField::Cache => row
                    .cache_hit_rate()
                    .map_or_else(|| fmt::DASH.into(), |rate| format!("{rate:.2}%")),
                DisplayField::Speed => row
                    .tps()
                    .map_or_else(|| fmt::DASH.into(), |tps| format!("{tps:.0} tok/s")),
                DisplayField::Retry => {
                    if has_retry(row) {
                        format!("{}×", row.attempt_count - 1)
                    } else {
                        fmt::DASH.into()
                    }
                }
                DisplayField::Cost => row.total_cost.map_or_else(|| fmt::DASH.into(), fmt::cost),
                DisplayField::Latency => fmt::latency(row.latency_ms, row.first_token_ms)
                    .unwrap_or_else(|| fmt::DASH.into()),
                DisplayField::Completion => completion_text(row),
                DisplayField::Source => source_text(row),
                DisplayField::Account => text_or_dash(Some(&row.account_name)),
                DisplayField::CreatedAt => fmt::relative_time(row.created_at.as_deref(), now),
            },
        }
    }

    /// The colour the cell is drawn with, matching the card layouts.
    fn color(
        self,
        row: &Row,
        v: &ListView,
        channels: &[(&str, u32)],
        models: &[(&str, u32)],
    ) -> u32 {
        match self {
            Column::Status => theme::status_color(row.status),
            Column::Field(field) => match field {
                DisplayField::Stream => {
                    if row.stream {
                        theme::WHITE
                    } else {
                        theme::TEXT_FAINT
                    }
                }
                DisplayField::Conversion => {
                    if row.protocol().is_converted() {
                        theme::YELLOW
                    } else {
                        theme::GREEN
                    }
                }
                DisplayField::PassThrough => {
                    if row.pass_through {
                        theme::GREEN
                    } else {
                        theme::GRAY
                    }
                }
                DisplayField::Model => models
                    .iter()
                    .find(|(name, _)| *name == row.served_model())
                    .map(|(_, color)| *color)
                    .unwrap_or(theme::TEXT),
                DisplayField::Protocol => theme::TEXT_DIM,
                DisplayField::Caller => theme::TEXT_FAINT,
                DisplayField::Channel => channels
                    .iter()
                    .find(|(name, _)| Some(*name) == row.channel.as_deref())
                    .map(|(_, color)| *color)
                    .unwrap_or(theme::CYAN),
                DisplayField::Tokens => theme::TEXT_DIM,
                DisplayField::Cache => {
                    if row.cache_hit_is_low() {
                        theme::RED
                    } else {
                        theme::GREEN
                    }
                }
                DisplayField::Speed => theme::TEXT_DIM,
                DisplayField::Retry => {
                    if has_retry(row) {
                        theme::RED
                    } else {
                        theme::TEXT_DIM
                    }
                }
                DisplayField::Cost => theme::YELLOW,
                DisplayField::Latency | DisplayField::Completion => theme::TEXT_DIM,
                DisplayField::Source => theme::TEXT_FAINT,
                DisplayField::Account => panel::account_color(v.accounts, &row.account_id),
                DisplayField::CreatedAt => theme::TEXT_FAINT,
            },
        }
    }
}

/// A retry happened only when the request was attempted more than once;
/// a first attempt — or none yet — leaves the cell with its placeholder.
fn has_retry(row: &Row) -> bool {
    row.attempt_count > 1
}

fn yes_no(value: bool) -> String {
    if value { "是" } else { "否" }.into()
}

fn text_or_dash(value: Option<&str>) -> String {
    match value.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => text.to_string(),
        None => fmt::DASH.into(),
    }
}

/// `出 1.2k 思 340`: completion tokens, with the reasoning share called out.
fn completion_text(row: &Row) -> String {
    if row.completion_tokens <= 0 {
        return fmt::DASH.into();
    }
    let mut label = format!("出 {}", fmt::tokens_compact(row.completion_tokens));
    if row.reasoning_tokens > 0 {
        label.push_str(&format!(
            " 思 {}",
            fmt::tokens_compact(row.reasoning_tokens)
        ));
    }
    label
}

/// `api · ::1`: the entry point and the client address, each shown on its own
/// when the other is missing.
fn source_text(row: &Row) -> String {
    match (&row.source, &row.client_ip) {
        (Some(source), Some(ip)) => format!("{source} · {ip}"),
        (Some(source), None) => source.clone(),
        (None, Some(ip)) => ip.clone(),
        (None, None) => fmt::DASH.into(),
    }
}

/// Width of every column: the widest cell under it, plus the padding on both
/// sides. Pure so the fit can be exercised without a device context — `measure`
/// is the real font metrics in production, a stub in tests.
fn column_widths(
    rows: &[Row],
    columns: &[Column],
    now: i64,
    hpad: f32,
    measure: impl Fn(&str) -> f32,
) -> Vec<f32> {
    columns
        .iter()
        .map(|column| {
            let mut width = 0.0f32;
            for row in rows {
                width = width.max(measure(&column.text(row, now)));
            }
            width + hpad * 2.0
        })
        .collect()
}

/// Draw the table and report the client width that fits it, or `None` when
/// there is nothing to size — an empty list keeps the window it has.
pub(super) fn draw(p: &Painter, fonts: &theme::Fonts, m: Metrics, v: &ListView) -> Option<f32> {
    if v.rows.is_empty() {
        panel::draw_empty(p, fonts, m, v);
        return None;
    }

    let hpad = CELL_HPAD * m.scale;
    let columns = Column::all(v);
    let widths = column_widths(v.rows, &columns, v.now, hpad, |text| {
        p.dual_measure(&fonts.small, text)
    });

    let channels = panel::channel_palette(v.rows);
    let models = panel::model_palette(v.rows);

    // Clip so rows scroll inside the list area rather than over the chrome.
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
        let row = &v.rows[index];
        if v.hover == Some(index) || v.selected == Some(index) {
            p.fill_rect(rect.x, rect.y, rect.w, rect.h, theme::CARD_HOVER);
        }
        let mut x = rect.x;
        for (column, width) in columns.iter().zip(&widths) {
            p.dual_text(
                &fonts.small,
                &column.text(row, v.now),
                x + hpad,
                rect.y,
                width - hpad * 2.0,
                rect.h,
                column.color(row, v, &channels, &models),
                theme::ALIGN_NEAR,
            );
            x += width;
        }
        p.fill_rect(rect.x, rect.y + rect.h - 1.0, rect.w, 1.0, theme::SEPARATOR);
    }
    p.reset_clip();

    // What the window must be for every column to fit, padding included.
    Some(m.pad() * 2.0 + widths.iter().sum::<f32>())
}

#[cfg(test)]
#[path = "../tests/ui/table.rs"]
mod tests;
