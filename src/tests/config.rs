use super::*;

/// A scratch directory per test, so nothing touches a real profile and the
/// tests can run in parallel without sharing process state.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ah-panel-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn store(&self, mode: CredentialMode, stored: &Stored) {
        store_in(&self.0, mode, stored);
    }

    fn load(&self) -> Option<Stored> {
        read_stored_in(&self.0)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sample() -> Stored {
    Stored {
        email: "a@b".into(),
        password: Some("secret".into()),
        token: Some("tok".into()),
        accounts: Vec::new(),
    }
}

#[test]
fn token_mode_keeps_the_token_and_drops_the_password() {
    let dir = Scratch::new("token");
    dir.store(CredentialMode::Token, &sample());

    let got = dir.load().expect("token should have been stored");
    assert_eq!(got.token.as_deref(), Some("tok"));
    assert_eq!(got.password, None, "token mode must not write a password");
}

#[test]
fn password_mode_keeps_both() {
    let dir = Scratch::new("password");
    dir.store(CredentialMode::Password, &sample());

    let got = dir.load().expect("credentials should have been stored");
    assert_eq!(got.token.as_deref(), Some("tok"));
    assert_eq!(got.password.as_deref(), Some("secret"));
}

#[test]
fn none_mode_writes_nothing_and_clears_what_was_there() {
    let dir = Scratch::new("none");
    dir.store(CredentialMode::Token, &sample());
    assert!(dir.load().is_some());

    dir.store(CredentialMode::None, &sample());
    assert!(dir.load().is_none(), "none mode must clear what was stored");
}

#[test]
fn switching_mode_purges_what_it_may_no_longer_keep() {
    let dir = Scratch::new("downgrade");
    dir.store(CredentialMode::Password, &sample());
    assert!(dir.load().unwrap().password.is_some());

    // Downgrading must not leave the old password behind.
    let current = dir.load().unwrap();
    dir.store(CredentialMode::Token, &current);
    assert_eq!(dir.load().unwrap().password, None);
}

#[test]
fn a_token_mode_without_a_token_does_not_leave_a_stale_blob() {
    let dir = Scratch::new("empty");
    dir.store(CredentialMode::Password, &sample());
    assert!(dir.load().is_some());

    // Nothing worth keeping in token mode: the old blob must go, not linger.
    dir.store(
        CredentialMode::Token,
        &Stored {
            email: String::new(),
            password: None,
            token: None,
            accounts: Vec::new(),
        },
    );
    assert!(dir.load().is_none());
}

#[test]
fn config_round_trips_through_disk() {
    // Config::save/load use the process-wide directory, so drive the serde
    // layer directly here to keep the test isolated.
    let mut config = Config::default();
    config.endpoint = "http://example:1234".into();
    config.row_limit = 7;
    config.credential_mode = CredentialMode::Password;
    config.single_line = true;

    let json = serde_json::to_string(&config).unwrap();
    let back: Config = serde_json::from_str(&json).unwrap();
    assert_eq!(back.endpoint, "http://example:1234");
    assert_eq!(back.row_limit, 7);
    assert_eq!(back.credential_mode, CredentialMode::Password);
    assert!(back.single_line);
}

#[test]
fn a_config_without_single_line_keeps_two_line_cards() {
    // A file written before the mode existed must not silently switch the
    // panel to the compact layout.
    let back: Config = serde_json::from_str("{}").unwrap();
    assert!(!back.single_line);
}

#[test]
fn every_card_cell_starts_visible_and_can_be_switched_off() {
    let mut config = Config::default();
    for field in DisplayField::ALL {
        assert!(config.shows(field), "{field:?} should start visible");
    }
    config.toggle_field(DisplayField::Tokens);
    assert!(!config.shows(DisplayField::Tokens));
    assert!(
        config.shows(DisplayField::Model),
        "hiding one keeps the rest"
    );
    assert_eq!(config.hidden_fields, vec![DisplayField::Tokens]);

    // Toggling again shows it, and the reset clears every choice.
    config.toggle_field(DisplayField::Tokens);
    assert!(config.shows(DisplayField::Tokens));
    config.toggle_field(DisplayField::Speed);
    config.show_all_fields();
    assert!(config.hidden_fields.is_empty());
}

#[test]
fn hidden_fields_round_trip_as_camel_case_names() {
    let config = Config {
        hidden_fields: vec![DisplayField::PassThrough, DisplayField::CreatedAt],
        ..Config::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"passThrough\""), "{json}");
    assert!(json.contains("\"createdAt\""), "{json}");

    let back: Config = serde_json::from_str(&json).unwrap();
    assert_eq!(back.hidden_fields, config.hidden_fields);
    assert!(!back.shows(DisplayField::PassThrough));
    // A file written before the setting existed shows everything.
    let older: Config = serde_json::from_str("{}").unwrap();
    assert!(older.hidden_fields.is_empty());
}

#[test]
fn clamping_keeps_the_whole_window_on_one_monitor() {
    let size = |x, y| WindowState {
        x,
        y,
        width: 452,
        height: 600,
    };

    // A monitor to the right of the primary one: positive coordinates.
    let right = (1920, 0, 3840, 1040);
    // Dragged past the right edge — pulled back to sit flush against it.
    let placed = clamp_to_virtual_screen(size(3700, 100), right, 1.0, false);
    assert_eq!((placed.x, placed.y), (3840 - 452, 100));
    // Dragged below the work area — the taskbar strip is not usable space.
    let placed = clamp_to_virtual_screen(size(2000, 900), right, 1.0, false);
    assert_eq!(placed.y, 1040 - 600);
    // Already inside: left exactly where it was.
    let inside = size(2000, 100);
    let placed = clamp_to_virtual_screen(inside, right, 1.0, false);
    assert_eq!(
        (placed.x, placed.y, placed.width, placed.height),
        (2000, 100, 452, 600)
    );

    // A monitor left of the primary one: negative coordinates must clamp
    // just as well, or the panel would jump back to the primary screen.
    let left = (-1920, 0, 0, 1040);
    let placed = clamp_to_virtual_screen(size(-2000, -50), left, 1.0, false);
    assert_eq!((placed.x, placed.y), (-1920, 0));
    let placed = clamp_to_virtual_screen(size(-2000, 100), left, 1.0, false);
    assert_eq!(placed.x, -1920);

    // Bigger than the work area: shrunk to it rather than left hanging off.
    let placed = clamp_to_virtual_screen(size(0, 0), (0, 0, 1920, 1040), 1.0, false);
    let huge = WindowState {
        width: 3000,
        height: 2000,
        ..placed
    };
    let placed = clamp_to_virtual_screen(huge, (0, 0, 1920, 1040), 1.0, false);
    assert_eq!((placed.width, placed.height), (1920, 1040));
}

#[test]
fn the_minimum_size_follows_the_layout_and_the_monitor_scale() {
    let screen = (0, 0, 1920, 1040);
    let tiny = WindowState {
        x: 10,
        y: 10,
        width: 100,
        height: 100,
    };
    // Two-line cards keep the authored 320x200 floor...
    let placed = clamp_to_virtual_screen(tiny, screen, 1.0, false);
    assert_eq!((placed.width, placed.height), (320, 200));
    // ...single-line mode lets a short panel through...
    let placed = clamp_to_virtual_screen(tiny, screen, 1.0, true);
    assert_eq!((placed.width, placed.height), (320, 110));
    // ...and both floors scale with the monitor density.
    let placed = clamp_to_virtual_screen(tiny, screen, 2.0, false);
    assert_eq!((placed.width, placed.height), (640, 400));
}
