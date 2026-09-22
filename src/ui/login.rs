//! In-panel sign-in form.
//!
//! Deliberately not a native dialog: a modal message loop would pump messages
//! meant for the panel and fight the always-on-top, no-activate window. The
//! form renders with the same primitives as the request cards and keeps all
//! input inside the panel's own message handling.

use crate::config::{Account, CredentialMode, Credentials};
use crate::theme::{self, Painter};
use crate::ui::layout::{Metrics, Rect};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    /// Name of the account being added or renamed.
    Name,
    Endpoint,
    Email,
    Password,
    Token,
    Remember,
}

/// Which credentials the form collects. Either a password or a token is enough
/// to obtain access; the token path avoids putting a password on disk at all.
///
/// Only `Token` is reachable from the form today — the password method is
/// hidden — but the variant and its handling are kept for when it returns.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    #[allow(dead_code)]
    Password,
    Token,
}

#[derive(Clone)]
pub struct LoginForm {
    /// Account management returns to the settings window instead of exiting.
    pub return_to_settings: bool,
    /// Label the account is saved under. Required: the menu lists accounts by
    /// name, so an unnamed one would be an unlabelled entry.
    pub account_name: String,
    /// The account being edited, or `None` when this is a new one.
    pub editing: Option<String>,
    pub endpoint: String,
    pub email: String,
    pub password: String,
    /// A pasted JWT, used when `method` is `Token`.
    pub token: String,
    pub method: Method,
    /// How (and whether) credentials are persisted.
    pub mode: CredentialMode,
    pub focus: Field,
    /// Caret position, in characters, within the focused field.
    pub caret: usize,
    /// Where the current selection was dragged from, in characters. `None`
    /// when the caret sits alone; the pair is normalised wherever it is read.
    pub anchor: Option<usize>,
    pub error: Option<String>,
    pub busy: bool,
    /// Blink phase for the caret, flipped by the UI timer.
    pub caret_on: bool,
}

/// The byte offset of the `chars`-th character, saturating at the end.
fn byte_at(s: &str, chars: usize) -> usize {
    s.char_indices()
        .nth(chars)
        .map(|(offset, _)| offset)
        .unwrap_or(s.len())
}

impl LoginForm {
    /// A form for a new account: nothing filled in but the endpoint the panel
    /// already knows, and the name field waiting for a label.
    pub fn adding(endpoint: &str, mode: CredentialMode) -> Self {
        let mut form = LoginForm::blank(endpoint, mode);
        form.focus = Field::Name;
        form
    }

    /// A form for an existing account: its name and endpoint are prefilled so
    /// signing in again is a paste of a fresh token.
    pub fn for_account(account: &Account, mode: CredentialMode) -> Self {
        let mut form = LoginForm::blank(&account.endpoint, mode);
        form.account_name = account.name.clone();
        form.editing = Some(account.id.clone());
        form.focus = Field::Token;
        form
    }

    fn blank(endpoint: &str, mode: CredentialMode) -> Self {
        // Token only: the password method is hidden from the form, so there is
        // nothing to choose and the token field takes the focus. The password
        // path — `Method::Password`, the email/password fields and everything
        // that submits them — is kept, just not reachable from here.
        LoginForm {
            return_to_settings: false,
            account_name: String::new(),
            editing: None,
            endpoint: endpoint.to_string(),
            email: String::new(),
            password: String::new(),
            token: String::new(),
            method: Method::Token,
            focus: Field::Token,
            caret: 0,
            anchor: None,
            error: None,
            busy: false,
            caret_on: true,
            mode,
        }
    }

    /// The value a field holds.
    pub fn value(&self, field: Field) -> &str {
        match field {
            Field::Name => &self.account_name,
            Field::Endpoint => &self.endpoint,
            Field::Email => &self.email,
            Field::Password => &self.password,
            Field::Token => &self.token,
            Field::Remember => "",
        }
    }

    fn value_mut(&mut self, field: Field) -> Option<&mut String> {
        match field {
            Field::Name => Some(&mut self.account_name),
            Field::Endpoint => Some(&mut self.endpoint),
            Field::Email => Some(&mut self.email),
            Field::Password => Some(&mut self.password),
            Field::Token => Some(&mut self.token),
            Field::Remember => None,
        }
    }

    /// Whether the field shows its value as asterisks.
    pub fn masked(field: Field) -> bool {
        field == Field::Password
    }

    /// The value as drawn: passwords become a run of asterisks.
    pub fn shown_value(&self, field: Field) -> String {
        let value = self.value(field);
        if Self::masked(field) {
            "*".repeat(value.chars().count())
        } else {
            value.to_string()
        }
    }

    /// Characters in the focused field.
    fn focused_len(&self) -> usize {
        self.value(self.focus).chars().count()
    }

    /// Move focus. Landing on a different field puts the caret at its end,
    /// which is where typing is expected to continue.
    pub fn focus_field(&mut self, field: Field) {
        if self.focus != field {
            self.focus = field;
            self.caret = self.focused_len();
            self.anchor = None;
        }
        self.caret_on = true;
    }

    /// The selection as an ordered (start, end) pair in characters, when the
    /// two ends are actually apart.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        let len = self.focused_len();
        let caret = self.caret.min(len);
        let anchor = anchor.min(len);
        (anchor != caret).then(|| (anchor.min(caret), anchor.max(caret)))
    }

    /// Place the caret, optionally dragging the selection anchor along with it
    /// (shift + arrows, or a mouse drag).
    pub fn set_caret(&mut self, index: usize, extend: bool) {
        let len = self.focused_len();
        let index = index.min(len);
        let previous = self.caret.min(len);
        if extend {
            self.anchor.get_or_insert(previous);
        } else {
            self.anchor = None;
        }
        self.caret = index;
        self.caret_on = true;
    }

    pub fn move_caret(&mut self, delta: isize, extend: bool) {
        let len = self.focused_len() as isize;
        let next = (self.caret as isize + delta).clamp(0, len) as usize;
        self.set_caret(next, extend);
    }

    /// Home (start = false) and End (start = true).
    pub fn caret_to_edge(&mut self, end: bool, extend: bool) {
        let len = self.focused_len();
        self.set_caret(if end { len } else { 0 }, extend);
    }

    /// Drop the caret where a mouse press landed and mark that spot as the
    /// start of a possible drag. A press that never moves leaves no selection
    /// behind, since the two ends coincide.
    pub fn press_caret(&mut self, index: usize) {
        self.set_caret(index, false);
        self.anchor = Some(self.caret);
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.caret = self.focused_len();
        self.caret_on = true;
    }

    /// Forget a selection whose ends sit on the same spot; a click that never
    /// became a drag must not leave a zero-width anchor behind.
    pub fn collapse_selection(&mut self) {
        if self.selection().is_none() {
            self.anchor = None;
        }
    }

    /// The selected text, for the clipboard.
    pub fn selected_text(&self) -> Option<String> {
        let (lo, hi) = self.selection()?;
        let value = self.value(self.focus);
        Some(value[byte_at(value, lo)..byte_at(value, hi)].to_string())
    }

    /// Cut: remove the selection and return what was removed. `None` means
    /// there was no selection to take.
    pub fn take_selection(&mut self) -> Option<String> {
        let text = self.selected_text()?;
        self.delete_selection();
        Some(text)
    }

    /// Delete the selected characters, leaving the caret where they started.
    /// Returns whether there was a selection to delete.
    fn delete_selection(&mut self) -> bool {
        let Some((lo, hi)) = self.selection() else {
            return false;
        };
        let field = self.focus;
        if let Some(text) = self.value_mut(field) {
            let (start, end) = (byte_at(text, lo), byte_at(text, hi));
            text.replace_range(start..end, "");
        }
        self.caret = lo;
        self.anchor = None;
        self.caret_on = true;
        true
    }

    pub fn set_mode(&mut self, mode: CredentialMode) {
        self.mode = mode;
        self.error = None;
    }

    pub fn credentials(&self) -> Credentials {
        Credentials {
            email: self.email.trim().to_string(),
            password: self.password.clone(),
        }
    }

    /// The pasted token, with any `Bearer ` prefix and whitespace removed.
    pub fn trimmed_token(&self) -> String {
        self.token
            .trim()
            .trim_start_matches("Bearer ")
            .trim()
            .to_string()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.account_name.trim().is_empty() {
            return Err("请填写账号名称".into());
        }
        if self.endpoint.trim().is_empty() {
            return Err("请填写 AxonHub 地址".into());
        }
        if !self.endpoint.trim().starts_with("http") {
            return Err("地址需要以 http:// 或 https:// 开头".into());
        }
        match self.method {
            Method::Password => {
                if self.email.trim().is_empty() {
                    return Err("邮箱不能为空".into());
                }
                if self.password.is_empty() {
                    return Err("密码不能为空".into());
                }
            }
            Method::Token => {
                let token = self.trimmed_token();
                if token.is_empty() {
                    return Err("请粘贴访问令牌".into());
                }
                if !crate::token::looks_like_jwt(&token) {
                    return Err("令牌格式不正确,应为三段以 . 分隔的字符串".into());
                }
                if crate::token::is_expired(&token, crate::time::now_unix()) {
                    return Err("该令牌已过期,请在 AxonHub 重新获取".into());
                }
            }
        }
        Ok(())
    }

    /// The text field currently receiving keystrokes.
    fn focused_text(&mut self) -> Option<&mut String> {
        self.value_mut(self.focus)
    }

    /// Maximum characters the focused field accepts. Endpoint and email have no
    /// legitimate need for long input; a JWT is a few hundred characters.
    fn char_limit(&self) -> usize {
        match self.focus {
            Field::Token => 4096,
            Field::Name => 64,
            _ => 256,
        }
    }

    /// Whether `ch` is allowed in the currently focused text field.
    ///
    /// Endpoint / email / token are URL- or JWT-shaped ASCII, so they keep the
    /// original ASCII-only policy. The password and account-name fields accept
    /// any printable Unicode (everything except control characters) so a
    /// non-ASCII password or a Chinese account name is not silently dropped.
    fn accepts(&self, ch: char) -> bool {
        match self.focus {
            Field::Password | Field::Name => !ch.is_control(),
            _ => ch.is_ascii_graphic() || ch == ' ',
        }
    }

    /// Type one character at the caret, replacing any selection. The caret
    /// advances with the text, the way an edit control behaves.
    pub fn insert(&mut self, ch: char) {
        if !self.accepts(ch) {
            return;
        }
        self.delete_selection();
        let limit = self.char_limit();
        let caret = self.caret.min(self.focused_len());
        let mut placed = false;
        if let Some(text) = self.focused_text() {
            if text.chars().count() < limit {
                let at = byte_at(text, caret);
                text.insert(at, ch);
                placed = true;
            }
        }
        if placed {
            self.caret = caret + 1;
        }
        self.caret_on = true;
        self.error = None;
    }

    /// Insert a pasted string at the caret, replacing any selection, filtered
    /// to characters the field can hold. A JWT copied from DevTools can arrive
    /// with surrounding whitespace or a `Bearer ` prefix, and may contain
    /// newlines if the copy spanned lines.
    pub fn paste(&mut self, raw: &str) {
        let cleaned: String = raw.chars().filter(|c| self.accepts(*c)).collect();
        // A pasted password is kept verbatim: only control characters were
        // filtered above, since a real password may legitimately contain
        // leading/trailing spaces. Other fields may carry surrounding
        // whitespace or a `Bearer ` prefix, so they keep the trim.
        let cleaned: &str = if self.focus == Field::Password {
            &cleaned
        } else {
            cleaned.trim()
        };
        if cleaned.is_empty() {
            return;
        }
        self.delete_selection();
        let limit = self.char_limit();
        let caret = self.caret.min(self.focused_len());
        let room = limit.saturating_sub(self.focused_len());
        let taken: String = cleaned.chars().take(room).collect();
        if taken.is_empty() {
            return;
        }
        if let Some(text) = self.focused_text() {
            let at = byte_at(text, caret);
            text.insert_str(at, &taken);
        }
        self.caret = caret + taken.chars().count();
        self.caret_on = true;
        self.error = None;
    }

    /// Empty the focused field (Ctrl+X with nothing selected).
    pub fn clear_focused(&mut self) {
        if let Some(text) = self.focused_text() {
            text.clear();
        }
        self.caret = 0;
        self.anchor = None;
        self.caret_on = true;
        self.error = None;
    }

    /// Delete the selection, or the character before the caret when there is
    /// none.
    pub fn backspace(&mut self) {
        if self.delete_selection() {
            self.error = None;
            return;
        }
        let caret = self.caret.min(self.focused_len());
        if caret == 0 {
            return;
        }
        if let Some(text) = self.focused_text() {
            let (start, end) = (byte_at(text, caret - 1), byte_at(text, caret));
            text.replace_range(start..end, "");
        }
        self.caret = caret - 1;
        self.caret_on = true;
        self.error = None;
    }

    /// Delete the selection, or the character under the caret when there is
    /// none.
    pub fn delete_forward(&mut self) {
        if self.delete_selection() {
            self.error = None;
            return;
        }
        let caret = self.caret.min(self.focused_len());
        if caret >= self.focused_len() {
            return;
        }
        if let Some(text) = self.focused_text() {
            let (start, end) = (byte_at(text, caret), byte_at(text, caret + 1));
            text.replace_range(start..end, "");
        }
        self.caret_on = true;
        self.error = None;
    }

    pub fn focus_next(&mut self, backwards: bool) {
        let order: &[Field] = match self.method {
            Method::Password => &[
                Field::Name,
                Field::Endpoint,
                Field::Email,
                Field::Password,
                Field::Remember,
            ],
            Method::Token => &[Field::Name, Field::Endpoint, Field::Token, Field::Remember],
        };
        let index = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        let len = order.len();
        let next = if backwards {
            (index + len - 1) % len
        } else {
            (index + 1) % len
        };
        self.focus_field(order[next]);
    }
}

/// Geometry of the sign-in form, recomputed per paint. The password and token
/// methods show a different set of fields, so the layout follows the method.
pub struct LoginLayout {
    pub card: Rect,
    /// (rect, label) for the fields currently shown.
    pub fields: Vec<(Rect, &'static str)>,
    pub mode_select: Rect,
    pub remember_box: Rect,
    pub button: Rect,
    pub error_y: f32,
}

/// Authored at 96 DPI; scaled by `Metrics::scale` in `layout`.
const FIELD_H: f32 = 34.0;
const LABEL_H: f32 = 18.0;
const FIELD_GAP: f32 = 14.0;
/// Title and subtitle, down to the first field label.
const HEAD_H: f32 = 62.0;
/// Everything below the last field: mode line, button and error line.
const TAIL_H: f32 = 118.0;

pub fn layout(form: &LoginForm, m: Metrics) -> LoginLayout {
    // The form is authored at 96 DPI and scales with the monitor so it stays
    // legible on a high-DPI display.
    let s = m.scale;
    let (field_h, label_h, field_gap) = (FIELD_H * s, LABEL_H * s, FIELD_GAP * s);

    let card_w = (m.width - m.pad() * 2.0).min(404.0 * s).max(280.0 * s);
    let card_x = (m.width - card_w) / 2.0;
    let inner_x = card_x + 18.0 * s;
    let inner_w = card_w - 36.0 * s;

    let has_credentials = form.method == Method::Password;
    let field_count = if has_credentials { 4.0 } else { 3.0 };
    let card_h = (HEAD_H + TAIL_H) * s + field_count * (label_h + field_h + field_gap);

    let card_y = ((m.height - card_h) / 2.0).max(16.0 * s);
    let card = Rect {
        x: card_x,
        y: card_y,
        w: card_w,
        h: card_h,
    };

    // Title and subtitle, down to where the first label starts. The method
    // tabs used to sit in here; with the password method hidden the form goes
    // straight to its fields.
    let mut y = card_y + HEAD_H * s;
    let mut fields = Vec::with_capacity(3);
    let push = |fields: &mut Vec<(Rect, &'static str)>, label: &'static str, y: &mut f32| {
        fields.push((
            Rect {
                x: inner_x,
                y: *y + label_h,
                w: inner_w,
                h: field_h,
            },
            label,
        ));
        *y += label_h + field_h + field_gap;
    };

    push(&mut fields, "账号名称", &mut y);
    push(&mut fields, "AxonHub 地址", &mut y);
    if has_credentials {
        push(&mut fields, "邮箱", &mut y);
        push(&mut fields, "密码", &mut y);
    } else {
        push(&mut fields, "访问令牌(JWT)  可 Ctrl+V 粘贴", &mut y);
    }

    let mode_select = Rect {
        x: inner_x,
        y: y + 2.0 * s,
        w: inner_w,
        h: 16.0 * s,
    };
    let remember_box = Rect {
        x: inner_x,
        y: y + 26.0 * s,
        w: 16.0 * s,
        h: 16.0 * s,
    };
    let button = Rect {
        x: inner_x,
        y: y + 56.0 * s,
        w: inner_w,
        h: 36.0 * s,
    };

    LoginLayout {
        card,
        fields,
        mode_select,
        remember_box,
        button,
        error_y: y + 98.0 * s,
    }
}

/// One text row of the form: what it holds and where the caret sits.
struct FieldView<'a> {
    label: &'static str,
    value: &'a str,
    placeholder: &'static str,
    masked: bool,
    /// Caret position in characters; only drawn for the focused row.
    caret: usize,
    /// Normalised selection (start, end) in characters, when one is active.
    selection: Option<(usize, usize)>,
}

/// Where the value sits inside its box: the left edge of the text area and its
/// width, both in device pixels.
fn text_area(rect: Rect) -> (f32, f32) {
    let s = rect.h / FIELD_H;
    (rect.x + 10.0 * s, rect.w - 20.0 * s)
}

/// Pixel width of the first `chars` characters of `shown`.
fn width_up_to(p: &Painter, fonts: &theme::Fonts, shown: &str, chars: usize) -> f32 {
    p.dual_measure(&fonts.body, &shown[..byte_at(shown, chars)])
}

/// How far the text is scrolled left so the caret stays in view. Long values
/// (a JWT is ~200 characters) therefore show their tail, while the caret can
/// still be moved back to reveal the rest.
fn scroll_offset(p: &Painter, fonts: &theme::Fonts, shown: &str, caret: usize, text_w: f32) -> f32 {
    (width_up_to(p, fonts, shown, caret) + CARET_MARGIN - text_w).max(0.0)
}

/// The character boundary under `local_x`, given in pixels from the start of
/// the text. Binary search: a long token would make a linear scan of every
/// prefix far too expensive to run on a mouse move.
fn index_at_x(p: &Painter, fonts: &theme::Fonts, shown: &str, local_x: f32) -> usize {
    let total = shown.chars().count();
    let (mut lo, mut hi) = (0usize, total + 1);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if width_up_to(p, fonts, shown, mid) <= local_x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Space kept between the caret and the right edge of the box.
const CARET_MARGIN: f32 = 8.0;

/// Draw a rounded text input, with its selection and caret when focused.
fn text_field(
    p: &Painter,
    fonts: &theme::Fonts,
    rect: Rect,
    row: &FieldView,
    focused: bool,
    caret_on: bool,
) {
    p.dual_text(
        &fonts.small,
        row.label,
        rect.x,
        rect.y - rect.h * (LABEL_H / FIELD_H),
        rect.w,
        rect.h * (LABEL_H / FIELD_H),
        if focused {
            theme::MAUVE
        } else {
            theme::TEXT_DIM
        },
        theme::ALIGN_NEAR,
    );

    let s = rect.h / FIELD_H;
    p.round_rect(
        rect.x,
        rect.y,
        rect.w,
        rect.h,
        6.0 * s,
        theme::INPUT_BG,
        true,
    );
    p.round_rect(
        rect.x,
        rect.y,
        rect.w,
        rect.h,
        6.0 * s,
        if focused {
            theme::MAUVE
        } else {
            theme::INPUT_BORDER
        },
        false,
    );

    let (text_x, text_w) = text_area(rect);
    let shown = if row.masked {
        "*".repeat(row.value.chars().count())
    } else {
        row.value.to_string()
    };
    let caret = row.caret.min(shown.chars().count());
    // An unfocused row always draws from the start; the focused one scrolls
    // just enough to keep the caret (and the text being typed) in view.
    let offset = if focused {
        scroll_offset(p, fonts, &shown, caret, text_w)
    } else {
        0.0
    };

    // Everything inside the field is clipped to it. The value may be wider
    // than the box and is drawn from a scrolled origin, so without the clip it
    // would spill out of the card.
    p.clip(theme::RectF {
        X: rect.x + 1.0,
        Y: rect.y,
        Width: rect.w - 2.0,
        Height: rect.h,
    });

    if shown.is_empty() {
        p.dual_text(
            &fonts.body,
            row.placeholder,
            text_x,
            rect.y,
            text_w,
            rect.h,
            theme::TEXT_FAINT,
            theme::ALIGN_NEAR,
        );
    } else {
        let origin = text_x - offset;
        if let Some((lo, hi)) = row.selection {
            let x1 = origin + width_up_to(p, fonts, &shown, lo);
            let x2 = origin + width_up_to(p, fonts, &shown, hi);
            p.fill_rect(
                x1,
                rect.y + 5.0 * s,
                (x2 - x1).max(1.0),
                rect.h - 10.0 * s,
                theme::SELECTION,
            );
        }
        // The full measured width (plus slack for `MeasureString` rounding
        // under what `DrawString` needs) so nothing is trimmed into an
        // ellipsis; the clip above cuts whatever runs past the box.
        let full = p.dual_measure(&fonts.body, &shown) + 4.0;
        p.dual_text(
            &fonts.body,
            &shown,
            origin,
            rect.y,
            full,
            rect.h,
            theme::TEXT,
            theme::ALIGN_NEAR,
        );
    }

    if focused && caret_on {
        let caret_x =
            (text_x - offset + width_up_to(p, fonts, &shown, caret)).min(rect.x + rect.w - 6.0 * s);
        p.fill_rect(
            caret_x,
            rect.y + 7.0 * s,
            1.5 * s,
            rect.h - 14.0 * s,
            theme::MAUVE,
        );
    }

    p.reset_clip();
}

/// Move the caret to the character under a mouse position. `press` starts a
/// new selection anchor there (button down); otherwise the existing anchor is
/// kept and the selection grows towards the pointer (drag). Returns whether
/// anything about the form changed, so a drag that stays within one character
/// does not repaint.
pub fn place_caret(
    p: &Painter,
    fonts: &theme::Fonts,
    form: &mut LoginForm,
    m: Metrics,
    x: f32,
    y: f32,
    press: bool,
) -> bool {
    let l = layout(form, m);
    let Some((field, rect)) = l.fields.iter().enumerate().find_map(|(i, (rect, _))| {
        rect.contains(x, y)
            .then_some((field_for(form.method, i), *rect))
    }) else {
        return false;
    };
    let before = (form.focus, form.caret, form.anchor);
    // The origin the row was last painted with: an unfocused row draws from
    // its start, the focused one is scrolled to keep its caret in view.
    let shown = form.shown_value(field);
    let (text_x, text_w) = text_area(rect);
    let offset = if form.focus == field {
        scroll_offset(p, fonts, &shown, form.caret, text_w)
    } else {
        0.0
    };
    form.focus_field(field);
    let index = index_at_x(p, fonts, &shown, x - (text_x - offset));
    if press {
        form.press_caret(index);
    } else {
        form.set_caret(index, true);
    }
    before != (form.focus, form.caret, form.anchor)
}

/// The placeholder a field shows while empty.
fn placeholder_for(field: Field) -> &'static str {
    match field {
        Field::Name => "例如:工作、个人",
        Field::Endpoint => "http://localhost:8090",
        Field::Email => "you@example.com",
        Field::Token => "eyJhbGciOi...",
        _ => "",
    }
}

/// Whether a point is over one of the text rows. Answered from the rectangles
/// alone, which is what lets the window show an I-beam without a painter.
pub fn hit_text_field(form: &LoginForm, m: Metrics, x: f32, y: f32) -> bool {
    layout(form, m)
        .fields
        .iter()
        .any(|(rect, _)| rect.contains(x, y))
}

pub fn draw(p: &Painter, fonts: &theme::Fonts, form: &LoginForm, m: Metrics) {
    let l = layout(form, m);
    let s = m.scale;
    let inset = 18.0 * s;

    p.fill_rect(0.0, 0.0, m.width, m.height, theme::BG);
    p.round_rect(
        l.card.x,
        l.card.y,
        l.card.w,
        l.card.h,
        10.0 * s,
        theme::CARD,
        true,
    );

    p.dual_text(
        &fonts.bold,
        "登录 AxonHub",
        l.card.x + inset,
        l.card.y + 14.0 * s,
        l.card.w - inset * 2.0,
        22.0 * s,
        theme::TEXT,
        theme::ALIGN_NEAR,
    );
    p.dual_text(
        &fonts.small,
        "可添加多个账号,请求合并显示;直接粘贴访问令牌即可",
        l.card.x + inset,
        l.card.y + 36.0 * s,
        l.card.w - inset * 2.0,
        18.0 * s,
        theme::TEXT_FAINT,
        theme::ALIGN_NEAR,
    );

    if form.return_to_settings {
        let r = back_button(form, m);
        p.dual_text(
            &fonts.small,
            "返回设置",
            r.x,
            r.y,
            r.w,
            r.h,
            theme::MAUVE,
            theme::ALIGN_FAR,
        );
    }

    // The method tabs are gone: the password method is hidden, so there is
    // nothing to choose. `Method` and everything behind it is still in place
    // should the choice ever come back.
    for (i, (rect, label)) in l.fields.iter().enumerate() {
        let field = field_for(form.method, i);
        let focused = form.focus == field;
        let row = FieldView {
            label,
            value: form.value(field),
            placeholder: placeholder_for(field),
            masked: LoginForm::masked(field),
            caret: form.caret,
            // A selection only ever belongs to the focused row.
            selection: if focused { form.selection() } else { None },
        };
        text_field(p, fonts, *rect, &row, focused, form.caret_on);
    }

    // Storage mode selector: a single line cycling through the three modes.
    let mode_focused = form.focus == Field::Remember;
    p.dual_text(
        &fonts.small,
        &format!("凭据:{}", form.mode.label()),
        l.mode_select.x,
        l.mode_select.y,
        160.0 * s,
        l.mode_select.h,
        if mode_focused {
            theme::MAUVE
        } else {
            theme::TEXT_DIM
        },
        theme::ALIGN_NEAR,
    );
    p.dual_text(
        &fonts.small,
        "点击切换",
        l.mode_select.x + l.mode_select.w - 60.0 * s,
        l.mode_select.y,
        60.0 * s,
        l.mode_select.h,
        theme::TEXT_FAINT,
        theme::ALIGN_FAR,
    );

    p.dual_text(
        &fonts.small,
        form.mode.hint(),
        l.mode_select.x,
        l.mode_select.y + 18.0 * s,
        l.mode_select.w,
        16.0 * s,
        theme::TEXT_FAINT,
        theme::ALIGN_NEAR,
    );

    let enabled = !form.busy;
    p.round_rect(
        l.button.x,
        l.button.y,
        l.button.w,
        l.button.h,
        7.0 * s,
        if enabled { 0xFF2E3644 } else { 0xFF23272E },
        true,
    );
    p.dual_text(
        &fonts.bold,
        if form.busy { "登录中…" } else { "登录" },
        l.button.x,
        l.button.y,
        l.button.w,
        l.button.h,
        if enabled {
            theme::TEXT
        } else {
            theme::TEXT_FAINT
        },
        theme::ALIGN_CENTER,
    );

    if let Some(err) = &form.error {
        p.dual_text(
            &fonts.small,
            err,
            l.card.x + inset,
            l.error_y,
            l.card.w - inset * 2.0,
            20.0 * s,
            theme::RED,
            theme::ALIGN_NEAR,
        );
    }
}

/// Map a click to a form control.
pub fn hit(form: &LoginForm, m: Metrics, x: f32, y: f32) -> Option<Action> {
    if form.return_to_settings && back_button(form, m).contains(x, y) {
        return Some(Action::Back);
    }
    let l = layout(form, m);
    if l.button.contains(x, y) {
        return Some(Action::Submit);
    }
    if l.mode_select.contains(x, y)
        || (l.remember_box.contains(x, y)
            || (y >= l.remember_box.y - 4.0
                && y <= l.remember_box.y + l.remember_box.h + 4.0
                && x >= l.remember_box.x
                && x <= l.remember_box.x + 320.0))
    {
        return Some(Action::CycleMode);
    }
    for (index, (rect, _)) in l.fields.iter().enumerate() {
        if rect.contains(x, y) {
            return Some(Action::Focus(field_for(form.method, index)));
        }
    }
    None
}

fn back_button(form: &LoginForm, m: Metrics) -> Rect {
    let card = layout(form, m).card;
    Rect {
        x: card.x + card.w - 104.0 * m.scale,
        y: card.y + 10.0 * m.scale,
        w: 86.0 * m.scale,
        h: 30.0 * m.scale,
    }
}

/// Which field a displayed row corresponds to. The token method shows one row
/// more than the password method, which has the two credential rows instead.
fn field_for(method: Method, index: usize) -> Field {
    match (method, index) {
        (_, 0) => Field::Name,
        (_, 1) => Field::Endpoint,
        (Method::Password, 2) => Field::Email,
        (Method::Password, _) => Field::Password,
        (Method::Token, _) => Field::Token,
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Back,
    Submit,
    CycleMode,
    /// Choosing a method is no longer possible from the form — the tabs are
    /// hidden — but the variant stays so the password method remains wired.
    #[allow(dead_code)]
    SetMethod(Method),
    Focus(Field),
}
