mod common;

use anki::decks::DeckId;
use anki::scheduler::answering::Rating;
use common::{add_basic, fresh_collection};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use yaac::config::Config;
use yaac::review::{Kind, Reviewer};
use yaac::session::Session;
use yaac::tui::review::{self, AGAIN, EASY, GOOD, HARD};

fn rows(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn reviews_cards_through_ankis_scheduler() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    add_basic(&path, "uno", "one");
    add_basic(&path, "dos", "two");
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let mut reviewer = Reviewer::start(&mut session.col, DeckId(1)).unwrap();

    assert_eq!(reviewer.deck, "Default");
    assert_eq!(reviewer.counts.new, 2);
    let current = reviewer.current.as_ref().expect("a card is due");
    assert_eq!(current.kind, Kind::New);
    assert!(!current.revealed);
    assert!(current.question.contains("uno"), "{}", current.question);
    assert!(
        !current.question.contains("one"),
        "answer hidden on the front"
    );
    assert!(current.answer.contains("one"), "{}", current.answer);
    assert!(current.labels.iter().all(|label| !label.is_empty()));

    reviewer.reveal();
    assert!(reviewer.current.as_ref().unwrap().revealed);
    reviewer.answer(Rating::Good).unwrap();
    assert_eq!(reviewer.answered, 1);
    assert_eq!(reviewer.counts.new, 1);
    assert!(reviewer.current.as_ref().unwrap().question.contains("dos"));

    assert!(reviewer.undo().unwrap(), "an answer can be undone");
    assert_eq!(reviewer.answered, 0);
    assert_eq!(reviewer.counts.new, 2);
    assert!(reviewer.current.as_ref().unwrap().question.contains("uno"));

    reviewer.reveal();
    reviewer.answer(Rating::Again).unwrap();
    assert_eq!(
        reviewer.counts.learning, 1,
        "Again puts the card into learning"
    );

    reviewer.cycle_flag().unwrap();
    assert_eq!(reviewer.current.as_ref().unwrap().flag, 1);
    reviewer.suspend().unwrap();
    assert_eq!(reviewer.counts.new, 0, "the second card was suspended");

    let mut guard = 0;
    while !reviewer.done() {
        reviewer.reveal();
        reviewer.answer(Rating::Good).unwrap();
        guard += 1;
        assert!(guard < 10, "learning steps must run out");
    }
    assert_eq!(reviewer.counts, Default::default());
    assert!(reviewer.answered >= 2);
    drop(reviewer);
    session.close().unwrap();
}

#[test]
fn keys_drive_the_reviewer() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    add_basic(&path, "tres", "three");
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let mut reviewer = Reviewer::start(&mut session.col, DeckId(1)).unwrap();

    review::handle(&mut reviewer, KeyEvent::from(KeyCode::Char('3'))).unwrap();
    assert_eq!(
        reviewer.answered, 0,
        "answers are ignored before the reveal"
    );
    review::handle(&mut reviewer, KeyEvent::from(KeyCode::Char(' '))).unwrap();
    assert!(reviewer.current.as_ref().unwrap().revealed);
    review::handle(&mut reviewer, KeyEvent::from(KeyCode::Char('3'))).unwrap();
    assert_eq!(reviewer.answered, 1);
    // Good on a new card starts a learning step, and with nothing else due Anki's
    // learn-ahead window shows it again immediately rather than ending the session.
    assert_eq!(reviewer.current.as_ref().unwrap().kind, Kind::Learning);
    assert!(!reviewer.current.as_ref().unwrap().revealed);
    assert!(matches!(
        review::handle(&mut reviewer, KeyEvent::from(KeyCode::Char('q'))).unwrap(),
        review::Action::Quit
    ));
}

#[test]
fn the_screen_centers_the_card_and_colors_the_answers() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    add_basic(&path, "Capital of France?", "Paris");
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let mut reviewer = Reviewer::start(&mut session.col, DeckId(1)).unwrap();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

    terminal
        .draw(|frame| review::draw(frame, &reviewer))
        .unwrap();
    let screen = rows(terminal.backend().buffer());
    assert!(screen[0].contains("Default"), "deck in the status line");
    assert!(screen[0].contains("new 1"), "{}", screen[0]);
    assert!(screen[0].contains("0 answered"));
    let row = screen
        .iter()
        .position(|line| line.contains("Capital of France?"))
        .expect("question on screen");
    assert!(
        (10..=13).contains(&row),
        "vertically centered, got row {row}"
    );
    let start = screen[row].find("Capital").unwrap();
    let end = start + "Capital of France?".len();
    assert!(
        start.abs_diff(80 - end) <= 2,
        "horizontally centered: starts at {start}, ends at {end}"
    );
    assert!(!screen.iter().any(|line| line.contains("Paris")));
    assert!(screen[22].contains("show answer"));

    reviewer.reveal();
    terminal
        .draw(|frame| review::draw(frame, &reviewer))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let screen = rows(&buffer);
    assert!(screen.iter().any(|line| line.contains("Paris")));
    let actions = &screen[22];
    for name in ["Again", "Hard", "Good", "Easy"] {
        assert!(actions.contains(name), "{actions}");
    }
    let color_of = |name: &str| {
        let x = actions.find(name).unwrap() as u16;
        buffer[(x, 22)].fg
    };
    assert_eq!(color_of("Again"), AGAIN);
    assert_eq!(color_of("Hard"), HARD);
    assert_eq!(color_of("Good"), GOOD);
    assert_eq!(color_of("Easy"), EASY);
    assert_ne!(AGAIN, Color::Reset);
}
