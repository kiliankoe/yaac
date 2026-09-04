mod common;

use anki::notes::NoteId;
use common::{add_basic, fresh_collection, yaac_on};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use yaac::config::Config;
use yaac::session::Session;
use yaac::tui::browse::{self, BrowseAction, Browser};
use yaac::tui::images::Images;

fn rows(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

fn press(browser: &mut Browser, code: KeyCode) -> BrowseAction {
    browser.handle(KeyEvent::from(code))
}

fn ctrl(browser: &mut Browser, c: char) -> BrowseAction {
    browser.handle(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

fn screen(browser: &mut Browser, width: u16, height: u16, media: &std::path::Path) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut images = Images::disabled(media);
    terminal
        .draw(|frame| browser.draw(frame, &mut images))
        .unwrap();
    rows(terminal.backend().buffer())
}

#[test]
fn typing_searches_on_every_key_and_enter_leaves_the_box() {
    let mut browser = Browser::new("");
    for c in "deck:Default".chars() {
        assert_eq!(press(&mut browser, KeyCode::Char(c)), BrowseAction::Search);
    }
    assert_eq!(browser.query(), "deck:Default");
    assert_eq!(press(&mut browser, KeyCode::Enter), BrowseAction::Continue);
    assert_eq!(
        press(&mut browser, KeyCode::Char('q')),
        BrowseAction::Quit,
        "after enter, letters are shortcuts again"
    );

    press(&mut browser, KeyCode::Char('/'));
    assert_eq!(
        press(&mut browser, KeyCode::Backspace),
        BrowseAction::Search
    );
    assert_eq!(browser.query(), "deck:Defaul");
    assert_eq!(press(&mut browser, KeyCode::Esc), BrowseAction::Continue);
    assert_eq!(
        browser.query(),
        "deck:Defaul",
        "esc leaves the box, keeps the text"
    );
    press(&mut browser, KeyCode::Char('/'));
    assert_eq!(ctrl(&mut browser, 'u'), BrowseAction::Search);
    assert_eq!(browser.query(), "");
    assert_eq!(ctrl(&mut browser, 'c'), BrowseAction::Quit);
}

#[test]
fn arrows_move_the_list_while_typing_and_an_empty_query_lists_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let carrot = add_basic(&path, "carrot", "vegetable");
    let apple = add_basic(&path, "apple", "fruit");
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let media = dir.path().join("collection.media");
    let mut browser = Browser::new("");

    press(&mut browser, KeyCode::Char('a'));
    browse::search(&mut session, &mut browser).unwrap();
    assert_eq!(browser.notes().len(), 2);
    assert_eq!(press(&mut browser, KeyCode::Down), BrowseAction::Continue);
    assert_eq!(browser.selected().map(|note| note.id), Some(carrot));
    press(&mut browser, KeyCode::Up);
    assert_eq!(browser.selected().map(|note| note.id), Some(apple));
    assert_eq!(
        press(&mut browser, KeyCode::Char('j')),
        BrowseAction::Search,
        "letters still go into the box"
    );
    assert_eq!(browser.query(), "aj");

    ctrl(&mut browser, 'u');
    browse::search(&mut session, &mut browser).unwrap();
    assert!(browser.notes().is_empty());
    assert_eq!(browser.selected().map(|note| note.id), None);
    let lines = screen(&mut browser, 100, 10, &media);
    assert!(lines.iter().any(|line| line.contains("Type a search")));

    for c in "deck:*".chars() {
        press(&mut browser, KeyCode::Char(c));
    }
    browse::search(&mut session, &mut browser).unwrap();
    assert_eq!(browser.notes().len(), 2, "deck:* lists every note");
    session.close().unwrap();
}

#[test]
fn a_query_on_the_command_line_lists_notes_by_sort_field_and_keys_drive_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let carrot = add_basic(&path, "carrot", "vegetable");
    let apple = add_basic(&path, "apple", "fruit");
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let mut browser = Browser::new("deck:Default");
    assert_eq!(
        press(&mut browser, KeyCode::Char('e')),
        BrowseAction::Continue,
        "nothing to edit before the search ran"
    );

    browse::search(&mut session, &mut browser).unwrap();
    let ids: Vec<i64> = browser.notes().iter().map(|note| note.id).collect();
    assert_eq!(ids, [apple, carrot], "alphabetical, not by creation");
    assert_eq!(browser.selected().map(|note| note.id), Some(apple));
    press(&mut browser, KeyCode::Char('j'));
    assert_eq!(browser.selected().map(|note| note.id), Some(carrot));
    press(&mut browser, KeyCode::Char('j'));
    assert_eq!(
        browser.selected().map(|note| note.id),
        Some(carrot),
        "stops at the end"
    );
    assert_eq!(
        press(&mut browser, KeyCode::Char('e')),
        BrowseAction::Edit(NoteId(carrot))
    );
    press(&mut browser, KeyCode::Char('g'));
    assert_eq!(browser.selected().map(|note| note.id), Some(apple));
    assert_eq!(press(&mut browser, KeyCode::Char('u')), BrowseAction::Undo);
    assert_eq!(
        press(&mut browser, KeyCode::Char('r')),
        BrowseAction::Redraw
    );

    // A search Anki rejects becomes a status line and keeps the previous results.
    press(&mut browser, KeyCode::Char('/'));
    for c in " prop:nonsense".chars() {
        press(&mut browser, KeyCode::Char(c));
    }
    assert_eq!(press(&mut browser, KeyCode::Enter), BrowseAction::Continue);
    browse::search(&mut session, &mut browser).unwrap();
    assert_eq!(browser.notes().len(), 2);
    let media = dir.path().join("collection.media");
    let lines = screen(&mut browser, 100, 24, &media);
    assert!(lines[23].contains("search failed"), "{}", lines[23]);
    session.close().unwrap();
}

#[test]
fn the_screen_shows_the_list_beside_the_selected_notes_fields_and_cards() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    yaac_on(&path)
        .args(["add", "-n", "Basic", "-d", "Default", "--tag", "food"])
        .arg("Front=apple")
        .arg("Back=a <b>fruit</b><br>with seeds")
        .assert()
        .success();
    add_basic(&path, "carrot", "vegetable");
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let mut browser = Browser::new("deck:Default");
    browse::search(&mut session, &mut browser).unwrap();
    let media = dir.path().join("collection.media");

    let lines = screen(&mut browser, 100, 24, &media);
    assert!(lines[0].contains("deck:Default"), "{}", lines[0]);
    assert!(lines[0].contains("2 notes"), "{}", lines[0]);
    let list_row = lines
        .iter()
        .position(|line| line.starts_with("▶ apple"))
        .expect("selected note first in the list");
    assert!(
        lines[list_row + 1].starts_with("  carrot"),
        "{}",
        lines[list_row + 1]
    );
    // Side by side: the detail pane starts to the right of the list on the same rows.
    let detail = |needle: &str| {
        lines
            .iter()
            .position(|line| line[40..].contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} in the detail pane"))
    };
    let header = detail("Basic");
    assert!(lines[header].contains("Default") && lines[header].contains("food"));
    let front = detail("Front");
    assert!(lines[front + 1][40..].contains("apple"));
    let back = detail("Back");
    assert!(lines[back + 1][40..].contains("a fruit"));
    assert!(
        lines[back + 2][40..].contains("with seeds"),
        "<br> breaks the line"
    );
    let cards = detail("Cards");
    assert!(lines[cards + 1].contains("Card 1") && lines[cards + 1].contains("new"));
    assert!(lines[22].contains("e edit"), "{}", lines[22]);

    // Narrow terminals stack the panes.
    let lines = screen(&mut browser, 80, 24, &media);
    let list_row = lines
        .iter()
        .position(|line| line.starts_with("▶ apple"))
        .unwrap();
    let front = lines
        .iter()
        .position(|line| line.trim_start().starts_with("Front"))
        .unwrap();
    assert!(front > list_row, "detail below the list");
    session.close().unwrap();
}

#[test]
fn the_detail_pane_scrolls_and_the_selection_resets_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let long: Vec<String> = (1..=40).map(|i| format!("line {i}")).collect();
    add_basic(&path, "long", &long.join("<br>"));
    add_basic(&path, "short", "x");
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let mut browser = Browser::new("deck:Default");
    browse::search(&mut session, &mut browser).unwrap();
    let media = dir.path().join("collection.media");

    let lines = screen(&mut browser, 100, 20, &media);
    assert!(lines.iter().any(|line| line.contains("line 1 ")));
    assert!(!lines.iter().any(|line| line.contains("line 40")));
    for _ in 0..10 {
        press(&mut browser, KeyCode::PageDown);
        screen(&mut browser, 100, 20, &media);
    }
    let lines = screen(&mut browser, 100, 20, &media);
    assert!(
        lines.iter().any(|line| line.contains("line 40")),
        "{lines:#?}"
    );
    assert!(!lines.iter().any(|line| line.contains("Front")));
    ctrl(&mut browser, 'u');
    let lines = screen(&mut browser, 100, 20, &media);
    assert!(!lines.iter().any(|line| line.contains("line 40")));

    press(&mut browser, KeyCode::Char('j'));
    press(&mut browser, KeyCode::Char('k'));
    let lines = screen(&mut browser, 100, 20, &media);
    assert!(
        lines.iter().any(|line| line.contains("Front")),
        "back at the top after moving"
    );
    session.close().unwrap();
}

#[test]
fn an_empty_search_box_explains_itself() {
    let dir = tempfile::tempdir().unwrap();
    let mut browser = Browser::new("");
    let media = dir.path().join("collection.media");
    let lines = screen(&mut browser, 100, 10, &media);
    assert!(lines[0].contains("/ ▏"), "{}", lines[0]);
    assert!(lines.iter().any(|line| line.contains("Type a search")));
    assert!(lines[8].contains("enter/esc done"), "{}", lines[8]);
}

#[test]
fn question_mark_shows_the_keys_and_closing_them_refreshes_images() {
    let dir = tempfile::tempdir().unwrap();
    let media = dir.path().join("collection.media");
    let mut browser = Browser::new("deck:*");
    assert_eq!(
        press(&mut browser, KeyCode::Char('?')),
        BrowseAction::Continue
    );
    let lines = screen(&mut browser, 100, 24, &media);
    assert!(
        lines.iter().any(|line| line.contains("ctrl-d")),
        "{lines:#?}"
    );
    assert!(lines.iter().any(|line| line.contains("undo")), "{lines:#?}");
    assert_eq!(
        press(&mut browser, KeyCode::Char('q')),
        BrowseAction::Refresh,
        "the closing key is swallowed, images under the box are sent again"
    );
    assert_eq!(press(&mut browser, KeyCode::Char('q')), BrowseAction::Quit);

    // While typing, ? is part of the query.
    press(&mut browser, KeyCode::Char('/'));
    assert_eq!(
        press(&mut browser, KeyCode::Char('?')),
        BrowseAction::Search
    );
    assert_eq!(browser.query(), "deck:*?");
}

#[test]
fn the_detail_pane_wraps_long_fields_at_a_readable_width() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let words: Vec<String> = (1..=60).map(|i| format!("word{i}")).collect();
    add_basic(&path, "long", &words.join(" "));
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let mut browser = Browser::new("deck:Default");
    browse::search(&mut session, &mut browser).unwrap();
    let media = dir.path().join("collection.media");

    let lines = screen(&mut browser, 300, 24, &media);
    let text_rows: Vec<&String> = lines.iter().filter(|line| line.contains("word")).collect();
    assert!(text_rows.len() >= 3, "wrapped: {text_rows:?}");
    for line in text_rows {
        // The list takes 40% (120 columns), the pane border and inset 2 more.
        assert!(
            line.trim_end().len() <= 122 + 120,
            "no wider than 120 columns after the list: {}",
            line.trim_end().len()
        );
    }
    session.close().unwrap();
}

#[test]
fn d_asks_before_deleting_and_y_deletes_the_note() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let carrot = add_basic(&path, "carrot", "vegetable");
    let apple = add_basic(&path, "apple", "fruit");
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let media = dir.path().join("collection.media");
    let mut browser = Browser::new("deck:Default");
    browse::search(&mut session, &mut browser).unwrap();
    assert_eq!(browser.selected().map(|note| note.id), Some(apple));

    assert_eq!(
        press(&mut browser, KeyCode::Char('d')),
        BrowseAction::Continue
    );
    let lines = screen(&mut browser, 100, 24, &media);
    assert!(
        lines[23].contains("delete") && lines[23].contains("apple") && lines[23].contains("y"),
        "{}",
        lines[23]
    );
    assert_eq!(
        press(&mut browser, KeyCode::Char('n')),
        BrowseAction::Continue,
        "anything but y cancels"
    );
    let lines = screen(&mut browser, 100, 24, &media);
    assert!(!lines[23].contains("delete"), "{}", lines[23]);
    assert_eq!(browser.notes().len(), 2);

    press(&mut browser, KeyCode::Char('d'));
    assert_eq!(
        press(&mut browser, KeyCode::Char('y')),
        BrowseAction::Delete(NoteId(apple))
    );
    browse::delete(&mut session, &mut browser, NoteId(apple)).unwrap();
    assert_eq!(browser.notes().len(), 1);
    assert_eq!(
        browser.selected().map(|note| note.id),
        Some(carrot),
        "the selection moves to the next note"
    );
    assert!(yaac::notes::get_note(&mut session.col, NoteId(apple)).is_err());
    let lines = screen(&mut browser, 100, 24, &media);
    assert!(lines[23].contains("deleted"), "{}", lines[23]);

    assert_eq!(press(&mut browser, KeyCode::Char('u')), BrowseAction::Undo);
    session.col.undo().unwrap();
    browse::search(&mut session, &mut browser).unwrap();
    assert_eq!(browser.notes().len(), 2, "undo brings the note back");

    // While typing, d is part of the query.
    press(&mut browser, KeyCode::Char('/'));
    assert_eq!(
        press(&mut browser, KeyCode::Char('d')),
        BrowseAction::Search
    );
    session.close().unwrap();
}

#[test]
fn esc_goes_back_and_q_quits_unless_typing() {
    let mut browser = Browser::new("deck:*");
    assert_eq!(press(&mut browser, KeyCode::Esc), BrowseAction::Back);
    assert_eq!(press(&mut browser, KeyCode::Char('q')), BrowseAction::Quit);
    press(&mut browser, KeyCode::Char('/'));
    assert_eq!(
        press(&mut browser, KeyCode::Esc),
        BrowseAction::Continue,
        "esc only leaves the search box"
    );
    assert_eq!(press(&mut browser, KeyCode::Esc), BrowseAction::Back);
}
