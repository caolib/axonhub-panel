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

#[test]
fn font_size_is_typed_into_a_field_and_saved_with_a_button() {
    let app = App::new(Config::default());
    let state = State {
        tab: Tab::Display,
        ..Default::default()
    };
    let layout = Layout::new(&app, &state, WIDTH, HEIGHT);
    let rect = layout
        .edit_box_rect(&Action::FontSizeInput, state.scroll)
        .expect("the display tab shows the size field");
    assert!(
        layout
            .controls
            .iter()
            .any(|c| c.action == Action::FontSizeInput),
        "the field has a layout cell for painting and Tab order"
    );
    let save = layout
        .controls
        .iter()
        .find(|c| c.action == Action::SaveFontSize)
        .expect("the field saves through its own button");
    // The button is painted from content coordinates; the native box is placed
    // from screen coordinates, so it must carry the same content-top offset —
    // placing it at its raw content y lands it at the top of the window.
    assert_eq!(rect.y, save.rect.y + TOP, "the box shares the button's row");
}

#[test]
fn opacity_is_typed_into_a_field_and_saved_with_a_button() {
    let app = App::new(Config::default());
    let state = State {
        tab: Tab::Display,
        ..Default::default()
    };
    let layout = Layout::new(&app, &state, WIDTH, HEIGHT);
    let rect = layout
        .edit_box_rect(&Action::OpacityInput, state.scroll)
        .expect("the display tab shows the opacity field");
    assert!(
        layout
            .controls
            .iter()
            .any(|c| c.action == Action::OpacityInput),
        "the field has a layout cell for painting and Tab order"
    );
    let save = layout
        .controls
        .iter()
        .find(|c| c.action == Action::SaveOpacity)
        .expect("the field saves through its own button");
    assert_eq!(rect.y, save.rect.y + TOP, "the box shares the button's row");
}

#[test]
fn opacity_field_only_lives_on_the_display_tab() {
    let app = App::new(Config::default());
    for tab in [Tab::Fields, Tab::Fonts, Tab::Accounts, Tab::Channels] {
        let state = State {
            tab,
            ..Default::default()
        };
        let layout = Layout::new(&app, &state, WIDTH, HEIGHT);
        assert!(
            layout.edit_box_rect(&Action::OpacityInput, 0.0).is_none(),
            "{tab:?} hides the field"
        );
    }
}

#[test]
fn the_size_field_follows_the_body_scroll() {
    let app = App::new(Config::default());
    let state = State {
        tab: Tab::Display,
        ..Default::default()
    };
    let layout = Layout::new(&app, &state, WIDTH, MIN_HEIGHT);
    let rest = layout
        .edit_box_rect(&Action::FontSizeInput, 0.0)
        .expect("visible at the top of the body");
    let scrolled = layout
        .edit_box_rect(&Action::FontSizeInput, 60.0)
        .expect("still visible after a short scroll");
    assert_eq!(rest.y - scrolled.y, 60.0, "the box moves with the content");
    assert!(
        layout
            .edit_box_rect(&Action::FontSizeInput, 10_000.0)
            .is_none(),
        "hidden once the body scrolls past the field"
    );
}

#[test]
fn font_size_field_only_lives_on_the_display_tab() {
    let app = App::new(Config::default());
    for tab in [Tab::Fields, Tab::Fonts, Tab::Accounts, Tab::Channels] {
        let state = State {
            tab,
            ..Default::default()
        };
        let layout = Layout::new(&app, &state, WIDTH, HEIGHT);
        assert!(
            layout.edit_box_rect(&Action::FontSizeInput, 0.0).is_none(),
            "{tab:?} hides the field"
        );
    }
}
