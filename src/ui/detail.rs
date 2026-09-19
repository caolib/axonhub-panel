//! The error-detail popup: content model, layout and painting.
//!
//! The cards stay status-only; a failed or canceled request is the one case
//! where the *reason* matters more than the summary, so clicking one opens this
//! window. Content is assembled from the card's own row plus the upstream
//! attempts AxonHub recorded for the request — a failed request usually has
//! several, one per retry — and laid out as a scrolling document of sections
//! and label/value rows.
//!
//! Geometry is authored at 96 DPI and multiplied by the scale, like the panel.

use windows::Win32::Graphics::GdiPlus::RectF;

use crate::format as fmt;
use crate::model::{ExecutionDetail, Protocol, Row};
use crate::theme::{self, Fonts, Painter};
use crate::ui::layout::Rect;

/// Default size in logical pixels at 96 DPI, and the floor the window can be
/// resized to. Each form gets the height it usually needs: the compact one is
/// a handful of rows, so a full-height window would sit mostly empty.
pub const POPUP_W: f32 = 470.0;
pub const POPUP_H: f32 = 480.0;
pub const POPUP_H_COMPACT: f32 = 340.0;
pub const MIN_W: f32 = 360.0;
pub const MIN_H: f32 = 220.0;

const PAD: f32 = 10.0;
const HEADER_H: f32 = 48.0;
const FOOTER_H: f32 = 38.0;
/// The label column. `value_x` starts after it and the value wraps there.
const LABEL_W: f32 = 58.0;
const ROW_H: f32 = 17.0;
/// Height of a section title, and of the air above it.
const SECTION_H: f32 = 18.0;
const GAP: f32 = 8.0;
const BUTTON_H: f32 = 22.0;
const CLOSE_W: f32 = 22.0;
const COPY_W: f32 = 52.0;
const OPEN_W: f32 = 72.0;
/// Wide enough for 完整信息 / 简化信息, the widest toggle label.
const TOGGLE_W: f32 = 76.0;
const BUTTON_GAP: f32 = 8.0;

/// How much of an execution the document shows.
///
/// A failed request is usually read for four things — which channel answered,
/// with what status code, on which model, and why it failed — so that is what
/// the popup opens with. Everything else is one click away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    Compact,
    Full,
}

/// The window height `depth` is authored for. The window is refitted when the
/// form changes, so neither one leaves the other's empty space behind.
pub fn default_height(depth: Depth) -> f32 {
    match depth {
        Depth::Compact => POPUP_H_COMPACT,
        Depth::Full => POPUP_H,
    }
}

/// The popup's only controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Copy,
    Open,
    Close,
    /// Switches between the compact and the full document.
    Toggle,
}

impl Button {
    /// The toggle's label names what clicking it will show, so `depth` only
    /// matters for that one button.
    pub fn label(self, depth: Depth) -> &'static str {
        match self {
            Button::Copy => "复制",
            Button::Open => "打开网页",
            // U+00D7: present in every face, unlike a heavier multiplication
            // or cross glyph.
            Button::Close => "×",
            Button::Toggle => match depth {
                Depth::Compact => "完整信息",
                Depth::Full => "简化信息",
            },
        }
    }
}

/// Popup geometry for one draw, in device pixels.
#[derive(Debug, Clone, Copy)]
pub struct Popup {
    pub width: f32,
    pub height: f32,
    pub scale: f32,
}

impl Popup {
    pub fn new(width: f32, height: f32, scale: f32) -> Self {
        Popup {
            width,
            height,
            scale: scale.max(0.25),
        }
    }

    fn s(&self, v: f32) -> f32 {
        v * self.scale
    }

    pub fn pad(&self) -> f32 {
        self.s(PAD)
    }

    pub fn header_h(&self) -> f32 {
        self.s(HEADER_H)
    }

    pub fn footer_h(&self) -> f32 {
        self.s(FOOTER_H)
    }

    /// Top of the scrolling area.
    pub fn content_top(&self) -> f32 {
        self.header_h()
    }

    /// Height of the scrolling area, i.e. what is visible at once.
    pub fn content_view_h(&self) -> f32 {
        (self.height - self.header_h() - self.footer_h()).max(0.0)
    }

    pub fn label_w(&self) -> f32 {
        self.s(LABEL_W)
    }

    pub fn row_h(&self) -> f32 {
        self.s(ROW_H)
    }

    pub fn section_h(&self) -> f32 {
        self.s(SECTION_H)
    }

    pub fn gap(&self) -> f32 {
        self.s(GAP)
    }

    pub fn value_x(&self) -> f32 {
        self.pad() + self.label_w()
    }

    pub fn value_w(&self) -> f32 {
        (self.width - self.pad() * 2.0 - self.label_w()).max(20.0)
    }
}

/// Where a control sits. Geometry only — no font measurement — so hit-testing
/// and drawing cannot disagree.
pub fn button_rect(m: &Popup, button: Button) -> Rect {
    let s = m.scale;
    let footer_y = m.height - (m.footer_h() + BUTTON_H * s) / 2.0;
    match button {
        Button::Close => Rect {
            x: m.width - m.pad() - CLOSE_W * s,
            y: (m.header_h() - CLOSE_W * s) / 2.0,
            w: CLOSE_W * s,
            h: CLOSE_W * s,
        },
        // The switch sits on the footer's left, away from the two actions.
        Button::Toggle => Rect {
            x: m.pad(),
            y: footer_y,
            w: TOGGLE_W * s,
            h: BUTTON_H * s,
        },
        Button::Open | Button::Copy => {
            let (w, offset) = match button {
                Button::Open => (OPEN_W * s, 0.0),
                _ => (COPY_W * s, (OPEN_W + BUTTON_GAP) * s),
            };
            Rect {
                x: m.width - m.pad() - offset - w,
                y: footer_y,
                w,
                h: BUTTON_H * s,
            }
        }
    }
}

pub fn hit_button(m: &Popup, x: f32, y: f32) -> Option<Button> {
    [Button::Close, Button::Toggle, Button::Copy, Button::Open]
        .into_iter()
        .find(|b| button_rect(m, *b).contains(x, y))
}

/// One row of the document.
pub enum Line {
    /// Section title, drawn in `color` with the body font.
    Section(String, u32),
    /// `label: value`; the value wraps under a hanging indent.
    Field {
        label: &'static str,
        value: String,
        color: u32,
    },
    /// A bare sentence, e.g. the loading or failure notice.
    Note(String, u32),
    Gap,
}

pub struct Doc {
    pub lines: Vec<Line>,
}

impl Doc {
    /// Assemble the popup's content. `executions` may be empty — nothing was
    /// recorded, or the request failed before an attempt — or hold several:
    /// AxonHub retries the next channel, and the reason for each attempt is the
    /// point of this view.
    pub fn build(row: &Row, executions: &[ExecutionDetail], now: i64, depth: Depth) -> Doc {
        let mut lines = Vec::new();
        if depth == Depth::Full {
            lines.push(Line::Section("请求".into(), theme::TEXT_DIM));
            lines.push(field(
                "状态",
                row.status.label(),
                theme::status_color(row.status),
            ));
            lines.push(field("模型", row.display_model(), theme::TEXT));
            if let Some(channel) = non_empty(&row.channel) {
                lines.push(field("渠道", channel, theme::CYAN));
            }
            if let Some(caller) = non_empty(&row.caller) {
                lines.push(field("调用者", caller, theme::TEXT_DIM));
            }
            lines.push(field("协议", protocol_text(row), theme::TEXT_DIM));
            lines.push(field("流式", yes_no(row.stream), theme::TEXT_DIM));
            lines.push(field("令牌", tokens_text(row), theme::TEXT_DIM));
            lines.push(field("耗时", timing_text(row), theme::TEXT_DIM));
            lines.push(field(
                "时间",
                format!(
                    "{} · {}",
                    fmt::local_datetime(row.created_at.as_deref()),
                    fmt::relative_time(row.created_at.as_deref(), now)
                ),
                theme::TEXT_DIM,
            ));
            lines.push(Line::Gap);
        }

        if executions.is_empty() {
            // Without attempts the compact view would open empty, so it says so
            // instead.
            if depth == Depth::Full {
                lines.push(Line::Section("执行".into(), theme::TEXT_DIM));
            }
            lines.push(Line::Note("没有记录到上游执行".into(), theme::TEXT_FAINT));
            return Doc { lines };
        }

        if depth == Depth::Full {
            lines.push(Line::Section(
                format!("执行 {} 次", executions.len()),
                theme::TEXT_DIM,
            ));
        }
        for (index, execution) in executions.iter().enumerate() {
            push_execution(&mut lines, index, execution, depth);
        }
        Doc { lines }
    }
}

fn field(label: &'static str, value: impl Into<String>, color: u32) -> Line {
    Line::Field {
        label,
        value: value.into(),
        color,
    }
}

fn push_execution(lines: &mut Vec<Line>, index: usize, ex: &ExecutionDetail, depth: Depth) {
    let status = ex.status();
    let mut title = format!("执行 {} · {}", index + 1, status.label());
    // The compact view keeps the code for its own row, so the heading names the
    // outcome only.
    if depth == Depth::Full
        && let Some(code) = ex.response_status_code
    {
        title.push_str(&format!(" · HTTP {code}"));
    }
    lines.push(Line::Section(title, theme::status_color(status)));

    if let Some(channel) = ex.channel.as_ref() {
        let name = non_empty(&channel.name).unwrap_or_default();
        let kind = non_empty(&channel.kind).unwrap_or_default();
        let label = match (name.is_empty(), kind.is_empty()) {
            (false, false) => format!("{name}({kind})"),
            (false, true) => name,
            (true, false) => kind,
            (true, true) => String::new(),
        };
        if !label.is_empty() {
            lines.push(field("渠道", label, theme::CYAN));
        }
        if depth == Depth::Full
            && let Some(base) = non_empty(&channel.base_url)
        {
            lines.push(field("端点", base, theme::TEXT_DIM));
        }
    }

    // What the compact view is for: which channel answered, with what code, on
    // which model, and why it failed.
    if depth == Depth::Compact {
        lines.push(field("状态码", status_code_text(ex), status_code_color(ex)));
        if let Some(model) = non_empty(&ex.model_id) {
            lines.push(field("模型", with_effort(model, ex), theme::TEXT));
        }
        if let Some(message) = non_empty(&ex.error_message) {
            lines.push(field("错误", message, theme::RED));
        }
        lines.push(Line::Gap);
        return;
    }

    if let Some(model) = non_empty(&ex.model_id) {
        lines.push(field("模型", with_effort(model, ex), theme::TEXT));
    }
    if let Some(span) = span_text(ex.created_at.as_deref(), ex.updated_at.as_deref()) {
        lines.push(field("时间", span, theme::TEXT_DIM));
    }
    if let Some(url) = non_empty(&ex.request_url) {
        lines.push(field("地址", url, theme::TEXT_DIM));
    }
    if let Some(timing) = execution_timing(ex) {
        lines.push(field("耗时", timing, theme::TEXT_DIM));
    }
    if let Some(format) = non_empty(&ex.format) {
        let mut text = format;
        if ex.pass_through_applied.unwrap_or(false) {
            text.push_str(" · 透传");
        }
        lines.push(field("格式", text, theme::TEXT_DIM));
    }
    if let Some(message) = non_empty(&ex.error_message) {
        lines.push(field("错误", message, theme::RED));
    }
    lines.push(Line::Gap);
}

/// `glm-5.2(max)`: the model an attempt ran on, with its reasoning effort.
fn with_effort(model: String, ex: &ExecutionDetail) -> String {
    match non_empty(&ex.reasoning_effort) {
        Some(effort) => format!("{model}({effort})"),
        None => model,
    }
}

fn status_code_text(ex: &ExecutionDetail) -> String {
    match ex.response_status_code {
        Some(code) => code.to_string(),
        None => fmt::DASH.into(),
    }
}

/// Red for a refusal the upstream reported, green for a success, faint when the
/// attempt never got a status line at all.
fn status_code_color(ex: &ExecutionDetail) -> u32 {
    match ex.response_status_code {
        Some(code) if code >= 400 => theme::RED,
        Some(_) => theme::GREEN,
        None => theme::TEXT_DIM,
    }
}

/// The document as copyable text: what `复制` and Ctrl+C put on the clipboard.
pub fn plain_text(doc: &Doc) -> String {
    let mut out = String::new();
    for line in &doc.lines {
        match line {
            Line::Section(text, _) | Line::Note(text, _) => {
                out.push_str(text);
                out.push('\n');
            }
            Line::Field { label, value, .. } => {
                out.push_str(label);
                out.push_str(": ");
                out.push_str(value);
                out.push('\n');
            }
            Line::Gap => out.push('\n'),
        }
    }
    out
}

fn non_empty(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn yes_no(value: bool) -> &'static str {
    if value { "是" } else { "否" }
}

/// How the inbound and upstream protocols relate. Unlike the card's 转 mark,
/// the popup can spell both sides out.
fn protocol_text(row: &Row) -> String {
    match row.protocol() {
        Protocol::Same(one) | Protocol::Single(one) => one,
        Protocol::Converted { from, to } => format!("{from} → {to}"),
        Protocol::Unknown => fmt::DASH.into(),
    }
}

fn tokens_text(row: &Row) -> String {
    if row.total_tokens <= 0 {
        return fmt::DASH.into();
    }
    let mut text = fmt::tokens_compact(row.total_tokens);
    if let Some(rate) = row.cache_hit_rate() {
        text.push_str(&format!(" · 缓存 {rate:.1}%"));
    }
    text
}

fn timing_text(row: &Row) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(ms) = row.latency_ms {
        parts.push(format!("总 {:.1}s", ms as f64 / 1000.0));
    }
    if let Some(ms) = row.first_token_ms {
        parts.push(format!("首字 {:.1}s", ms as f64 / 1000.0));
    }
    if let Some(tps) = row.tps() {
        parts.push(format!("{tps:.0} tok/s"));
    }
    if parts.is_empty() {
        fmt::DASH.into()
    } else {
        parts.join(" · ")
    }
}

/// From an attempt starting to its record being written. The end timestamp is
/// when AxonHub finished writing, so the span is an upper bound on the attempt
/// itself — still the only duration an execution carries.
fn span_text(created: Option<&str>, updated: Option<&str>) -> Option<String> {
    let start = created.and_then(crate::time::parse_unix)?;
    let clock = fmt::local_clock(created);
    let Some(end) = updated.and_then(crate::time::parse_unix) else {
        return Some(clock);
    };
    if end <= start {
        return Some(clock);
    }
    Some(format!(
        "{clock} → {} · {}",
        fmt::local_clock(updated),
        fmt::duration_span(end - start)
    ))
}

fn execution_timing(ex: &ExecutionDetail) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(ms) = ex.metrics_first_token_latency_ms {
        parts.push(format!("首字 {:.1}s", ms as f64 / 1000.0));
    }
    if let Some(ms) = ex.metrics_reasoning_duration_ms {
        parts.push(format!("推理 {:.1}s", ms as f64 / 1000.0));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// What the window needs to paint one frame.
pub struct View<'a> {
    pub row: &'a Row,
    /// `None` until the executions arrive.
    pub doc: Option<&'a Doc>,
    /// Why the fetch failed; shown instead of the document.
    pub error: Option<&'a str>,
    /// Which form the document is in; drives the toggle's label.
    pub depth: Depth,
    pub scroll: f32,
    pub hover: Option<Button>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Section,
    Label,
    Value,
    Note,
}

/// One drawable string, positioned relative to the top of the content area.
struct Visual {
    kind: Kind,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    text: String,
    color: u32,
}

/// Paint the popup and return the content height, so the caller can clamp the
/// scroll offset without laying the document out a second time.
pub fn draw(p: &Painter, fonts: &Fonts, m: &Popup, v: &View) -> f32 {
    p.fill_rect(0.0, 0.0, m.width, m.height, theme::BG);
    draw_edge(p, m);
    p.fill_rect(0.0, m.header_h() - 1.0, m.width, 1.0, theme::SEPARATOR);
    p.fill_rect(0.0, m.height - m.footer_h(), m.width, 1.0, theme::SEPARATOR);
    draw_header(p, fonts, m, v);
    draw_footer(p, fonts, m, v);
    draw_body(p, fonts, m, v)
}

fn draw_edge(p: &Painter, m: &Popup) {
    let c = theme::with_alpha(theme::BORDER, 0xE0);
    let t = m.scale.max(1.0);
    p.fill_rect(0.0, 0.0, m.width, t, c);
    p.fill_rect(0.0, m.height - t, m.width, t, c);
    p.fill_rect(0.0, 0.0, t, m.height, c);
    p.fill_rect(m.width - t, 0.0, t, m.height, c);
}

fn draw_header(p: &Painter, fonts: &Fonts, m: &Popup, v: &View) {
    let s = m.scale;
    let line_h = m.row_h();
    let y = m.pad() + 2.0 * s;

    let status = v.row.status.label();
    let width = p.dual_measure(&fonts.body, status);
    p.dual_text(
        &fonts.body,
        status,
        m.pad(),
        y,
        width,
        line_h,
        theme::status_color(v.row.status),
        theme::ALIGN_NEAR,
    );

    // The model, unless the status word already fills the row.
    let x = m.pad() + width + 6.0 * s;
    let model = v.row.display_model();
    let model_w = (m.width - m.pad() * 2.0 - CLOSE_W * s - 8.0 * s - (x - m.pad())).max(0.0);
    p.dual_text(
        &fonts.body,
        &model,
        x,
        y,
        model_w,
        line_h,
        theme::TEXT,
        theme::ALIGN_NEAR,
    );

    // The request GUID, so it can be read off and pasted elsewhere.
    let sub_w = (m.width - m.pad() * 2.0 - CLOSE_W * s - 8.0 * s).max(0.0);
    p.dual_text(
        &fonts.small,
        &v.row.id,
        m.pad(),
        y + line_h + 2.0 * s,
        sub_w,
        line_h,
        theme::TEXT_FAINT,
        theme::ALIGN_NEAR,
    );

    let rect = button_rect(m, Button::Close);
    let hovered = v.hover == Some(Button::Close);
    if hovered {
        p.round_rect(
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            4.0 * s,
            theme::CARD_HOVER,
            true,
        );
    }
    let label = Button::Close.label(v.depth);
    let w = p.dual_measure(&fonts.body, label);
    p.dual_text(
        &fonts.body,
        label,
        rect.x + (rect.w - w) / 2.0,
        rect.y + (rect.h - line_h) / 2.0,
        w,
        line_h,
        if hovered { theme::RED } else { theme::TEXT_DIM },
        theme::ALIGN_NEAR,
    );
}

fn draw_footer(p: &Painter, fonts: &Fonts, m: &Popup, v: &View) {
    let s = m.scale;
    for button in [Button::Toggle, Button::Copy, Button::Open] {
        let rect = button_rect(m, button);
        let hovered = v.hover == Some(button);
        p.round_rect(
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            4.0 * s,
            if hovered {
                theme::CARD_HOVER
            } else {
                theme::CARD
            },
            true,
        );
        let label = button.label(v.depth);
        let w = p.dual_measure(&fonts.small, label);
        p.dual_text(
            &fonts.small,
            label,
            rect.x + (rect.w - w) / 2.0,
            rect.y + (rect.h - m.row_h()) / 2.0,
            w,
            m.row_h(),
            if hovered {
                theme::WHITE
            } else {
                theme::TEXT_DIM
            },
            theme::ALIGN_NEAR,
        );
    }
}

fn draw_body(p: &Painter, fonts: &Fonts, m: &Popup, v: &View) -> f32 {
    // One of three states: the document, a failure notice, or the wait.
    let notice;
    let doc = match (v.doc, v.error) {
        (Some(doc), _) => doc,
        (None, Some(error)) => {
            notice = Doc {
                lines: vec![
                    Line::Note("读取执行记录失败".into(), theme::RED),
                    Line::Note(error.to_string(), theme::RED),
                ],
            };
            &notice
        }
        (None, None) => {
            notice = Doc {
                lines: vec![Line::Note("正在读取执行记录…".into(), theme::TEXT_FAINT)],
            };
            &notice
        }
    };

    let (visuals, content_h) = layout(p, fonts, m, doc);
    let top = m.content_top();
    let view_h = m.content_view_h();
    p.clip(RectF {
        X: 0.0,
        Y: top,
        Width: m.width,
        Height: view_h,
    });
    for visual in &visuals {
        let y = top + visual.y - v.scroll;
        if y + visual.h < top || y > top + view_h {
            continue;
        }
        let font = match visual.kind {
            Kind::Section => &fonts.body,
            _ => &fonts.small,
        };
        p.dual_text(
            font,
            &visual.text,
            visual.x,
            y,
            visual.w,
            visual.h,
            visual.color,
            theme::ALIGN_NEAR,
        );
    }
    p.reset_clip();
    draw_scrollbar(p, m, content_h, v.scroll);
    content_h
}

/// A passive indicator: the popup scrolls with the wheel and the keyboard, so
/// this only shows how much is left.
fn draw_scrollbar(p: &Painter, m: &Popup, content_h: f32, scroll: f32) {
    let view_h = m.content_view_h();
    let max_scroll = content_h - view_h;
    if max_scroll <= 1.0 {
        return;
    }
    let s = m.scale;
    let x = m.width - 4.5 * s;
    let w = 3.0 * s;
    let thumb_h = (view_h / content_h * view_h).max(24.0 * s).min(view_h);
    let offset = (scroll / max_scroll).clamp(0.0, 1.0) * (view_h - thumb_h);
    p.round_rect(x, m.content_top(), w, view_h, w / 2.0, theme::CARD, true);
    p.round_rect(
        x,
        m.content_top() + offset,
        w,
        thumb_h,
        w / 2.0,
        theme::TEXT_FAINT,
        true,
    );
}

/// Lay the document out into drawable strings, `y` measured from the top of the
/// content area. Wrapping happens here, against the real font metrics, so the
/// returned height is what the scroll range must be computed from.
fn layout(p: &Painter, fonts: &Fonts, m: &Popup, doc: &Doc) -> (Vec<Visual>, f32) {
    let mut out: Vec<Visual> = Vec::new();
    let row_h = m.row_h();
    let value_x = m.value_x();
    let value_w = m.value_w();
    let mut y = 0.0f32;
    let mut started = false;

    for line in &doc.lines {
        match line {
            Line::Section(text, color) => {
                y += if started { m.gap() } else { m.gap() / 2.0 };
                out.push(Visual {
                    kind: Kind::Section,
                    x: m.pad(),
                    y,
                    w: (m.width - m.pad() * 2.0).max(0.0),
                    h: m.section_h(),
                    text: text.clone(),
                    color: *color,
                });
                y += m.section_h() + m.gap() / 2.0;
                started = true;
            }
            Line::Field {
                label,
                value,
                color,
            } => {
                for (index, text) in p.wrap(&fonts.small, value, value_w).into_iter().enumerate() {
                    if index == 0 {
                        out.push(Visual {
                            kind: Kind::Label,
                            x: m.pad(),
                            y,
                            w: m.label_w(),
                            h: row_h,
                            text: format!("{label}:"),
                            color: theme::TEXT_FAINT,
                        });
                    }
                    out.push(Visual {
                        kind: Kind::Value,
                        x: value_x,
                        y,
                        w: value_w,
                        h: row_h,
                        text,
                        color: *color,
                    });
                    y += row_h;
                }
                started = true;
            }
            Line::Note(text, color) => {
                for wrapped in p.wrap(&fonts.small, text, m.width - m.pad() * 2.0) {
                    out.push(Visual {
                        kind: Kind::Note,
                        x: m.pad(),
                        y,
                        w: (m.width - m.pad() * 2.0).max(0.0),
                        h: row_h,
                        text: wrapped,
                        color: *color,
                    });
                    y += row_h;
                }
                started = true;
            }
            Line::Gap => {
                y += m.gap();
                started = true;
            }
        }
    }
    (out, y + m.pad())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DetailData, Envelope, Status};

    /// Two attempts of one failed request — the shape the popup exists for.
    /// Values are stand-ins: the real ones carry gateway internals.
    const SAMPLE: &str = r#"{
      "data": { "node": { "executions": { "edges": [
        { "node": {
          "createdAt": "2026-09-18T11:32:33.2110863Z",
          "updatedAt": "2026-09-18T11:33:23.0073162Z",
          "modelID": "glm-5.3-flash",
          "channel": { "id": "gid://axonhub/Channel/2", "name": "ch-b", "type": "openai", "baseURL": "https://api.example.com/v1" },
          "status": "canceled",
          "responseStatusCode": null,
          "errorMessage": "failed to do request: HTTP request failed: context canceled",
          "requestURL": "https://api.example.com/v1/chat/completions",
          "format": "openai/chat_completions",
          "reasoningEffort": "max",
          "passThroughApplied": false,
          "metricsFirstTokenLatencyMs": null,
          "metricsReasoningDurationMs": null
        } },
        { "node": {
          "createdAt": "2026-09-18T11:28:23.0328274Z",
          "updatedAt": "2026-09-18T11:32:32.2058876Z",
          "modelID": "glm-5.3-flash",
          "channel": { "id": "gid://axonhub/Channel/1", "name": "ch-a", "type": "openai", "baseURL": "http://10.0.0.1:18090/v1" },
          "status": "failed",
          "responseStatusCode": 429,
          "errorMessage": "Concurrency limit exceeded for account, please retry later",
          "requestURL": "http://10.0.0.1:18090/v1/chat/completions",
          "format": "openai/chat_completions",
          "reasoningEffort": "max",
          "passThroughApplied": false,
          "metricsFirstTokenLatencyMs": null,
          "metricsReasoningDurationMs": null
        } }
      ], "totalCount": 2 } } }
    }"#;

    fn executions() -> Vec<ExecutionDetail> {
        // Parsed through the envelope, exactly as the client does.
        let envelope: Envelope<DetailData> = serde_json::from_str(SAMPLE).expect("sample parses");
        envelope
            .data
            .expect("data")
            .node
            .expect("node")
            .executions
            .expect("executions")
            .edges
            .into_iter()
            .filter_map(|e| e.node)
            .collect()
    }

    fn row() -> Row {
        Row {
            id: "gid://axonhub/Request/42899".into(),
            created_at: Some("2026-09-18T11:28:03Z".into()),
            status: Status::Failed,
            model: "glm-5.3-flash".into(),
            routed_model: None,
            channel: Some("ch-a".into()),
            caller: Some("cc".into()),
            format: Some("chat".into()),
            reasoning_effort: Some("max".into()),
            upstream_format: Some("chat".into()),
            pass_through: false,
            stream: false,
            latency_ms: Some(269_000),
            first_token_ms: None,
            prompt_tokens: 900,
            total_tokens: 1200,
            cached_tokens: 0,
            attempt_count: 2,
            failed_attempts: 1,
            attempts_truncated: false,
        }
    }

    #[test]
    fn parses_the_executions_query_answer() {
        let found = executions();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].status(), Status::Canceled);
        assert_eq!(found[1].status(), Status::Failed);
        assert_eq!(found[1].response_status_code, Some(429));
        assert_eq!(
            found[1].channel.as_ref().and_then(|c| c.name.as_deref()),
            Some("ch-a")
        );
    }

    #[test]
    fn the_full_document_carries_every_channel_code_and_error() {
        let doc = Doc::build(&row(), &executions(), 1_789_179_657, Depth::Full);
        let text = plain_text(&doc);

        assert!(text.contains("状态: 失败"), "{text}");
        assert!(text.contains("模型: glm-5.3-flash(max)"), "{text}");
        assert!(text.contains("执行 2 次"), "{text}");
        assert!(text.contains("执行 1 · 已取消"), "{text}");
        assert!(text.contains("执行 2 · 失败 · HTTP 429"), "{text}");
        assert!(text.contains("渠道: ch-a(openai)"), "{text}");
        assert!(text.contains("端点: http://10.0.0.1:18090/v1"), "{text}");
        assert!(
            text.contains("错误: Concurrency limit exceeded for account, please retry later"),
            "{text}"
        );
        assert!(
            text.contains("错误: failed to do request: HTTP request failed: context canceled"),
            "{text}"
        );
        // Every line stands alone — a field never runs into the next heading,
        // and a blank line separates one execution from the next.
        assert!(
            text.contains("context canceled\n\n执行 2 · 失败 · HTTP 429\n"),
            "{text:?}"
        );
        assert!(
            text.contains("地址: https://api.example.com/v1/chat/completions"),
            "{text}"
        );
    }

    #[test]
    fn the_compact_document_keeps_the_channel_code_model_and_error() {
        let doc = Doc::build(&row(), &executions(), 1_789_179_657, Depth::Compact);
        let text = plain_text(&doc);

        assert!(text.contains("执行 1 · 已取消"), "{text}");
        assert!(text.contains("执行 2 · 失败"), "{text}");
        assert!(text.contains("渠道: ch-b(openai)"), "{text}");
        assert!(text.contains("渠道: ch-a(openai)"), "{text}");
        assert!(text.contains("状态码: 429"), "{text}");
        assert!(text.contains("模型: glm-5.3-flash(max)"), "{text}");
        assert!(
            text.contains("错误: Concurrency limit exceeded for account, please retry later"),
            "{text}"
        );
        // Everything else is behind the toggle.
        for hidden in [
            "请求",
            "调用者",
            "协议",
            "令牌",
            "耗时",
            "端点",
            "地址",
            "格式",
            "HTTP 429",
        ] {
            assert!(
                !text.contains(hidden),
                "{hidden} leaked into the compact view: {text}"
            );
        }
        // Every attempt gets a status row, even the one that never got a status
        // line — there the value is a dash rather than a number.
        assert_eq!(text.matches("状态码:").count(), 2, "{text}");
        assert!(text.contains("状态码: —"), "{text}");
    }

    #[test]
    fn a_request_without_executions_says_so() {
        for depth in [Depth::Compact, Depth::Full] {
            let doc = Doc::build(&row(), &[], 1_789_179_657, depth);
            let text = plain_text(&doc);
            assert!(text.contains("没有记录到上游执行"), "{depth:?}: {text}");
        }
    }

    #[test]
    fn buttons_hit_where_they_are_drawn() {
        let m = Popup::new(470.0, 430.0, 1.0);
        for button in [Button::Close, Button::Toggle, Button::Copy, Button::Open] {
            let r = button_rect(&m, button);
            let hit = hit_button(&m, r.x + r.w / 2.0, r.y + r.h / 2.0);
            assert_eq!(hit, Some(button), "{button:?} at {r:?}");
        }
        // The footer controls must not overlap: 完整信息 on the left, then 复制
        // and 打开网页 on the right.
        let toggle = button_rect(&m, Button::Toggle);
        let copy = button_rect(&m, Button::Copy);
        let open = button_rect(&m, Button::Open);
        assert!(copy.x > toggle.x + toggle.w, "{toggle:?} vs {copy:?}");
        assert!(open.x > copy.x + copy.w, "{copy:?} vs {open:?}");
        // The body is not a button.
        assert_eq!(hit_button(&m, m.width / 2.0, m.height / 2.0), None);
        // The toggle names what clicking it shows.
        assert_eq!(Button::Toggle.label(Depth::Compact), "完整信息");
        assert_eq!(Button::Toggle.label(Depth::Full), "简化信息");
    }

    #[test]
    fn scaled_popup_keeps_its_proportions() {
        let at_100 = Popup::new(470.0, 430.0, 1.0);
        let at_150 = Popup::new(705.0, 645.0, 1.5);
        assert_eq!(at_150.row_h(), at_100.row_h() * 1.5);
        assert_eq!(at_150.content_top(), at_100.content_top() * 1.5);
        let r = button_rect(&at_150, Button::Copy);
        assert!(r.x + r.w <= at_150.width - at_150.pad() + 0.01);
        assert!(r.y + r.h <= at_150.height + 0.01);
        // The close button sits inside the header.
        let c = button_rect(&at_150, Button::Close);
        assert!(c.y + c.h <= at_150.header_h() + 0.01);
    }
}
