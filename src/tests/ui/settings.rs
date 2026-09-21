use super::*;
use crate::config::{Account, Config};

#[test]
fn keyboard_reaches_last_account_and_scrolls_it_into_view() {
    let mut app = App::new(Config::default());
    for i in 0..80 {
        app.accounts.push(Account {
            id: format!("a{i}"),
            name: format!("账号{i}"),
            ..Default::default()
        });
    }
    let mut state = State {
        tab: Tab::Accounts,
        ..Default::default()
    };
    let layout = Layout::new(&app, &state, MIN_WIDTH, MIN_HEIGHT);
    // Walk one full lap of the real control list rather than a hard-coded
    // step count: every control added to an account card used to leave the
    // loop stranded mid-list, never reaching the last account.
    for _ in 0..layout.controls.len() {
        if state.focus == Some(Action::RemoveAccount("a79".into())) {
            break;
        }
        layout.focus_next(&mut state, false);
    }
    assert_eq!(state.focus, Some(Action::RemoveAccount("a79".into())));
    let control = layout
        .controls
        .iter()
        .find(|c| Some(&c.action) == state.focus.as_ref())
        .unwrap();
    let rect = layout.screen_rect(control, state.scroll);
    assert!(rect.y >= TOP && rect.y + rect.h <= layout.height - FOOTER);
    assert_eq!(layout.hit(&state, rect.x + 1.0, rect.y + 1.0), state.focus);
    assert!(layout.hit(&state, rect.x, TOP - 1.0).is_none());
}

#[test]
fn confirmation_blocks_background_controls_and_starts_on_cancel() {
    let app = App::new(Config::default());
    let mut state = State {
        pending: Some(Action::ClearCredentials),
        ..Default::default()
    };
    let layout = Layout::new(&app, &state, WIDTH, HEIGHT);
    assert!(layout.hit(&state, 30.0, 75.0).is_none());
    layout.focus_next(&mut state, false);
    assert_eq!(state.focus, Some(Action::Cancel));
    layout.focus_next(&mut state, false);
    assert_eq!(state.focus, Some(Action::Confirm));
}

#[test]
fn every_card_cell_has_a_checkbox_that_hits_its_own_cell() {
    let app = App::new(Config::default());
    let state = State {
        tab: Tab::Fields,
        ..Default::default()
    };
    let layout = Layout::new(&app, &state, WIDTH, HEIGHT);
    for field in DisplayField::ALL {
        let control = layout
            .controls
            .iter()
            .find(|c| c.action == Action::Field(field))
            .unwrap_or_else(|| panic!("{field:?} should have a checkbox"));
        assert!(control.check, "{field:?} should be a checkbox row");
        assert!(control.selected, "{field:?} should start checked");
        let rect = layout.screen_rect(control, state.scroll);
        assert_eq!(
            layout.hit(&state, rect.x + 2.0, rect.y + 2.0),
            Some(Action::Field(field)),
        );
    }
    assert_eq!(DisplayField::ALL.len(), 13);
}

#[test]
fn hidden_cells_show_unchecked_and_there_is_a_reset() {
    let mut app = App::new(Config::default());
    app.config.toggle_field(DisplayField::Retry);
    let state = State {
        tab: Tab::Fields,
        ..Default::default()
    };
    let layout = Layout::new(&app, &state, WIDTH, HEIGHT);
    let retry = layout
        .controls
        .iter()
        .find(|c| c.action == Action::Field(DisplayField::Retry))
        .unwrap();
    assert!(!retry.selected);
    assert!(
        layout
            .controls
            .iter()
            .any(|c| c.action == Action::ShowAllFields),
        "the tab offers a reset"
    );
}
