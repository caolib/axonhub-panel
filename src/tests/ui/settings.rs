use super::*;
use crate::config::{Account, ChannelKeyAlias, Config};

fn key_alias(account_id: &str, channel: &str, key_index: i64, alias: &str) -> ChannelKeyAlias {
    ChannelKeyAlias {
        account_id: account_id.to_string(),
        channel: channel.to_string(),
        key_index,
        alias: alias.to_string(),
    }
}

fn app_with_targets() -> App {
    let mut app = App::new(Config::default());
    app.config.accounts = vec![
        Account {
            id: "a".into(),
            name: "账号A".into(),
            ..Default::default()
        },
        Account {
            id: "b".into(),
            name: "账号B".into(),
            ..Default::default()
        },
    ];
    // `channel_filters()` also lists saved hidden channels, so this stands in
    // for a channel seen in fetched requests without touching private state.
    app.config.hidden_channels = vec![HiddenChannel {
        account_id: "a".into(),
        channel: "alpha".into(),
    }];
    app.config.channel_key_aliases = vec![
        key_alias("a", "beta", 2, "my-k2"),
        key_alias("b", "gamma", 0, "B-0"),
    ];
    app
}

fn key_state(target: Option<HiddenChannel>) -> State {
    State {
        tab: Tab::Models,
        mapping_keys: true,
        key_alias_target: target,
        ..Default::default()
    }
}

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
        // The 21-row grid is taller than the body, so scroll to each cell
        // before hit-testing it.
        let scroll = (control.rect.y + control.rect.h - layout.view_height())
            .clamp(0.0, layout.max_scroll());
        let scrolled = State {
            tab: Tab::Fields,
            scroll,
            ..Default::default()
        };
        let rect = layout.screen_rect(control, scroll);
        assert!(
            rect.y >= TOP && rect.y + rect.h <= HEIGHT - FOOTER,
            "{field:?} is not inside the viewport: {rect:?}"
        );
        assert_eq!(
            layout.hit(&scrolled, rect.x + 2.0, rect.y + 2.0),
            Some(Action::Field(field)),
        );
    }
    assert_eq!(DisplayField::ALL.len(), 21);
    assert_eq!(
        layout.controls.iter().filter(|c| c.check).count(),
        21,
        "every field gets exactly one checkbox"
    );
}

#[test]
fn the_last_field_checkbox_is_reachable_by_scrolling() {
    let app = App::new(Config::default());
    let mut state = State {
        tab: Tab::Fields,
        ..Default::default()
    };
    let layout = Layout::new(&app, &state, WIDTH, HEIGHT);
    let last = *DisplayField::ALL.last().unwrap();
    for _ in 0..layout.controls.len() {
        if state.focus == Some(Action::Field(last)) {
            break;
        }
        layout.focus_next(&mut state, false);
    }
    assert_eq!(state.focus, Some(Action::Field(last)));
    assert!(layout.max_scroll() > 0.0, "21 rows need a scrollable body");
    let control = layout
        .controls
        .iter()
        .find(|c| c.action == Action::Field(last))
        .unwrap();
    let rect = layout.screen_rect(control, state.scroll);
    assert!(
        rect.y >= TOP && rect.y + rect.h <= HEIGHT - FOOTER,
        "last field {last:?} must be scrolled into view: {rect:?}"
    );
    assert_eq!(
        layout.hit(&state, rect.x + 2.0, rect.y + 2.0),
        Some(Action::Field(last))
    );
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

#[test]
fn key_targets_merge_rows_and_saved_aliases_and_keep_accounts_apart() {
    let app = app_with_targets();
    let targets = key_targets(&app);
    assert!(
        targets.contains(&HiddenChannel {
            account_id: "a".into(),
            channel: "alpha".into(),
        }),
        "a channel seen in requests is a target"
    );
    assert!(
        targets.contains(&HiddenChannel {
            account_id: "a".into(),
            channel: "beta".into(),
        }),
        "a saved alias target is still offered"
    );
    assert!(
        targets.contains(&HiddenChannel {
            account_id: "b".into(),
            channel: "gamma".into(),
        }),
        "the other account keeps its own target"
    );
    // Same channel name on two accounts must not collapse into one entry.
    let mut app = app_with_targets();
    app.config
        .channel_key_aliases
        .push(key_alias("b", "alpha", 1, "b-a"));
    let targets = key_targets(&app);
    assert_eq!(
        targets
            .iter()
            .filter(|target| target.channel == "alpha")
            .count(),
        2,
        "account isolation is part of the target identity"
    );
}

#[test]
fn first_key_target_is_frozen_when_the_subpage_opens() {
    let app = app_with_targets();
    let targets = key_targets(&app);
    let mut state = State::default();
    state.ensure_key_alias_target(&targets);
    assert_eq!(state.key_alias_target.as_ref(), targets.first());

    // A later poll may prepend a channel; the chosen target must not move.
    let chosen = state.key_alias_target.clone();
    let mut later = vec![HiddenChannel {
        account_id: "b".into(),
        channel: "new-channel".into(),
    }];
    later.extend(targets);
    state.ensure_key_alias_target(&later);
    assert_eq!(state.key_alias_target, chosen);
}

#[test]
fn selected_key_target_is_highlighted_and_the_others_are_not() {
    let app = app_with_targets();
    let targets = key_targets(&app);
    let selected = targets[1].clone();
    let state = key_state(Some(selected.clone()));
    let layout = Layout::new(&app, &state, WIDTH, HEIGHT);
    let chosen = layout
        .controls
        .iter()
        .find(|c| c.action == Action::KeyTarget(selected.clone()))
        .expect("every target gets a button");
    assert!(chosen.selected);
    for target in targets.iter().filter(|target| **target != selected) {
        let control = layout
            .controls
            .iter()
            .find(|c| c.action == Action::KeyTarget(target.clone()))
            .unwrap();
        assert!(!control.selected, "{target:?} must not look selected");
    }
}

#[test]
fn a_selected_target_that_vanishes_from_polling_stays_visible_and_selected() {
    let app = app_with_targets();
    assert!(
        !key_targets(&app)
            .iter()
            .any(|target| target.channel == "gone"),
        "the target is absent from the current poll and saved aliases"
    );
    let selected = HiddenChannel {
        account_id: "a".into(),
        channel: "gone".into(),
    };
    let state = key_state(Some(selected.clone()));
    let layout = Layout::new(&app, &state, WIDTH, HEIGHT);
    let chosen = layout
        .controls
        .iter()
        .find(|c| c.action == Action::KeyTarget(selected.clone()))
        .expect("the explicit target must stay on screen");
    assert!(chosen.selected);
    // And no other row may quietly take its place as the selection.
    for control in layout
        .controls
        .iter()
        .filter(|c| matches!(c.action, Action::KeyTarget(_)))
    {
        if control.action != Action::KeyTarget(selected.clone()) {
            assert!(!control.selected);
        }
    }
    // The frozen UI state is untouched by rendering.
    assert_eq!(state.key_alias_target, Some(selected));
}

#[test]
fn key_index_zero_is_valid_but_pasted_fractions_are_not() {
    assert_eq!(normalize_key_index("0"), Some(0));
    assert_eq!(normalize_key_index(" 2 "), Some(2));
    assert_eq!(normalize_key_index("2.5"), None);
    assert_eq!(normalize_key_index("-1"), None);
    assert_eq!(normalize_key_index(""), None);
}

#[test]
fn key_edit_boxes_follow_scroll_and_only_live_on_the_key_subpage() {
    let mut app = app_with_targets();
    // Extra rows on the same target make the body scrollable without moving
    // the form: the target list is deduplicated.
    for i in 0..10 {
        app.config
            .channel_key_aliases
            .push(key_alias("a", "beta", i, &format!("k{i}")));
    }
    let targets = key_targets(&app);
    let state = key_state(Some(targets[0].clone()));
    let layout = Layout::new(&app, &state, WIDTH, HEIGHT);
    let rest = layout
        .edit_box_rect(&Action::KeyIndexInput, 0.0)
        .expect("the key form is visible at the top");
    let scrolled = layout
        .edit_box_rect(&Action::KeyIndexInput, 60.0)
        .expect("the box follows the body scroll");
    assert_eq!(rest.y - scrolled.y, 60.0);
    assert!(
        layout
            .edit_box_rect(&Action::KeyIndexInput, 10_000.0)
            .is_none(),
        "hidden once the form scrolls out of view"
    );
    assert!(layout.edit_box_rect(&Action::KeyAliasInput, 0.0).is_some());

    let model_state = State {
        tab: Tab::Models,
        ..Default::default()
    };
    let model_layout = Layout::new(&app, &model_state, WIDTH, HEIGHT);
    assert!(
        model_layout
            .edit_box_rect(&Action::KeyIndexInput, 0.0)
            .is_none(),
        "the model subpage hides the key boxes"
    );
    assert!(
        model_layout
            .edit_box_rect(&Action::KeyAliasInput, 0.0)
            .is_none()
    );
}

#[test]
fn add_and_delete_key_mapping_buttons_hit_their_cells() {
    let app = app_with_targets();
    let targets = key_targets(&app);
    let state = key_state(Some(targets[0].clone()));
    let layout = Layout::new(&app, &state, WIDTH, HEIGHT);
    for action in [Action::AddKeyAlias, Action::RemoveKeyAlias(0)] {
        let control = layout
            .controls
            .iter()
            .find(|c| c.action == action)
            .unwrap_or_else(|| panic!("{action:?} should be laid out"));
        let rect = layout.screen_rect(control, state.scroll);
        assert_eq!(
            layout.hit(&state, rect.x + 2.0, rect.y + 2.0),
            Some(action.clone()),
            "{action:?} must be clickable"
        );
    }
}

#[test]
fn a_long_key_mapping_list_can_scroll_to_its_last_delete() {
    let mut app = app_with_targets();
    app.config.channel_key_aliases = (0..30)
        .map(|i| key_alias("a", "alpha", i, &format!("k{i}")))
        .collect();
    let targets = key_targets(&app);
    let mut state = key_state(Some(targets[0].clone()));
    let layout = Layout::new(&app, &state, WIDTH, MIN_HEIGHT);
    let last = Action::RemoveKeyAlias(29);
    for _ in 0..layout.controls.len() {
        if state.focus == Some(last.clone()) {
            break;
        }
        layout.focus_next(&mut state, false);
    }
    assert_eq!(state.focus, Some(last.clone()));
    let control = layout.controls.iter().find(|c| c.action == last).unwrap();
    let rect = layout.screen_rect(control, state.scroll);
    assert!(
        rect.y >= layout.content_top && rect.y + rect.h <= layout.height - FOOTER,
        "the last delete row must be scrolled into view: {rect:?}"
    );
    assert_eq!(
        layout.hit(&state, rect.x + 2.0, rect.y + 2.0),
        Some(Action::RemoveKeyAlias(29))
    );
}

#[test]
fn the_model_subpage_keeps_its_original_form_below_the_new_switch() {
    let mut app = App::new(Config::default());
    app.config.model_aliases = vec![crate::config::ModelAlias {
        from: "deepseek".into(),
        to: "ds".into(),
    }];
    let state = State {
        tab: Tab::Models,
        ..Default::default()
    };
    let layout = Layout::new(&app, &state, WIDTH, HEIGHT);
    assert!(layout.controls.iter().any(|c| c.action == Action::AddAlias));
    assert!(
        layout
            .controls
            .iter()
            .any(|c| c.action == Action::RemoveAlias(0))
    );
    assert!(
        !layout.controls.iter().any(|c| matches!(
            c.action,
            Action::KeyIndexInput | Action::KeyAliasInput | Action::AddKeyAlias
        )),
        "the model subpage must not show the key form"
    );
    let from = layout
        .edit_box_rect(&Action::AliasFromInput, 0.0)
        .expect("the model form still has its native boxes");
    // The whole model page moved down by the sub-tab row (36-44px).
    assert_eq!(from.y, TOP + 44.0 + 62.0);
    // And the subpage switch itself sits above the scrolling content.
    assert!(
        layout
            .controls
            .iter()
            .any(|c| c.action == Action::MappingPage(false) && c.selected)
    );
}
