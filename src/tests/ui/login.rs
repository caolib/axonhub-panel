use super::*;
use crate::config::Account;

const ENDPOINT: &str = "http://localhost:8090";

fn form(method: Method, mode: CredentialMode) -> LoginForm {
    let mut f = LoginForm::blank(ENDPOINT, mode);
    f.set_method(method);
    // A named, valid endpoint form, so only the credential checks can fail.
    f.account_name = "工作".into();
    f
}

fn tab_center(m: Metrics, method: Method) -> (f32, f32) {
    let f = form(Method::Token, CredentialMode::None);
    let l = layout(&f, m);
    let (rect, _) = l
        .tabs
        .iter()
        .find(|(_, m)| *m == method)
        .expect("tab exists");
    (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0)
}

#[test]
fn method_tabs_are_hit_by_their_own_rects() {
    let m = Metrics::new(452.0, 600.0, 1.0);
    let f = form(Method::Token, CredentialMode::None);

    let (x, y) = tab_center(m, Method::Password);
    assert!(matches!(
        hit(&f, m, x, y),
        Some(Action::SetMethod(Method::Password))
    ));
    let (x, y) = tab_center(m, Method::Token);
    assert!(matches!(
        hit(&f, m, x, y),
        Some(Action::SetMethod(Method::Token))
    ));

    // A point between the tabs is not a tab.
    let l = layout(&f, m);
    let gap_x = l.tabs[0].0.x + l.tabs[0].0.w + 2.0;
    assert!(!matches!(
        hit(&f, m, gap_x, l.tabs[0].0.y + 2.0),
        Some(Action::SetMethod(_))
    ));
}

#[test]
fn busy_form_refuses_tab_clicks_and_method_switches() {
    let m = Metrics::new(452.0, 600.0, 1.0);
    let mut f = form(Method::Token, CredentialMode::Token);
    f.error = Some("旧错误".into());
    f.busy = true;

    let (x, y) = tab_center(m, Method::Password);
    assert!(hit(&f, m, x, y).is_none());

    f.set_method(Method::Password);
    assert_eq!(f.method, Method::Token);
    assert_eq!(f.mode, CredentialMode::Token);
    assert_eq!(f.error.as_deref(), Some("旧错误"));

    f.cycle_mode();
    assert_eq!(f.mode, CredentialMode::Token);
}

#[test]
fn layout_fits_the_default_window_for_both_methods() {
    for method in [Method::Token, Method::Password] {
        let m = Metrics::new(452.0, 600.0, 1.0);
        let f = form(method, CredentialMode::None);
        let l = layout(&f, m);

        assert!(l.card.y >= 0.0, "{method:?}");
        assert!(l.card.y + l.card.h <= 600.0, "{method:?}");
        assert_eq!(
            l.fields.len(),
            if method == Method::Password { 4 } else { 3 }
        );
        // Tabs sit above the first field, and the button and error line sit
        // inside the card.
        assert!(l.tabs[0].0.y + l.tabs[0].0.h <= l.fields[0].0.y);
        assert!(l.button.y + l.button.h <= l.card.y + l.card.h);
        assert!(l.error_y + 20.0 <= l.card.y + l.card.h);
    }
}

#[test]
fn layout_and_window_height_scale_with_the_monitor() {
    let at_100 = Metrics::new(452.0, 600.0, 1.0);
    let at_150 = Metrics::new(678.0, 900.0, 1.5);
    let f = form(Method::Password, CredentialMode::Password);
    let a = layout(&f, at_100);
    let b = layout(&f, at_150);

    assert!((b.card.h - a.card.h * 1.5).abs() < 0.01);
    assert!((b.tabs[0].0.h - a.tabs[0].0.h * 1.5).abs() < 0.01);
    assert!((b.fields[0].0.h - a.fields[0].0.h * 1.5).abs() < 0.01);
    assert!((b.button.h - a.button.h * 1.5).abs() < 0.01);

    assert_eq!(window_height(1.0), 516);
    assert_eq!(window_height(1.5), 774);
    // The returned height must hold the tallest form with its margins.
    let m = Metrics::new(452.0, window_height(1.0) as f32, 1.0);
    let l = layout(&form(Method::Password, CredentialMode::Password), m);
    assert!(l.card.y >= 16.0);
    assert!(l.card.y + l.card.h <= 516.0 - 16.0);
}

#[test]
fn password_method_requires_email_and_password() {
    let mut f = form(Method::Password, CredentialMode::Password);
    assert_eq!(f.validate(), Err("邮箱不能为空".into()));

    f.email = "you@example.com".into();
    assert_eq!(f.validate(), Err("密码不能为空".into()));

    f.password = "secret".into();
    assert_eq!(f.validate(), Ok(()));
}

#[test]
fn password_validation_does_not_trim_or_mangle_unicode() {
    let mut f = form(Method::Password, CredentialMode::Password);
    f.email = "  you@example.com  ".into();
    f.password = " 密码🙂 ".into();

    assert_eq!(f.validate(), Ok(()));
    // The submitted credentials keep the password byte for byte; only the
    // email is trimmed.
    let creds = f.credentials();
    assert_eq!(creds.email, "you@example.com");
    assert_eq!(creds.password, " 密码🙂 ");

    // Masking counts characters, not bytes.
    assert_eq!(f.shown_value(Field::Password), "*****");
}

#[test]
fn typing_and_pasting_into_password_keep_unicode() {
    let mut f = form(Method::Password, CredentialMode::Password);
    f.focus_field(Field::Password);
    for ch in "密码🙂".chars() {
        f.insert(ch);
    }
    assert_eq!(f.password, "密码🙂");

    f.password.clear();
    f.caret = 0;
    f.paste(" 密码 ");
    assert_eq!(f.password, " 密码 ");
    assert_eq!(f.caret, 4);
}

#[test]
fn switching_to_password_selects_password_storage_and_email_focus() {
    let mut f = form(Method::Token, CredentialMode::Token);
    f.error = Some("旧错误".into());

    f.set_method(Method::Password);
    assert_eq!(f.method, Method::Password);
    assert_eq!(f.mode, CredentialMode::Password);
    assert_eq!(f.focus, Field::Email);
    assert_eq!(f.caret, 0);
    assert!(f.error.is_none());

    // Switching back leaves the password mode, which the token method cannot
    // honour, so it falls back to storing the token.
    f.set_method(Method::Token);
    assert_eq!(f.method, Method::Token);
    assert_eq!(f.mode, CredentialMode::Token);
    assert_eq!(f.focus, Field::Token);
}

#[test]
fn switching_to_the_same_method_only_clears_the_error() {
    let mut f = form(Method::Token, CredentialMode::None);
    f.error = Some("旧错误".into());
    f.set_method(Method::Token);
    assert_eq!(f.mode, CredentialMode::None);
    assert_eq!(f.focus, Field::Token);
    assert!(f.error.is_none());
}

#[test]
fn password_method_cycles_all_three_modes() {
    let mut f = form(Method::Password, CredentialMode::Password);
    // Switching to the password tab forces the password mode; the cycle still
    // has to start from wherever the mode is left.
    f.set_mode(CredentialMode::None);
    f.cycle_mode();
    assert_eq!(f.mode, CredentialMode::Token);
    f.cycle_mode();
    assert_eq!(f.mode, CredentialMode::Password);
    f.cycle_mode();
    assert_eq!(f.mode, CredentialMode::None);
}

#[test]
fn token_method_cycles_only_the_two_token_modes() {
    let mut f = form(Method::Token, CredentialMode::None);
    f.cycle_mode();
    assert_eq!(f.mode, CredentialMode::Token);
    f.cycle_mode();
    assert_eq!(f.mode, CredentialMode::None);
    // A stale password mode still falls back to not storing anything.
    f.mode = CredentialMode::Password;
    f.cycle_mode();
    assert_eq!(f.mode, CredentialMode::None);
}

#[test]
fn blank_chooses_method_and_focus_from_the_storage_mode() {
    let token = LoginForm::blank(ENDPOINT, CredentialMode::Token);
    assert_eq!(token.method, Method::Token);
    assert_eq!(token.focus, Field::Token);

    let none = LoginForm::blank(ENDPOINT, CredentialMode::None);
    assert_eq!(none.method, Method::Token);
    assert_eq!(none.focus, Field::Token);

    let password = LoginForm::blank(ENDPOINT, CredentialMode::Password);
    assert_eq!(password.method, Method::Password);
    assert_eq!(password.focus, Field::Email);
}

#[test]
fn for_account_keeps_its_signature_and_focuses_the_methods_first_field() {
    let account = Account {
        id: "a1".into(),
        name: "工作".into(),
        endpoint: ENDPOINT.into(),
        project_id: String::new(),
    };

    let token = LoginForm::for_account(&account, CredentialMode::Token);
    assert_eq!(token.account_name, "工作");
    assert_eq!(token.endpoint, ENDPOINT);
    assert_eq!(token.editing.as_deref(), Some("a1"));
    assert_eq!(token.focus, Field::Token);

    let password = LoginForm::for_account(&account, CredentialMode::Password);
    assert_eq!(password.method, Method::Password);
    assert_eq!(password.focus, Field::Email);
}

#[test]
fn field_rows_map_to_the_methods_credentials() {
    let m = Metrics::new(452.0, 600.0, 1.0);
    let password = form(Method::Password, CredentialMode::Password);
    let l = layout(&password, m);
    let fields: Vec<Field> = (0..l.fields.len())
        .map(|i| field_for(Method::Password, i))
        .collect();
    assert_eq!(
        fields,
        vec![Field::Name, Field::Endpoint, Field::Email, Field::Password]
    );

    let token = form(Method::Token, CredentialMode::Token);
    let l = layout(&token, m);
    let fields: Vec<Field> = (0..l.fields.len())
        .map(|i| field_for(Method::Token, i))
        .collect();
    assert_eq!(fields, vec![Field::Name, Field::Endpoint, Field::Token]);
}
