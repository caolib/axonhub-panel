//! Settings content and geometry, shared by painting, pointer and keyboard input.

use crate::app::{App, ProbeState};
use crate::config::{CredentialMode, DisplayField, HiddenChannel};
use crate::font_catalog::InstalledFont;
use crate::theme::{self, Fonts, Painter};
use crate::ui::layout::Rect;

pub const WIDTH: f32 = 560.0;
pub const HEIGHT: f32 = 650.0;
pub const MIN_WIDTH: f32 = 460.0;
pub const MIN_HEIGHT: f32 = 420.0;
const TOP: f32 = 112.0;
const FOOTER: f32 = 70.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Display,
    Fields,
    Fonts,
    Accounts,
    Channels,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Tab(Tab),
    Rows(usize),
    Field(DisplayField),
    ShowAllFields,
    Font(f32),
    FontFamily(String),
    FontSearch,
    SingleLine,
    Pin,
    Topmost,
    Bottommost,
    AddAccount,
    Login(String),
    /// Ask the worker to prove one account's credentials still work.
    TestAccount(String),
    RemoveAccount(String),
    CredentialMode(CredentialMode),
    ClearCredentials,
    Channel(HiddenChannel),
    ClearChannels,
    Refresh,
    OpenRequests,
    Quit,
    Close,
    Confirm,
    Cancel,
}

#[derive(Default)]
pub struct State {
    pub tab: Tab,
    /// Logical pixels, independent of the panel's font-size setting.
    pub scroll: f32,
    pub hover: Option<Action>,
    pub focus: Option<Action>,
    pub pressed: Option<Action>,
    pub pending: Option<Action>,
    pub font_names: Vec<InstalledFont>,
    pub font_matches: Vec<usize>,
    pub font_query: String,
}

impl State {
    pub fn set_font_query(&mut self, query: String) {
        let lower = query.to_lowercase();
        self.font_matches = self
            .font_names
            .iter()
            .enumerate()
            .filter_map(|(i, font)| font.matches(&lower).then_some(i))
            .collect();
        self.font_query = query;
        self.scroll = 0.0;
        self.hover = None;
        self.pressed = None;
    }
}

pub struct Control {
    pub rect: Rect,
    pub label: String,
    pub action: Action,
    pub selected: bool,
    pub danger: bool,
    /// Content controls scroll; header and footer controls stay in place.
    pub body: bool,
    /// A checkbox row: label left-aligned, state carried by the ☑/☐ prefix.
    pub check: bool,
}

struct Label {
    rect: Rect,
    text: String,
    heading: bool,
    /// Overrides the default text colour, for labels that carry a verdict
    /// (green when an account answers, red when it does not).
    color: Option<u32>,
}

pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub controls: Vec<Control>,
    labels: Vec<Label>,
    pub content_height: f32,
    content_top: f32,
    font_header: Option<String>,
}

impl Layout {
    pub fn new(app: &App, state: &State, width: f32, height: f32) -> Self {
        let mut l = Self {
            width,
            height,
            controls: Vec::new(),
            labels: Vec::new(),
            content_height: 0.0,
            content_top: if state.tab == Tab::Fonts {
                TOP + 112.0
            } else {
                TOP
            },
            font_header: None,
        };
        let tab_w = (width - 56.0) / 5.0;
        for (i, (tab, label)) in [
            (Tab::Display, "显示与窗口".to_string()),
            (Tab::Fields, "属性显示".to_string()),
            (Tab::Fonts, "字体".to_string()),
            (
                Tab::Accounts,
                format!("账号 · {}", app.accounts.len()),
            ),
            (Tab::Channels, "渠道过滤".to_string()),
        ]
        .into_iter()
        .enumerate()
        {
            l.button(
                Rect {
                    x: 20.0 + i as f32 * (tab_w + 4.0),
                    y: 66.0,
                    w: tab_w,
                    h: 32.0,
                },
                label,
                Action::Tab(tab),
                state.tab == tab,
                false,
                false,
            );
        }
        match state.tab {
            Tab::Display => l.display(app),
            Tab::Fields => l.fields(app),
            Tab::Fonts => l.fonts(app, state),
            Tab::Accounts => l.accounts(app),
            Tab::Channels => l.channels(app),
        }
        let y = height - 40.0;
        if state.pending.is_some() {
            l.button(
                Rect {
                    x: width - 194.0,
                    y,
                    w: 82.0,
                    h: 28.0,
                },
                "取消",
                Action::Cancel,
                false,
                false,
                false,
            );
            l.button(
                Rect {
                    x: width - 104.0,
                    y,
                    w: 84.0,
                    h: 28.0,
                },
                "确认操作",
                Action::Confirm,
                false,
                true,
                false,
            );
        } else {
            l.button(
                Rect {
                    x: 20.0,
                    y,
                    w: 90.0,
                    h: 28.0,
                },
                "退出面板",
                Action::Quit,
                false,
                false,
                false,
            );
            l.button(
                Rect {
                    x: width - 104.0,
                    y,
                    w: 84.0,
                    h: 28.0,
                },
                "完成",
                Action::Close,
                true,
                false,
                false,
            );
        }
        l
    }

    fn button(
        &mut self,
        rect: Rect,
        label: impl Into<String>,
        action: Action,
        selected: bool,
        danger: bool,
        body: bool,
    ) {
        self.controls.push(Control {
            rect,
            label: label.into(),
            action,
            selected,
            danger,
            body,
            check: false,
        });
    }

    /// A two-column checkbox row: `checked` means the cell is shown on cards.
    fn checkbox(&mut self, rect: Rect, label: impl Into<String>, action: Action, checked: bool) {
        self.controls.push(Control {
            rect,
            label: format!("{}  {}", if checked { "☑" } else { "☐" }, label.into()),
            action,
            selected: checked,
            danger: false,
            body: true,
            check: true,
        });
    }

    /// Lay checkbox rows out in `columns` columns.
    fn check_grid(&mut self, y: f32, rows: Vec<(String, Action, bool)>, columns: usize) {
        let w = (self.width - 48.0 - (columns - 1) as f32 * 6.0) / columns as f32;
        for (i, (label, action, checked)) in rows.into_iter().enumerate() {
            self.checkbox(
                Rect {
                    x: 24.0 + (i % columns) as f32 * (w + 6.0),
                    y: y + (i / columns) as f32 * 36.0,
                    w,
                    h: 30.0,
                },
                label,
                action,
                checked,
            );
        }
    }

    fn label(&mut self, y: f32, text: impl Into<String>, heading: bool) {
        self.label_colored(y, text, heading, None);
    }

    fn label_colored(&mut self, y: f32, text: impl Into<String>, heading: bool, color: Option<u32>) {
        self.labels.push(Label {
            rect: Rect {
                x: 24.0,
                y,
                w: self.width - 48.0,
                h: 24.0,
            },
            text: text.into(),
            heading,
            color,
        });
    }

    fn choices(&mut self, y: f32, choices: Vec<(String, Action, bool)>, columns: usize) {
        let w = (self.width - 48.0 - (columns - 1) as f32 * 6.0) / columns as f32;
        for (i, (label, action, selected)) in choices.into_iter().enumerate() {
            self.button(
                Rect {
                    x: 24.0 + (i % columns) as f32 * (w + 6.0),
                    y: y + (i / columns) as f32 * 36.0,
                    w,
                    h: 30.0,
                },
                label,
                action,
                selected,
                false,
                true,
            );
        }
    }

    fn toggle(&mut self, y: f32, label: &str, hint: &str, action: Action, on: bool) {
        self.label(y, label, true);
        self.label(y + 22.0, hint, false);
        // Leave room for the switch even at the minimum window width.
        for label in self.labels.iter_mut().rev().take(2) {
            label.rect.w -= 100.0;
        }
        self.button(
            Rect {
                x: self.width - 104.0,
                y: y + 6.0,
                w: 80.0,
                h: 30.0,
            },
            if on { "已开启" } else { "已关闭" },
            action,
            on,
            false,
            true,
        );
    }

    fn display(&mut self, app: &App) {
        let c = &app.config;
        self.label(0.0, format!("显示数量 · 当前 {} 条", c.row_limit), true);
        self.choices(
            32.0,
            [5, 10, 15, 20]
                .into_iter()
                .map(|n| (format!("{n} 条"), Action::Rows(n), c.row_limit == n as i64))
                .collect(),
            4,
        );
        self.label(80.0, format!("字号 · 当前 {} px", c.font_size), true);
        let mut sizes = vec![(
            "12.5".into(),
            Action::Font(12.5),
            (c.font_size - 12.5).abs() < 0.01,
        )];
        sizes.extend((10..=24).map(|n| {
            (
                n.to_string(),
                Action::Font(n as f32),
                (c.font_size - n as f32).abs() < 0.01,
            )
        }));
        self.choices(112.0, sizes, 8);
        self.toggle(
            200.0,
            "单行模式",
            "压缩卡片高度，保持显示条数",
            Action::SingleLine,
            c.single_line,
        );
        self.toggle(
            262.0,
            "固定窗口位置",
            "锁定面板的移动和大小调整",
            Action::Pin,
            c.pin_position,
        );
        self.toggle(
            324.0,
            "置顶",
            "让面板保持在其他窗口上方",
            Action::Topmost,
            c.always_on_top,
        );
        self.toggle(
            386.0,
            "置底",
            "让面板保持在其他窗口下方",
            Action::Bottommost,
            c.always_on_bottom,
        );
        self.label(456.0, "常用操作", true);
        self.choices(
            488.0,
            vec![
                ("刷新请求".into(), Action::Refresh, false),
                ("打开请求页".into(), Action::OpenRequests, false),
            ],
            2,
        );
        self.content_height = 536.0;
    }

    fn fields(&mut self, app: &App) {
        let hidden = app.config.hidden_fields.len();
        self.label(0.0, format!("属性显示 · 已隐藏 {hidden} 项"), true);
        self.label(26.0, "取消勾选可在卡片上隐藏对应信息，默认全部显示", false);
        self.button(
            Rect {
                x: 24.0,
                y: 62.0,
                w: 144.0,
                h: 30.0,
            },
            "全部显示",
            Action::ShowAllFields,
            false,
            false,
            true,
        );
        let rows = DisplayField::ALL
            .into_iter()
            .map(|field| {
                (
                    field.label().to_string(),
                    Action::Field(field),
                    app.config.shows(field),
                )
            })
            .collect();
        self.check_grid(108.0, rows, 2);
        // Seven rows of checkboxes: the button above plus 7*36 of grid.
        self.content_height = 108.0 + 7.0 * 36.0 + 12.0;
    }

    fn accounts(&mut self, app: &App) {
        self.label(0.0, "账号管理", true);
        self.label(26.0, "多个账号的请求会合并显示", false);
        self.button(
            Rect {
                x: self.width - 120.0,
                y: 0.0,
                w: 96.0,
                h: 30.0,
            },
            "添加账号",
            Action::AddAccount,
            false,
            false,
            true,
        );
        let mut y = 68.0;
        if app.accounts.is_empty() {
            self.label(y, "暂无账号，点击「添加账号」连接 AxonHub", false);
            y += 44.0;
        }
        for account in &app.accounts {
            self.label(y, &account.name, true);
            self.label(y + 22.0, &account.endpoint, false);
            // A test result, while it is the freshest thing known about the
            // account, stands in for the poll's own verdict.
            let probe = app.probe.as_ref().filter(|p| p.id == account.id);
            let (status, status_color) = match probe.map(|p| &p.state) {
                Some(ProbeState::Running) => ("正在测试连接…".to_string(), None),
                Some(ProbeState::Ok(total)) => (
                    format!("连接正常 · 共 {total} 条请求"),
                    Some(theme::GREEN),
                ),
                Some(ProbeState::Failed(message)) => (
                    format!("测试失败 · {message}"),
                    Some(theme::RED),
                ),
                None => (
                    app.account_state(&account.id)
                        .unwrap_or_else(|| {
                            if app.open_accounts.contains(&account.id) {
                                "已连接"
                            } else {
                                "等待连接"
                            }
                        })
                        .to_string(),
                    None,
                ),
            };
            self.label_colored(y + 44.0, status, false, status_color);
            for label in self.labels.iter_mut().rev().take(3) {
                label.rect.w -= 208.0;
            }
            self.button(
                Rect {
                    x: self.width - 224.0,
                    y,
                    w: 96.0,
                    h: 30.0,
                },
                "测试连接",
                Action::TestAccount(account.id.clone()),
                false,
                false,
                true,
            );
            self.button(
                Rect {
                    x: self.width - 120.0,
                    y,
                    w: 96.0,
                    h: 30.0,
                },
                "编辑 / 登录",
                Action::Login(account.id.clone()),
                false,
                false,
                true,
            );
            self.button(
                Rect {
                    x: self.width - 120.0,
                    y: y + 38.0,
                    w: 96.0,
                    h: 28.0,
                },
                "删除账号",
                Action::RemoveAccount(account.id.clone()),
                false,
                true,
                true,
            );
            y += 92.0;
        }
        self.label(y, "凭据保存方式", true);
        self.choices(
            y + 32.0,
            [
                CredentialMode::None,
                CredentialMode::Token,
                CredentialMode::Password,
            ]
            .into_iter()
            .map(|mode| {
                (
                    mode.label().into(),
                    Action::CredentialMode(mode),
                    app.config.credential_mode == mode,
                )
            })
            .collect(),
            3,
        );
        self.label(
            y + 70.0,
            "保存方式适用于所有账号，凭据由 Windows 加密",
            false,
        );
        self.label(
            y + 96.0,
            "清除凭据会退出所有账号，保留账号名称与地址",
            false,
        );
        self.button(
            Rect {
                x: 24.0,
                y: y + 130.0,
                w: 170.0,
                h: 32.0,
            },
            "清除保存的凭据",
            Action::ClearCredentials,
            false,
            true,
            true,
        );
        self.content_height = y + 182.0;
    }

    fn fonts(&mut self, app: &App, state: &State) {
        let current = if app.config.font_family.is_empty() {
            "自动选择默认字体"
        } else {
            &app.config.font_family
        };
        self.font_header = Some(format!("当前字体 · {current}"));
        self.button(
            Rect {
                x: 24.0,
                y: TOP + 48.0,
                w: self.width - 48.0,
                h: 32.0,
            },
            "",
            Action::FontSearch,
            false,
            false,
            false,
        );
        self.button(
            Rect {
                x: 24.0,
                y: 0.0,
                w: self.width - 48.0,
                h: 32.0,
            },
            "使用默认字体",
            Action::FontFamily(String::new()),
            app.config.font_family.is_empty(),
            false,
            true,
        );
        let mut y = 42.0;
        for &index in &state.font_matches {
            let font = &state.font_names[index];
            self.button(
                Rect {
                    x: 24.0,
                    y,
                    w: self.width - 48.0,
                    h: 32.0,
                },
                &font.name,
                Action::FontFamily(font.name.clone()),
                app.config.font_family == font.name,
                false,
                true,
            );
            y += 38.0;
        }
        if state.font_matches.is_empty() {
            self.label(y, "没有匹配的本地字体，请换一个名称搜索", false);
            y += 32.0;
        }
        self.content_height = y + 12.0;
    }

    pub fn search_rect(&self) -> Option<Rect> {
        self.controls
            .iter()
            .find(|c| c.action == Action::FontSearch)
            .map(|c| c.rect)
    }

    fn channels(&mut self, app: &App) {
        self.label(
            0.0,
            format!("渠道过滤 · 已隐藏 {} 项", app.config.hidden_channels.len()),
            true,
        );
        self.label(26.0, "选中后隐藏该账号的渠道请求，避免转发重复显示", false);
        self.button(
            Rect {
                x: 24.0,
                y: 62.0,
                w: 144.0,
                h: 30.0,
            },
            "清除渠道过滤",
            Action::ClearChannels,
            false,
            false,
            true,
        );
        let filters = app.channel_filters();
        if filters.is_empty() {
            self.label(112.0, "暂无可过滤渠道，连接账号并刷新请求后会显示", false);
        }
        let mut y = 108.0;
        for entry in filters {
            let name = app
                .config
                .account(&entry.account_id)
                .map(|a| a.name.as_str())
                .unwrap_or(&entry.account_id);
            let hidden = app.config.hides_channel(&entry.account_id, &entry.channel);
            self.button(
                Rect {
                    x: 24.0,
                    y,
                    w: self.width - 48.0,
                    h: 36.0,
                },
                format!(
                    "{}  {name} · {}",
                    if hidden { "☑" } else { "☐" },
                    entry.channel
                ),
                Action::Channel(entry),
                hidden,
                false,
                true,
            );
            y += 44.0;
        }
        self.content_height = (y + 16.0).max(160.0);
    }

    pub fn view_height(&self) -> f32 {
        (self.height - self.content_top - FOOTER).max(0.0)
    }

    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.view_height()).max(0.0)
    }

    fn screen_rect(&self, control: &Control, scroll: f32) -> Rect {
        let mut rect = control.rect;
        if control.body {
            rect.y += self.content_top - scroll.clamp(0.0, self.max_scroll());
        }
        rect
    }

    pub fn hit(&self, state: &State, x: f32, y: f32) -> Option<Action> {
        self.controls
            .iter()
            .find(|c| {
                self.available(state, c)
                    && (!c.body || (y >= self.content_top && y < self.height - FOOTER))
                    && self.screen_rect(c, state.scroll).contains(x, y)
            })
            .map(|c| c.action.clone())
    }

    fn available(&self, state: &State, control: &Control) -> bool {
        state.pending.is_none() || matches!(control.action, Action::Confirm | Action::Cancel)
    }

    pub fn focus_next(&self, state: &mut State, backwards: bool) {
        let controls: Vec<_> = self
            .controls
            .iter()
            .filter(|c| self.available(state, c))
            .collect();
        if controls.is_empty() {
            return;
        }
        let index = controls
            .iter()
            .position(|c| Some(&c.action) == state.focus.as_ref());
        let next = match index {
            Some(i) if backwards => (i + controls.len() - 1) % controls.len(),
            Some(i) => (i + 1) % controls.len(),
            None if backwards => controls.len() - 1,
            None => 0,
        };
        let c = controls[next];
        state.focus = Some(c.action.clone());
        if c.body {
            state.scroll = state
                .scroll
                .min(c.rect.y)
                .max(c.rect.y + c.rect.h - self.view_height())
                .clamp(0.0, self.max_scroll());
        }
    }

    pub fn move_font_focus(&self, state: &mut State, backwards: bool) {
        let fonts: Vec<_> = self
            .controls
            .iter()
            .filter(|c| matches!(c.action, Action::FontFamily(_)))
            .collect();
        if fonts.is_empty() {
            return;
        }
        let current = fonts
            .iter()
            .position(|c| Some(&c.action) == state.focus.as_ref());
        let index = match current {
            Some(i) if backwards => i.saturating_sub(1),
            Some(i) => (i + 1).min(fonts.len() - 1),
            None => usize::from(fonts.len() > 1),
        };
        let control = fonts[index];
        state.focus = Some(control.action.clone());
        state.scroll = state
            .scroll
            .min(control.rect.y)
            .max(control.rect.y + control.rect.h - self.view_height())
            .clamp(0.0, self.max_scroll());
    }

    pub fn draw(&self, p: &Painter, fonts: &Fonts, state: &State, scale: f32) {
        let text = |rect: Rect, label: &str, heading: bool, color, align| {
            p.dual_text(
                if heading { &fonts.bold } else { &fonts.small },
                label,
                rect.x * scale,
                rect.y * scale,
                rect.w * scale,
                rect.h * scale,
                color,
                align,
            );
        };
        p.fill_rect(0.0, 0.0, self.width * scale, self.height * scale, theme::BG);
        text(
            Rect {
                x: 24.0,
                y: 14.0,
                w: self.width - 48.0,
                h: 24.0,
            },
            "面板设置",
            true,
            theme::TEXT,
            theme::ALIGN_NEAR,
        );
        text(
            Rect {
                x: 24.0,
                y: 38.0,
                w: self.width - 48.0,
                h: 20.0,
            },
            "AxonHub · 调整显示、管理账号与请求过滤",
            false,
            theme::TEXT_DIM,
            theme::ALIGN_NEAR,
        );
        if let Some(header) = &self.font_header {
            text(
                Rect {
                    x: 24.0,
                    y: TOP,
                    w: self.width - 48.0,
                    h: 24.0,
                },
                header,
                true,
                theme::TEXT,
                theme::ALIGN_NEAR,
            );
            text(
                Rect {
                    x: 24.0,
                    y: TOP + 24.0,
                    w: self.width - 48.0,
                    h: 20.0,
                },
                "搜索本地字体（Ctrl+F），支持中文和英文名称",
                false,
                theme::TEXT_DIM,
                theme::ALIGN_NEAR,
            );
            let count = format!(
                "匹配 {} / {} 款字体 · 点击或按 Enter 应用",
                state.font_matches.len(),
                state.font_names.len()
            );
            text(
                Rect {
                    x: 24.0,
                    y: TOP + 84.0,
                    w: self.width - 48.0,
                    h: 24.0,
                },
                &count,
                false,
                theme::TEXT_DIM,
                theme::ALIGN_NEAR,
            );
        }
        p.clip(theme::RectF {
            X: 0.0,
            Y: self.content_top * scale,
            Width: self.width * scale,
            Height: self.view_height() * scale,
        });
        let scroll = state.scroll.clamp(0.0, self.max_scroll());
        for label in &self.labels {
            let mut r = label.rect;
            r.y += self.content_top - scroll;
            text(
                r,
                &label.text,
                label.heading,
                label.color.unwrap_or(if label.heading {
                    theme::TEXT
                } else {
                    theme::TEXT_DIM
                }),
                theme::ALIGN_NEAR,
            );
        }
        for body in [true, false] {
            if !body {
                p.reset_clip();
            }
            for c in self.controls.iter().filter(|c| c.body == body) {
                if c.action == Action::FontSearch {
                    continue;
                }
                let r = self.screen_rect(c, state.scroll);
                if body && (r.y + r.h <= self.content_top || r.y >= self.height - FOOTER) {
                    continue;
                }
                let enabled = self.available(state, c);
                let hover = enabled && state.hover.as_ref() == Some(&c.action);
                let focused = enabled && state.focus.as_ref() == Some(&c.action);
                // A checkbox row keeps the card surface: its state is already
                // legible from the ☑/☐ prefix, so a selected fill would only
                // add noise to a list of thirteen.
                let fill = if !c.check && c.selected {
                    theme::TAB_ON
                } else if hover {
                    theme::CARD_HOVER
                } else {
                    theme::CARD
                };
                p.round_rect(
                    r.x * scale,
                    r.y * scale,
                    r.w * scale,
                    r.h * scale,
                    5.0 * scale,
                    fill,
                    true,
                );
                p.round_rect(
                    r.x * scale,
                    r.y * scale,
                    r.w * scale,
                    r.h * scale,
                    5.0 * scale,
                    if focused {
                        theme::MAUVE
                    } else if c.selected && !c.check {
                        theme::INPUT_BORDER
                    } else {
                        theme::BORDER
                    },
                    false,
                );
                let color = if !enabled {
                    theme::TEXT_FAINT
                } else if c.danger {
                    theme::RED
                } else if c.check {
                    if c.selected {
                        theme::TEXT
                    } else {
                        theme::TEXT_FAINT
                    }
                } else if c.selected {
                    theme::MAUVE
                } else {
                    theme::TEXT
                };
                let (label_x, align) = if c.check {
                    (r.x + 10.0, theme::ALIGN_NEAR)
                } else {
                    (r.x + 5.0, theme::ALIGN_CENTER)
                };
                text(
                    Rect {
                        x: label_x,
                        w: r.w - 15.0,
                        ..r
                    },
                    &c.label,
                    false,
                    color,
                    align,
                );
            }
        }
        let footer_y = self.height - FOOTER;
        p.fill_rect(
            0.0,
            footer_y * scale,
            self.width * scale,
            scale,
            theme::BORDER,
        );
        let hint = match state.pending {
            Some(Action::RemoveAccount(_)) => "确认删除该账号及其本机凭据？此操作无法撤销。",
            Some(Action::ClearCredentials) => "确认清除所有账号的凭据？之后需要重新登录。",
            _ => "设置即时生效并自动保存 · 失焦或 Esc 关闭",
        };
        text(
            Rect {
                x: 24.0,
                y: footer_y + 4.0,
                w: self.width - 48.0,
                h: 22.0,
            },
            hint,
            false,
            theme::TEXT_DIM,
            theme::ALIGN_NEAR,
        );
        if self.max_scroll() > 0.0 {
            let view = self.view_height();
            let thumb = (view * view / self.content_height).max(24.0).min(view);
            let y = self.content_top + (view - thumb) * scroll / self.max_scroll();
            p.round_rect(
                (self.width - 7.0) * scale,
                y * scale,
                3.0 * scale,
                thumb * scale,
                1.5 * scale,
                theme::INPUT_BORDER,
                true,
            );
        }
    }
}

#[cfg(test)]
#[path = "../tests/ui/settings.rs"]
mod tests;
