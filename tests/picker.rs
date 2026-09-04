mod common;

use std::time::{Duration, Instant};

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
    let picker = Picker::new(rows(), Vec::new());
    assert_eq!(picker.selected().map(|row| row.id), Some(2));
}

#[test]
fn slash_filters_decks_and_enter_selects_the_match() {
    let mut picker = Picker::new(rows(), Vec::new());
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
    let mut picker = Picker::new(rows(), Vec::new());
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
    let mut picker = Picker::new(rows(), Vec::new());
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
    let mut picker = Picker::new(rows(), Vec::new());
    let status = picker.sync(&mut session, None);
    assert!(status.contains("not logged in"), "{status}");
    session.close().unwrap();
}

#[test]
fn the_picker_draws_counts_filter_and_status() {
    let mut picker = Picker::new(rows(), Vec::new());
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

#[test]
fn a_opens_a_notetype_chooser_for_the_selected_deck() {
    let mut picker = Picker::new(rows(), vec!["Basic".to_string(), "Cloze".to_string()]);
    assert_eq!(
        press(&mut picker, KeyCode::Char('a')),
        PickerAction::Continue
    );
    assert!(picker.choosing_notetype());
    assert_eq!(press(&mut picker, KeyCode::Esc), PickerAction::Continue);
    assert!(!picker.choosing_notetype(), "esc closes the chooser");
    assert_eq!(picker.visible().len(), 4, "and does not touch the filter");

    press(&mut picker, KeyCode::Char('a'));
    assert_eq!(
        press(&mut picker, KeyCode::Char('j')),
        PickerAction::Continue
    );
    assert_eq!(
        press(&mut picker, KeyCode::Enter),
        PickerAction::Add {
            deck: DeckId(2),
            notetype: "Cloze".to_string()
        }
    );
    assert!(!picker.choosing_notetype());

    // The chooser starts on the notetype used last, and A skips it altogether.
    press(&mut picker, KeyCode::Char('a'));
    assert_eq!(
        press(&mut picker, KeyCode::Enter),
        PickerAction::Add {
            deck: DeckId(2),
            notetype: "Cloze".to_string()
        }
    );
    press(&mut picker, KeyCode::Char('j'));
    assert_eq!(
        press(&mut picker, KeyCode::Char('A')),
        PickerAction::Add {
            deck: DeckId(3),
            notetype: "Cloze".to_string()
        }
    );
    assert!(!picker.choosing_notetype());

    // Before any note was added, A opens the chooser like a, on the config's default.
    let mut picker = Picker::new(rows(), vec!["Basic".to_string(), "Cloze".to_string()]);
    assert_eq!(
        press(&mut picker, KeyCode::Char('A')),
        PickerAction::Continue
    );
    assert!(picker.choosing_notetype());
    press(&mut picker, KeyCode::Esc);
    picker.set_default_notetype("Cloze");
    assert_eq!(
        press(&mut picker, KeyCode::Char('A')),
        PickerAction::Add {
            deck: DeckId(2),
            notetype: "Cloze".to_string()
        }
    );
}

#[test]
fn the_chooser_draws_over_the_deck_list() {
    let mut picker = Picker::new(rows(), vec!["Basic".to_string(), "Cloze".to_string()]);
    press(&mut picker, KeyCode::Char('a'));
    let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
    terminal.draw(|frame| picker.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    let screen: Vec<String> = (0..12)
        .map(|y| (0..60).map(|x| buffer[(x, y)].symbol()).collect())
        .collect();
    assert!(
        screen.iter().any(|line| line.contains("Spanish")),
        "deck to add to"
    );
    let basic = screen
        .iter()
        .position(|line| line.contains("Basic"))
        .unwrap();
    assert!(screen[basic + 1].contains("Cloze"));
    assert!(
        screen[basic].contains('│'),
        "inside a box: {}",
        screen[basic]
    );
}

#[test]
fn question_mark_shows_the_keys_until_the_next_key() {
    let mut picker = Picker::new(rows(), Vec::new());
    assert_eq!(
        press(&mut picker, KeyCode::Char('?')),
        PickerAction::Continue
    );
    let mut terminal = Terminal::new(TestBackend::new(60, 16)).unwrap();
    terminal.draw(|frame| picker.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    let screen: Vec<String> = (0..16)
        .map(|y| (0..60).map(|x| buffer[(x, y)].symbol()).collect())
        .collect();
    assert!(
        screen.iter().any(|line| line.contains("sync")),
        "{screen:#?}"
    );
    assert!(
        screen.iter().any(|line| line.contains("add a note")),
        "{screen:#?}"
    );
    // The key closing the overlay is not also a shortcut.
    assert_eq!(
        press(&mut picker, KeyCode::Char('q')),
        PickerAction::Continue
    );
    assert_eq!(press(&mut picker, KeyCode::Char('q')), PickerAction::Quit);
}

#[test]
fn adding_from_the_editor_checks_the_note_like_the_cli_does() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let notetype = session.col.get_notetype_by_name("Basic").unwrap().unwrap();
    let editor = common::scripted_editor(
        dir.path(),
        &[
            // An empty first field is sent back with the error on top.
            "tags: new\n\n# Front\n\n\n# Back\n\nx\n",
            "tags: new\n\n# Front\n\nhello\n\n# Back\n\nworld\n",
            // A duplicate is refused once, then accepted when saved again unchanged.
            "# Front\n\nhello\n",
            "# Front\n\nhello\n",
        ],
    );

    let nid = yaac::editor::add_note(&mut session.col, DeckId(1), &notetype, &editor)
        .unwrap()
        .expect("added");
    let seen = common::seen_by_editor(dir.path(), 1);
    assert!(seen.starts_with("<!-- yaac: new Basic note."), "{seen}");
    assert!(
        seen.contains("# Front\n\n\n# Back\n"),
        "empty headings: {seen}"
    );
    let seen = common::seen_by_editor(dir.path(), 2);
    assert!(
        seen.starts_with("<!-- yaac error: the first field is empty"),
        "{seen}"
    );
    let note = yaac::notes::get_note(&mut session.col, nid).unwrap();
    assert_eq!(note.fields().as_slice(), ["hello", "world"]);
    assert_eq!(note.tags, ["new"]);

    let again = yaac::editor::add_note(&mut session.col, DeckId(1), &notetype, &editor)
        .unwrap()
        .expect("added despite the duplicate");
    let seen = common::seen_by_editor(dir.path(), 4);
    assert!(seen.contains("already exists"), "{seen}");
    assert_ne!(again, nid);
    session.close().unwrap();
}

#[test]
fn quitting_the_editor_untouched_aborts_the_add() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let notetype = session.col.get_notetype_by_name("Basic").unwrap().unwrap();
    // `true` leaves the file as it was.
    let editor = yaac::editor::Editor::new("true");
    let added = yaac::editor::add_note(&mut session.col, DeckId(1), &notetype, &editor).unwrap();
    assert_eq!(added, None);
    let nids = session.col.search_notes_unordered("deck:*").unwrap();
    assert!(nids.is_empty(), "nothing was added");
    session.close().unwrap();
}

#[test]
fn b_opens_browse_on_the_selected_deck() {
    use yaac::tui::browse::deck_query;
    let mut picker = Picker::new(rows(), Vec::new());
    assert_eq!(
        press(&mut picker, KeyCode::Char('b')),
        PickerAction::Browse(DeckId(2))
    );
    assert_eq!(deck_query("Spanish"), "deck:Spanish");
    assert_eq!(deck_query("Spanish::Verbs"), "deck:Spanish::Verbs");
    assert_eq!(deck_query("Irregular verbs"), "\"deck:Irregular verbs\"");
    assert_eq!(deck_query("Say \"hi\""), "\"deck:Say \\\"hi\\\"\"");
}

#[test]
fn a_status_message_clears_itself_after_a_few_moments() {
    let mut picker = Picker::new(rows(), Vec::new());
    picker.set_status("synced");
    let now = Instant::now();

    picker.expire_status(now);
    assert!(status_line(&mut picker).contains("synced"), "still fresh");

    picker.expire_status(now + Duration::from_secs(30));
    assert!(
        status_line(&mut picker).trim().is_empty(),
        "the status line is empty again"
    );
}

/// The last row of the picker, where status messages go.
fn status_line(picker: &mut Picker) -> String {
    let mut terminal = Terminal::new(TestBackend::new(60, 10)).unwrap();
    terminal.draw(|frame| picker.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..60).map(|x| buffer[(x, 9)].symbol()).collect()
}
