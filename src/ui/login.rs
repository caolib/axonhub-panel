//! In-panel sign-in form.
//!
//! Deliberately not a native dialog: a modal message loop would pump messages
//! meant for the panel and fight the always-on-top, no-activate window. The
//! form renders with the same primitives as the request cards and keeps all
//! input inside the panel's own message handling.

use crate::config::{CredentialMode, Credentials};
use crate::theme::{self, Painter};
use crate::ui::layout::{Metrics, Rect};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Endpoint,
    Email,
    Password,
    Token,
    Remember,
}

/// Which credentials the form collects. Either a password or a token is enough
/// to obtain access; the token path avoids putting a password on disk at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    Password,
    Token,
}

#[derive(Clone)]
pub struct LoginForm {
    pub endpoint: String,
    pub email: String,
    pub password: String,
    /// A pasted JWT, used when `method` is `Token`.
    pub token: String,
    pub method: Method,
    /// How (and whether) credentials are persisted.
    pub mode: CredentialMode,
    pub focus: Field,
    pub error: Option<String>,
    pub busy: bool,
    /// Blink phase for the caret, flipped by the UI timer.
    pub caret_on: bool,
}

impl LoginForm {
    pub fn new(endpoint: &str, prefill: Option<&Credentials>, mode: CredentialMode) -> Self {
        // Default to the token method: it is the primary path, since a pasted
        // token means the password is never given to the panel at all. Stored
        // credentials, when present, switch to the password form instead.
        let method = if prefill.is_some() {
            Method::Password
        } else {
            Method::Token
        };
        let focus = match method {
            Method::Password => Field::Email,
            Method::Token => Field::Token,
        };
        LoginForm {
            endpoint: endpoint.to_string(),
            email: prefill.map(|c| c.email.clone()).unwrap_or_default(),
            password: prefill.map(|c| c.password.clone()).unwrap_or_default(),
            token: String::new(),
            method,
            focus,
            error: None,
            busy: false,
            caret_on: true,
            mode,
        }
    }

    /// Start on the token tab, for when the caller already has one.
    pub fn with_token(mut self, token: &str) -> Self {
        self.token = token.to_string();
        self.method = Method::Token;
        self.focus = Field::Token;
        self
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
        match self.focus {
            Field::Endpoint => Some(&mut self.endpoint),
            Field::Email => Some(&mut self.email),
            Field::Password => Some(&mut self.password),
            Field::Token => Some(&mut self.token),
            Field::Remember => None,
        }
    }

    /// Maximum characters the focused field accepts. Endpoint and email have no
    /// legitimate need for long input; a JWT is a few hundred characters.
    fn char_limit(&self) -> usize {
        if self.focus == Field::Token {
            4096
        } else {
            256
        }
    }

    /// Whether `ch` is allowed in the currently focused text field.
    ///
    /// Endpoint / email / token are URL- or JWT-shaped ASCII, so they keep the
    /// original ASCII-only policy. The password field accepts any printable
    /// Unicode (everything except control characters) so non-ASCII passwords
    /// (Chinese, emoji, …) are no longer silently dropped.
    fn accepts(&self, ch: char) -> bool {
        match self.focus {
            Field::Password => !ch.is_control(),
            _ => ch.is_ascii_graphic() || ch == ' ',
        }
    }

    pub fn insert(&mut self, ch: char) {
        if !self.accepts(ch) {
            return;
        }
        let limit = self.char_limit();
        if let Some(text) = self.focused_text() {
            if text.chars().count() < limit {
                text.push(ch);
            }
        }
        self.error = None;
    }

    /// Insert a pasted string, filtered to characters the field can hold. A JWT
    /// copied from DevTools can arrive with surrounding whitespace or a
    /// `Bearer ` prefix, and may contain newlines if the copy spanned lines.
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
        let limit = self.char_limit();
        if let Some(text) = self.focused_text() {
            let room = limit.saturating_sub(text.chars().count());
            if room > 0 {
                text.extend(cleaned.chars().take(room));
            }
        }
        self.error = None;
    }

    /// Empty the focused field. The panel has no selection model, so Ctrl+A /
    /// Ctrl+X are treated as "replace this value".
    pub fn clear_focused(&mut self) {
        if let Some(text) = self.focused_text() {
            text.clear();
        }
        self.error = None;
    }

    pub fn backspace(&mut self) {
        if let Some(text) = self.focused_text() {
            text.pop();
        }
        self.error = None;
    }

    pub fn focus_next(&mut self, backwards: bool) {
        let order: &[Field] = match self.method {
            Method::Password => &[
                Field::Endpoint,
                Field::Email,
                Field::Password,
                Field::Remember,
            ],
            Method::Token => &[Field::Endpoint, Field::Token, Field::Remember],
        };
        let index = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        let len = order.len();
        let next = if backwards {
            (index + len - 1) % len
        } else {
            (index + 1) % len
        };
        self.focus = order[next];
        self.caret_on = true;
    }
}

/// Geometry of the sign-in form, recomputed per paint. The password and token
/// methods show a different set of fields, so the layout follows the method.
pub struct LoginLayout {
    pub card: Rect,
    pub tabs: [Rect; 2],
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
    let field_count = if has_credentials { 3.0 } else { 2.0 };
    // Title + subtitle + tabs + fields + mode line + button + error line.
    let card_h = 92.0 * s + field_count * (label_h + field_h + field_gap) + 96.0 * s;

    let card_y = ((m.height - card_h) / 2.0).max(16.0 * s);
    let card = Rect {
        x: card_x,
        y: card_y,
        w: card_w,
        h: card_h,
    };

    let tab_h = 28.0 * s;
    let tab_y = card_y + 62.0 * s;
    let tab_w = (inner_w - 8.0 * s) / 2.0;
    let tabs = [
        Rect {
            x: inner_x,
            y: tab_y,
            w: tab_w,
            h: tab_h,
        },
        Rect {
            x: inner_x + tab_w + 8.0 * s,
            y: tab_y,
            w: tab_w,
            h: tab_h,
        },
    ];
    let mut y = tab_y + tab_h + 16.0 * s;
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
        tabs,
        fields,
        mode_select,
        remember_box,
        button,
        error_y: y + 98.0 * s,
    }
}

/// Draw a rounded text input, returning nothing; the caret is drawn by callers
/// that know the field is focused.
#[allow(clippy::too_many_arguments)]
fn text_field(
    p: &Painter,
    fonts: &theme::Fonts,
    rect: Rect,
    label: &str,
    value: &str,
    placeholder: &str,
    focused: bool,
    masked: bool,
    caret_on: bool,
) {
    p.dual_text(
        &fonts.small,
        label,
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
    p.round_rect(rect.x, rect.y, rect.w, rect.h, 6.0 * s, theme::BG, true);
    p.round_rect(
        rect.x,
        rect.y,
        rect.w,
        rect.h,
        6.0 * s,
        if focused { theme::MAUVE } else { theme::BORDER },
        false,
    );

    let text_x = rect.x + 10.0 * s;
    let text_w = rect.w - 20.0 * s;

    if value.is_empty() {
        p.dual_text(
            &fonts.body,
            placeholder,
            text_x,
            rect.y,
            text_w,
            rect.h,
            theme::TEXT_FAINT,
            theme::ALIGN_NEAR,
        );
    }

    let shown: String = if masked {
        "*".repeat(value.chars().count())
    } else {
        value.to_string()
    };

    if !shown.is_empty() {
        // For long values (a JWT is ~200 chars) show the tail so the end of the
        // token stays visible while typing.
        let measured = p.dual_measure(&fonts.body, &shown);
        let offset = (measured - text_w).max(0.0);
        p.dual_text(
            &fonts.body,
            &shown,
            text_x - offset,
            rect.y,
            text_w + offset,
            rect.h,
            theme::TEXT,
            theme::ALIGN_NEAR,
        );
        if focused && caret_on {
            let caret_x = (text_x + measured.min(text_w) + 1.0).min(rect.x + rect.w - 6.0 * s);
            p.fill_rect(
                caret_x,
                rect.y + 7.0 * s,
                1.5 * s,
                rect.h - 14.0 * s,
                theme::MAUVE,
            );
        }
    } else if focused && caret_on {
        p.fill_rect(
            text_x,
            rect.y + 7.0 * s,
            1.5 * s,
            rect.h - 14.0 * s,
            theme::MAUVE,
        );
    }
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
        "可直接粘贴访问令牌,无需保存账号密码",
        l.card.x + inset,
        l.card.y + 36.0 * s,
        l.card.w - inset * 2.0,
        18.0 * s,
        theme::TEXT_FAINT,
        theme::ALIGN_NEAR,
    );

    // Method tabs
    let labels = [("账号密码", Method::Password), ("访问令牌", Method::Token)];
    for (i, (label, method)) in labels.iter().enumerate() {
        let rect = l.tabs[i];
        let selected = form.method == *method;
        p.round_rect(
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            6.0 * s,
            if selected { 0xFF2A3140 } else { theme::BG },
            true,
        );
        p.dual_text(
            &fonts.body,
            label,
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            if selected {
                theme::TEXT
            } else {
                theme::TEXT_DIM
            },
            theme::ALIGN_CENTER,
        );
    }

    let values: [&str; 3] = [&form.endpoint, &form.email, &form.password];
    let kinds = [Field::Endpoint, Field::Email, Field::Password];
    let token_index = if form.method == Method::Password {
        3
    } else {
        1
    };

    for (i, (rect, label)) in l.fields.iter().enumerate() {
        let is_token = i == token_index && form.method == Method::Token;
        let field = if is_token { Field::Token } else { kinds[i] };
        let (value, placeholder, masked) = if is_token {
            (form.token.as_str(), "eyJhbGciOi...", false)
        } else {
            match field {
                Field::Endpoint => (values[0], "http://localhost:8090", false),
                Field::Email => (values[1], "you@example.com", false),
                _ => (values[2], "", true),
            }
        };
        text_field(
            p,
            fonts,
            *rect,
            label,
            value,
            placeholder,
            form.focus == field,
            masked,
            form.caret_on,
        );
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
    let l = layout(form, m);
    if l.button.contains(x, y) {
        return Some(Action::Submit);
    }
    if l.tabs[0].contains(x, y) {
        return Some(Action::SetMethod(Method::Password));
    }
    if l.tabs[1].contains(x, y) {
        return Some(Action::SetMethod(Method::Token));
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

/// Which field a displayed row corresponds to. The token method shows two
/// fields where the password method shows three.
fn field_for(method: Method, index: usize) -> Field {
    match (method, index) {
        (_, 0) => Field::Endpoint,
        (Method::Password, 1) => Field::Email,
        (Method::Password, _) => Field::Password,
        (Method::Token, _) => Field::Token,
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Submit,
    CycleMode,
    SetMethod(Method),
    Focus(Field),
}
