mod common;

use anki::decks::DeckId;
use common::fresh_collection;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use yaac::config::Config;
use yaac::decks::DeckRow;
use yaac::session::Session;
use yaac::tui::decks::{Picker, PickerAction};

fn row(id: i64, name: &str, level: u32, new: u32) -> DeckRow {
    DeckRow {
        id,
        name: name.to_string(),
        level,
        new,
        learn: 0,
        review: 0,
        total: new,
    }
}

fn rows() -> Vec<DeckRow> {
    vec![
        row(1, "Default", 1, 0),
        row(2, "Spanish", 1, 3),
        row(3, "Spanish::Verbs", 2, 1),
        row(4, "Geography", 1, 0),
    ]
}

fn press(picker: &mut Picker, code: KeyCode) -> PickerAction {
    picker.handle(KeyEvent::from(code))
}

#[test]
fn starts_on_the_first_deck_with_cards_due() {
    let picker = Picker::new(rows());
    assert_eq!(picker.selected().map(|row| row.id), Some(2));
}

#[test]
fn slash_filters_decks_and_enter_selects_the_match() {
    let mut picker = Picker::new(rows());
    assert_eq!(
        press(&mut picker, KeyCode::Char('/')),
        PickerAction::Continue
    );
    for c in "geo".chars() {
        assert_eq!(press(&mut picker, KeyCode::Char(c)), PickerAction::Continue);
    }
    let names: Vec<&str> = picker
        .visible()
        .iter()
        .map(|row| row.name.as_str())
        .collect();
    assert_eq!(names, ["Geography"]);
    assert_eq!(
        press(&mut picker, KeyCode::Enter),
        PickerAction::Select(DeckId(4))
    );

    // While searching, letters go into the filter instead of acting as shortcuts.
    assert_eq!(
        press(&mut picker, KeyCode::Char('/')),
        PickerAction::Continue
    );
    assert_eq!(
        press(&mut picker, KeyCode::Char('q')),
        PickerAction::Continue
    );
    assert!(picker.visible().is_empty(), "no deck contains a q");
    assert_eq!(press(&mut picker, KeyCode::Esc), PickerAction::Continue);
    assert_eq!(picker.visible().len(), 4, "esc clears the filter");
    assert_eq!(
        press(&mut picker, KeyCode::Esc),
        PickerAction::Continue,
        "esc never quits"
    );
    assert_eq!(press(&mut picker, KeyCode::Char('q')), PickerAction::Quit);
}

#[test]
fn esc_clears_a_filter_left_over_from_a_review_before_it_quits() {
    let mut picker = Picker::new(rows());
    press(&mut picker, KeyCode::Char('/'));
    press(&mut picker, KeyCode::Char('g'));
    assert_eq!(
        press(&mut picker, KeyCode::Enter),
        PickerAction::Select(DeckId(4))
    );
    // Back from the review: the filter is still applied but no longer being typed.
    picker.set_rows(rows());
    assert_eq!(picker.visible().len(), 1);
    assert_eq!(press(&mut picker, KeyCode::Esc), PickerAction::Continue);
    assert_eq!(picker.visible().len(), 4, "esc shows all decks again");
    assert_eq!(press(&mut picker, KeyCode::Esc), PickerAction::Continue);
}

#[test]
fn shortcuts_sync_and_quit_and_rows_refresh_keeps_the_selection() {
    let mut picker = Picker::new(rows());
    assert_eq!(
        press(&mut picker, KeyCode::Char('j')),
        PickerAction::Continue
    );
    assert_eq!(picker.selected().map(|row| row.id), Some(3));
    assert_eq!(press(&mut picker, KeyCode::Char('s')), PickerAction::Sync);

    let mut refreshed = rows();
    refreshed.remove(0);
    picker.set_rows(refreshed);
    assert_eq!(picker.selected().map(|row| row.id), Some(3));
    assert_eq!(press(&mut picker, KeyCode::Char('q')), PickerAction::Quit);
}

#[test]
fn syncing_without_credentials_explains_itself() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let mut picker = Picker::new(rows());
    let status = picker.sync(&mut session, None);
    assert!(status.contains("not logged in"), "{status}");
    session.close().unwrap();
}

#[test]
fn the_picker_draws_counts_filter_and_status() {
    let mut picker = Picker::new(rows());
    press(&mut picker, KeyCode::Char('/'));
    press(&mut picker, KeyCode::Char('s'));
    picker.set_status("synced");
    let mut terminal = Terminal::new(TestBackend::new(60, 10)).unwrap();
    terminal.draw(|frame| picker.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    let screen: Vec<String> = (0..10)
        .map(|y| (0..60).map(|x| buffer[(x, y)].symbol()).collect())
        .collect();
    assert!(screen[0].contains("Choose a deck"));
    assert!(
        screen
            .iter()
            .any(|line| line.contains("Spanish") && line.contains("3"))
    );
    assert!(
        !screen.iter().any(|line| line.contains("Geography")),
        "filtered out"
    );
    assert!(screen[8].contains("/s"), "{}", screen[8]);
    assert!(screen[9].contains("synced"));
}
