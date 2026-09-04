mod common;

use std::path::Path;

use anki::card::CardId;
use anki::notes::NoteId;
use anki_proto::scheduler::bury_or_suspend_cards_request::Mode as BuryOrSuspendMode;
use common::{add_basic, fresh_collection, json, yaac_on};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use yaac::config::Config;
use yaac::session::Session;
use yaac::tui::browse::{self, BrowseAction, Browser, TagMode};
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

fn draw(browser: &mut Browser, width: u16, height: u16, media: &Path) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut images = Images::disabled(media);
    terminal
        .draw(|frame| browser.draw(frame, &mut images))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn screen(browser: &mut Browser, width: u16, height: u16, media: &Path) -> Vec<String> {
    rows(&draw(browser, width, height, media))
}

fn first_card(session: &mut Session, nid: i64) -> CardId {
    session.col.storage.all_cards_of_note(NoteId(nid)).unwrap()[0].id()
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
fn the_screen_shows_the_list_above_the_selected_notes_fields_and_cards() {
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

    let lines = screen(&mut browser, 100, 30, &media);
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
    let detail = |needle: &str| {
        lines
            .iter()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} in the detail pane"))
    };
    let header = detail("Basic");
    assert!(header > list_row + 1, "the note sits below the list");
    assert!(lines[header].contains("Default") && lines[header].contains("food"));
    let front = detail("Front");
    assert!(lines[front + 1].contains("apple"));
    let back = detail("Back");
    assert!(lines[back + 1].contains("a fruit"));
    assert!(
        lines[back + 2].contains("with seeds"),
        "<br> breaks the line"
    );
    let cards = detail("Cards");
    assert!(lines[cards + 1].contains("Card 1") && lines[cards + 1].contains("new"));
    assert!(lines[28].contains("e edit"), "{}", lines[28]);
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

    let lines = screen(&mut browser, 300, 30, &media);
    let text_rows: Vec<&String> = lines.iter().filter(|line| line.contains("word")).collect();
    assert!(text_rows.len() >= 3, "wrapped: {text_rows:?}");
    for line in text_rows {
        // One column of inset, then at most 120 columns of text.
        assert!(
            line.trim_end().len() <= 121,
            "no wider than 120 columns: {}",
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

#[test]
fn the_list_marks_flagged_marked_and_suspended_notes_and_the_header_names_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let apple = add_basic(&path, "apple", "flagged");
    let banana = add_basic(&path, "banana", "marked");
    let cherry = add_basic(&path, "cherry", "suspended");
    add_basic(&path, "date", "plain");
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let card = first_card(&mut session, apple);
    session.col.set_card_flag(&[card], 1).unwrap();
    session
        .col
        .add_tags_to_notes(&[NoteId(banana)], "marked")
        .unwrap();
    let card = first_card(&mut session, cherry);
    session
        .col
        .bury_or_suspend_cards(&[card], BuryOrSuspendMode::Suspend)
        .unwrap();
    let media = dir.path().join("collection.media");
    let mut browser = Browser::new("deck:Default");
    browse::search(&mut session, &mut browser).unwrap();

    let buffer = draw(&mut browser, 100, 30, &media);
    let lines = rows(&buffer);
    let list_row = |name: &str| {
        lines
            .iter()
            .position(|line| {
                line.starts_with(&format!("▶ {name}")) || line.starts_with(&format!("  {name}"))
            })
            .unwrap_or_else(|| panic!("{name} in the list"))
    };
    let cell = |x: usize, y: usize| buffer[(x as u16, y as u16)].clone();
    let (apple_row, banana_row, cherry_row, date_row) = (
        list_row("apple"),
        list_row("banana"),
        list_row("cherry"),
        list_row("date"),
    );
    let flag_x = lines[apple_row]
        .chars()
        .position(|c| c == '⚑')
        .expect("a flag glyph on the flagged note");
    assert_eq!(cell(flag_x, apple_row).fg, Color::Red);
    let tail = |row: usize| lines[row].chars().skip(flag_x).collect::<String>();
    assert_eq!(
        tail(apple_row),
        "⚑  Default",
        "the marks hug the deck name rather than the front"
    );
    assert_eq!(
        tail(banana_row),
        " ★ Default",
        "the star has its own cell after the flag's, so the two line up"
    );
    assert_eq!(cell(flag_x + 1, banana_row).fg, Color::Yellow);
    assert!(
        cell(2, cherry_row).modifier.contains(Modifier::DIM),
        "a suspended note's name is dimmed"
    );
    assert!(!cell(2, apple_row).modifier.contains(Modifier::DIM));
    assert_eq!(tail(date_row), "   Default", "nothing on a plain note");

    // The column is there on a terminal too narrow for the deck column.
    let lines = screen(&mut browser, 40, 20, &media);
    let apple_row = lines
        .iter()
        .position(|line| line.starts_with("▶ apple"))
        .unwrap();
    assert!(lines[apple_row].contains('⚑'), "{}", lines[apple_row]);
    assert!(!lines[apple_row].contains("Default"));

    // The header names the state, next to the notetype and deck.
    let header = |lines: &[String]| {
        lines
            .iter()
            .find(|line| line.contains("Basic"))
            .cloned()
            .expect("the header line")
    };
    let lines = screen(&mut browser, 100, 30, &media);
    assert!(header(&lines).contains("⚑ red"), "{}", header(&lines));
    press(&mut browser, KeyCode::Char('j'));
    let lines = screen(&mut browser, 100, 30, &media);
    assert!(header(&lines).contains("★ marked"), "{}", header(&lines));
    assert!(!header(&lines).contains('⚑'));
    press(&mut browser, KeyCode::Char('j'));
    let lines = screen(&mut browser, 100, 30, &media);
    assert!(header(&lines).contains("suspended"), "{}", header(&lines));
    session.close().unwrap();
}

#[test]
fn s_f_and_m_act_on_every_card_of_the_note() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let output = yaac_on(&path)
        .args([
            "add",
            "-n",
            "Basic (and reversed card)",
            "-d",
            "Default",
            "--json",
        ])
        .arg("Front=sol")
        .arg("Back=sun")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let nid = NoteId(json(&output)[0]["id"].as_i64().unwrap());
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let media = dir.path().join("collection.media");
    let mut browser = Browser::new("deck:Default");
    for c in ['s', 'f', 'm'] {
        assert_eq!(
            press(&mut browser, KeyCode::Char(c)),
            BrowseAction::Continue,
            "nothing to act on before the search ran"
        );
    }
    browse::search(&mut session, &mut browser).unwrap();
    assert_eq!(browser.selected().unwrap().cards.len(), 2);
    let status = |browser: &mut Browser| screen(browser, 100, 24, &media)[23].clone();

    // s suspends every card; once all are suspended it brings them all back.
    assert_eq!(
        press(&mut browser, KeyCode::Char('s')),
        BrowseAction::Suspend(nid)
    );
    browse::toggle_suspend(&mut session, &mut browser, nid).unwrap();
    let note = browser.selected().unwrap();
    assert!(note.suspended());
    assert!(note.cards.iter().all(|card| card.queue == "suspended"));
    assert!(status(&mut browser).contains("suspended 2 card"));
    let first = first_card(&mut session, nid.0);
    session.col.unbury_or_unsuspend_cards(&[first]).unwrap();
    browse::search(&mut session, &mut browser).unwrap();
    assert!(!browser.selected().unwrap().suspended());
    browse::toggle_suspend(&mut session, &mut browser, nid).unwrap();
    assert!(
        browser.selected().unwrap().suspended(),
        "a partly suspended note gets suspended, not unsuspended"
    );
    browse::toggle_suspend(&mut session, &mut browser, nid).unwrap();
    let note = browser.selected().unwrap();
    assert!(note.cards.iter().all(|card| card.queue == "new"));
    assert!(status(&mut browser).contains("unsuspended 2 card"));

    // m toggles the marked tag on the note.
    assert_eq!(
        press(&mut browser, KeyCode::Char('m')),
        BrowseAction::Mark(nid)
    );
    browse::toggle_mark(&mut session, &mut browser, nid).unwrap();
    assert!(browser.selected().unwrap().marked());
    assert_eq!(
        yaac::notes::get_note(&mut session.col, nid).unwrap().tags,
        ["marked"]
    );
    assert!(status(&mut browser).contains("marked"));
    browse::toggle_mark(&mut session, &mut browser, nid).unwrap();
    assert!(!browser.selected().unwrap().marked());
    assert!(
        yaac::notes::get_note(&mut session.col, nid)
            .unwrap()
            .tags
            .is_empty()
    );

    // f steps every card through the seven colours and back to none.
    assert_eq!(
        press(&mut browser, KeyCode::Char('f')),
        BrowseAction::Flag(nid)
    );
    browse::cycle_flag(&mut session, &mut browser, nid).unwrap();
    assert!(
        browser
            .selected()
            .unwrap()
            .cards
            .iter()
            .all(|card| card.flag == 1)
    );
    assert!(status(&mut browser).contains("flagged red"));
    for _ in 0..6 {
        browse::cycle_flag(&mut session, &mut browser, nid).unwrap();
    }
    assert_eq!(browser.selected().unwrap().flag(), 7);
    browse::cycle_flag(&mut session, &mut browser, nid).unwrap();
    assert!(
        browser
            .selected()
            .unwrap()
            .cards
            .iter()
            .all(|card| card.flag == 0)
    );
    assert!(status(&mut browser).contains("flag removed"));
    // With flags differing between cards, the first flagged card sets the pace.
    let second = session.col.storage.all_cards_of_note(nid).unwrap()[1].id();
    session.col.set_card_flag(&[second], 3).unwrap();
    browse::search(&mut session, &mut browser).unwrap();
    assert_eq!(browser.selected().unwrap().flag(), 3);
    browse::cycle_flag(&mut session, &mut browser, nid).unwrap();
    assert!(
        browser
            .selected()
            .unwrap()
            .cards
            .iter()
            .all(|card| card.flag == 4)
    );

    // u undoes these like any other change.
    assert_eq!(press(&mut browser, KeyCode::Char('u')), BrowseAction::Undo);
    session.col.undo().unwrap();
    browse::search(&mut session, &mut browser).unwrap();
    assert_eq!(browser.selected().unwrap().flag(), 3);

    let lines = screen(&mut browser, 100, 30, &media);
    assert!(lines[28].contains("s suspend"), "{}", lines[28]);
    press(&mut browser, KeyCode::Char('?'));
    let lines = screen(&mut browser, 100, 30, &media);
    assert!(
        lines.iter().any(|line| line.contains("unsuspend")),
        "{lines:#?}"
    );

    // While typing, the letters go into the query.
    press(&mut browser, KeyCode::Char('q'));
    press(&mut browser, KeyCode::Char('/'));
    for c in ['s', 'f', 'm'] {
        assert_eq!(press(&mut browser, KeyCode::Char(c)), BrowseAction::Search);
    }
    assert_eq!(browser.query(), "deck:Defaultsfm");
    session.close().unwrap();
}

fn add_tagged(path: &Path, front: &str, tags: &str) -> NoteId {
    let output = yaac_on(path)
        .args([
            "add", "-n", "Basic", "-d", "Default", "--json", "--tag", tags,
        ])
        .arg(format!("Front={front}"))
        .arg("Back=x")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    NoteId(json(&output)[0]["id"].as_i64().unwrap())
}

fn tags_of(session: &mut Session, nid: NoteId) -> Vec<String> {
    let mut tags = yaac::notes::get_note(&mut session.col, nid).unwrap().tags;
    tags.sort();
    tags
}

fn type_text(browser: &mut Browser, text: &str) {
    for c in text.chars() {
        assert_eq!(
            press(browser, KeyCode::Char(c)),
            BrowseAction::Continue,
            "{c:?} goes into the prompt"
        );
    }
}

#[test]
fn t_and_shift_t_prompt_for_tags_complete_them_and_apply_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let apple = add_tagged(&path, "apple", "fruit food fresh");
    let banana = add_tagged(&path, "banana", "fruit");
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let media = dir.path().join("collection.media");
    let mut browser = Browser::new("deck:Default");
    assert_eq!(
        press(&mut browser, KeyCode::Char('t')),
        BrowseAction::Continue,
        "nothing to tag before the search ran"
    );
    browse::search(&mut session, &mut browser).unwrap();
    press(&mut browser, KeyCode::Char('j'));
    assert_eq!(browser.selected().map(|note| note.id), Some(banana.0));
    let bottom = |browser: &mut Browser| {
        let lines = screen(browser, 100, 24, &media);
        (lines[22].clone(), lines[23].clone())
    };

    // t asks for tags to add and completes them from the whole collection.
    assert_eq!(
        press(&mut browser, KeyCode::Char('t')),
        BrowseAction::TagPrompt(banana, TagMode::Add)
    );
    browse::prompt_tags(&mut session, &mut browser, banana, TagMode::Add).unwrap();
    let (help, prompt) = bottom(&mut browser);
    assert!(prompt.contains("add tags: ▏"), "{prompt}");
    assert!(help.contains("tab complete"), "{help}");
    type_text(&mut browser, "f");
    let (_, prompt) = bottom(&mut browser);
    assert!(
        prompt.contains("f▏") && prompt.contains("food") && prompt.contains("fresh"),
        "candidates for the word being typed: {prompt}"
    );
    assert_eq!(press(&mut browser, KeyCode::Tab), BrowseAction::Continue);
    let (_, prompt) = bottom(&mut browser);
    assert!(prompt.contains("add tags: food▏"), "{prompt}");
    press(&mut browser, KeyCode::Tab);
    assert!(bottom(&mut browser).1.contains("add tags: fresh▏"));
    press(&mut browser, KeyCode::Tab);
    assert!(bottom(&mut browser).1.contains("add tags: fruit▏"));
    press(&mut browser, KeyCode::Tab);
    assert!(
        bottom(&mut browser).1.contains("add tags: food▏"),
        "tab wraps around"
    );
    press(&mut browser, KeyCode::BackTab);
    assert!(bottom(&mut browser).1.contains("add tags: fruit▏"));
    press(&mut browser, KeyCode::BackTab);
    press(&mut browser, KeyCode::BackTab);
    assert!(bottom(&mut browser).1.contains("add tags: food▏"));
    type_text(&mut browser, " fr");
    press(&mut browser, KeyCode::Tab);
    assert!(
        bottom(&mut browser).1.contains("add tags: food fresh▏"),
        "only the word at the cursor is completed"
    );
    assert_eq!(
        press(&mut browser, KeyCode::Enter),
        BrowseAction::Tags(
            banana,
            TagMode::Add,
            vec!["food".to_string(), "fresh".to_string()]
        )
    );
    browse::apply_tags(
        &mut session,
        &mut browser,
        banana,
        TagMode::Add,
        &["food".to_string(), "fresh".to_string()],
    )
    .unwrap();
    assert_eq!(tags_of(&mut session, banana), ["food", "fresh", "fruit"]);
    let lines = screen(&mut browser, 100, 30, &media);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Basic") && line.contains("fresh")),
        "the header shows the new tags"
    );
    assert!(lines[29].contains("added 2 tag"), "{}", lines[29]);
    browse::apply_tags(
        &mut session,
        &mut browser,
        banana,
        TagMode::Add,
        &["fruit".to_string()],
    )
    .unwrap();
    assert!(bottom(&mut browser).1.contains("already tagged"));

    // T asks for tags to remove and completes from the note's own tags, so a bare
    // tab cycles through them.
    assert_eq!(
        press(&mut browser, KeyCode::Char('T')),
        BrowseAction::TagPrompt(banana, TagMode::Remove)
    );
    browse::prompt_tags(&mut session, &mut browser, banana, TagMode::Remove).unwrap();
    assert!(bottom(&mut browser).1.contains("remove tags: ▏"));
    press(&mut browser, KeyCode::Tab);
    assert!(bottom(&mut browser).1.contains("remove tags: food▏"));
    assert_eq!(
        press(&mut browser, KeyCode::Enter),
        BrowseAction::Tags(banana, TagMode::Remove, vec!["food".to_string()])
    );
    browse::apply_tags(
        &mut session,
        &mut browser,
        banana,
        TagMode::Remove,
        &["food".to_string()],
    )
    .unwrap();
    assert_eq!(tags_of(&mut session, banana), ["fresh", "fruit"]);
    assert!(bottom(&mut browser).1.contains("removed 1 tag"));
    browse::apply_tags(
        &mut session,
        &mut browser,
        banana,
        TagMode::Remove,
        &["nothing".to_string()],
    )
    .unwrap();
    assert!(bottom(&mut browser).1.contains("no such tag"));

    // u undoes the last change, like everything else.
    assert_eq!(press(&mut browser, KeyCode::Char('u')), BrowseAction::Undo);
    session.col.undo().unwrap();
    browse::search(&mut session, &mut browser).unwrap();
    assert_eq!(tags_of(&mut session, banana), ["food", "fresh", "fruit"]);

    // Esc cancels, enter on an empty prompt just closes it, ctrl-u clears.
    press(&mut browser, KeyCode::Char('t'));
    browse::prompt_tags(&mut session, &mut browser, banana, TagMode::Add).unwrap();
    type_text(&mut browser, "later");
    assert_eq!(press(&mut browser, KeyCode::Esc), BrowseAction::Continue);
    assert!(!bottom(&mut browser).1.contains("add tags"));
    assert_eq!(tags_of(&mut session, banana), ["food", "fresh", "fruit"]);
    press(&mut browser, KeyCode::Char('t'));
    browse::prompt_tags(&mut session, &mut browser, banana, TagMode::Add).unwrap();
    type_text(&mut browser, "later");
    assert_eq!(ctrl(&mut browser, 'u'), BrowseAction::Continue);
    assert!(bottom(&mut browser).1.contains("add tags: ▏"));
    assert_eq!(press(&mut browser, KeyCode::Enter), BrowseAction::Continue);
    assert!(!bottom(&mut browser).1.contains("add tags"));
    assert_eq!(
        press(&mut browser, KeyCode::Char('q')),
        BrowseAction::Quit,
        "the prompt is closed, keys are shortcuts again"
    );

    // The apple's tags are untouched throughout.
    assert_eq!(tags_of(&mut session, apple), ["food", "fresh", "fruit"]);
    press(&mut browser, KeyCode::Char('?'));
    let lines = screen(&mut browser, 100, 30, &media);
    assert!(
        lines.iter().any(|line| line.contains("add tags")),
        "{lines:#?}"
    );
    press(&mut browser, KeyCode::Esc);

    // While typing a search, t and T are part of the query.
    press(&mut browser, KeyCode::Char('/'));
    assert_eq!(
        press(&mut browser, KeyCode::Char('t')),
        BrowseAction::Search
    );
    assert_eq!(
        press(&mut browser, KeyCode::Char('T')),
        BrowseAction::Search
    );
    assert_eq!(browser.query(), "deck:DefaulttT");
    session.close().unwrap();
}

#[test]
fn enter_runs_the_search_again_and_stays_near_the_selected_note() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let apple = add_tagged(&path, "apple", "todo");
    let banana = add_tagged(&path, "banana", "todo");
    let cherry = add_tagged(&path, "cherry", "todo");
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let mut browser = Browser::new("tag:todo");
    browse::search(&mut session, &mut browser).unwrap();
    press(&mut browser, KeyCode::Char('j'));
    assert_eq!(browser.selected().map(|note| note.id), Some(banana.0));

    // The note stays listed after the change, until the search runs again.
    browse::apply_tags(
        &mut session,
        &mut browser,
        banana,
        TagMode::Remove,
        &["todo".to_string()],
    )
    .unwrap();
    assert_eq!(browser.notes().len(), 3);
    assert_eq!(press(&mut browser, KeyCode::Enter), BrowseAction::Rerun);
    browse::rerun(&mut session, &mut browser).unwrap();
    assert_eq!(browser.notes().len(), 2);
    assert_eq!(
        browser.selected().map(|note| note.id),
        Some(cherry.0),
        "the selection moves to the note that took the place"
    );
    press(&mut browser, KeyCode::Char('k'));
    browse::rerun(&mut session, &mut browser).unwrap();
    assert_eq!(
        browser.selected().map(|note| note.id),
        Some(apple.0),
        "a note still listed keeps the selection"
    );
    session.close().unwrap();
}
