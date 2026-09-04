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
use yaac::tui::images::Images;
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
    let mut images = Images::disabled(dir.path().join("collection.media"));

    terminal
        .draw(|frame| review::draw(frame, &reviewer, &mut images, None))
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
        .draw(|frame| review::draw(frame, &reviewer, &mut images, None))
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

#[test]
fn esc_goes_back_to_the_deck_list_and_q_quits() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    add_basic(&path, "cuatro", "four");
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let mut reviewer = Reviewer::start(&mut session.col, DeckId(1)).unwrap();

    assert_eq!(
        review::handle(&mut reviewer, KeyEvent::from(KeyCode::Esc)).unwrap(),
        review::Action::Back
    );
    assert_eq!(
        review::handle(&mut reviewer, KeyEvent::from(KeyCode::Char('q'))).unwrap(),
        review::Action::Quit
    );
}

#[test]
fn images_render_as_half_blocks_or_labels() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let media = dir.path().join("collection.media");
    std::fs::create_dir_all(&media).unwrap();
    image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 30, 30, 255]))
        .save(media.join("red.png"))
        .unwrap();
    let output = common::yaac_on(&path)
        .args(["add", "-n", "Basic", "-d", "Default", "--json"])
        .arg("Front=Which colour?<br><img src=\"red.png\">")
        .arg("Back=Red")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(common::json(&output)[0]["fields"]["Back"], "Red");
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let reviewer = Reviewer::start(&mut session.col, DeckId(1)).unwrap();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

    let mut labels = Images::disabled(&media);
    terminal
        .draw(|frame| review::draw(frame, &reviewer, &mut labels, None))
        .unwrap();
    let screen = rows(terminal.backend().buffer());
    assert!(screen.iter().any(|line| line.contains("Which colour?")));
    assert!(
        screen
            .iter()
            .any(|line| line.contains("[image: red.png] (images are off)"))
    );

    let mut blocks = Images::new(Some(ratatui_image::picker::Picker::halfblocks()), &media);
    terminal
        .draw(|frame| review::draw(frame, &reviewer, &mut blocks, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let screen = rows(&buffer);
    let question = screen
        .iter()
        .position(|line| line.contains("Which colour?"))
        .expect("question above the image");
    // A uniform image encodes as blank cells carrying the colour, so look for the
    // colour rather than a glyph.
    let red = Color::Rgb(200, 30, 30);
    let painted = (question as u16 + 1..24)
        .flat_map(|y| (0..80).map(move |x| (x, y)))
        .filter(|&(x, y)| buffer[(x, y)].bg == red || buffer[(x, y)].fg == red)
        .count();
    assert!(
        painted >= 4,
        "expected a row of red cells below the question, got {painted}"
    );
    assert!(
        !screen.iter().any(|line| line.contains("[image:")),
        "no label when drawn"
    );
}

#[test]
fn editing_the_note_rerenders_the_card_and_is_undoable() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    add_basic(&path, "old question", "answer");
    let script = dir.path().join("editor.sh");
    std::fs::write(
        &script,
        "sed 's/^old question$/new question/' \"$1\" > \"$1.tmp\" && mv \"$1.tmp\" \"$1\"\n",
    )
    .unwrap();
    let editor = yaac::editor::Editor::new(format!("sh {}", script.display()));
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let mut reviewer = Reviewer::start(&mut session.col, DeckId(1)).unwrap();
    reviewer.reveal();

    let outcome = reviewer.edit(&editor).unwrap();
    assert_eq!(outcome, Some(yaac::editor::Outcome::Saved));
    let current = reviewer.current.as_ref().unwrap();
    assert!(
        current.question.contains("new question"),
        "{}",
        current.question
    );
    assert!(current.revealed, "the side shown stays the same");
    assert_eq!(
        reviewer.edit(&editor).unwrap(),
        Some(yaac::editor::Outcome::Unchanged),
        "the same edit again changes nothing"
    );

    assert!(reviewer.undo().unwrap(), "the edit is on the undo stack");
    let current = reviewer.current.as_ref().unwrap();
    assert!(
        current.question.contains("old question"),
        "{}",
        current.question
    );
    drop(reviewer);
    session.close().unwrap();
}

#[test]
fn marking_toggles_the_marked_tag_and_is_undoable() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let nid = anki::notes::NoteId(add_basic(&path, "cinco", "five"));
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let mut reviewer = Reviewer::start(&mut session.col, DeckId(1)).unwrap();
    assert!(!reviewer.current.as_ref().unwrap().marked);

    review::handle(&mut reviewer, KeyEvent::from(KeyCode::Char('m'))).unwrap();
    assert!(reviewer.current.as_ref().unwrap().marked);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut images = Images::disabled(dir.path().join("collection.media"));
    terminal
        .draw(|frame| review::draw(frame, &reviewer, &mut images, None))
        .unwrap();
    let screen = rows(terminal.backend().buffer());
    assert!(screen[23].contains("marked"), "{}", screen[23]);

    assert!(reviewer.undo().unwrap());
    assert!(!reviewer.current.as_ref().unwrap().marked);
    reviewer.toggle_mark().unwrap();
    reviewer.toggle_mark().unwrap();
    assert!(!reviewer.current.as_ref().unwrap().marked);
    drop(reviewer);
    let note = yaac::notes::get_note(&mut session.col, nid).unwrap();
    assert!(
        note.tags.is_empty(),
        "unmarking removes the tag: {:?}",
        note.tags
    );

    let mut reviewer = Reviewer::start(&mut session.col, DeckId(1)).unwrap();
    reviewer.toggle_mark().unwrap();
    drop(reviewer);
    let note = yaac::notes::get_note(&mut session.col, nid).unwrap();
    assert_eq!(note.tags, ["marked"]);
    let reviewer = Reviewer::start(&mut session.col, DeckId(1)).unwrap();
    assert!(
        reviewer.current.as_ref().unwrap().marked,
        "the mark is read from the note's tags"
    );
    drop(reviewer);
    session.close().unwrap();
}

#[test]
fn long_text_wraps_at_a_readable_width_on_wide_terminals() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let words: Vec<String> = (1..=80).map(|i| format!("word{i}")).collect();
    add_basic(&path, &words.join(" "), "x");
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let reviewer = Reviewer::start(&mut session.col, DeckId(1)).unwrap();
    let mut terminal = Terminal::new(TestBackend::new(240, 24)).unwrap();
    let mut images = Images::disabled(dir.path().join("collection.media"));

    terminal
        .draw(|frame| review::draw(frame, &reviewer, &mut images, None))
        .unwrap();
    let screen = rows(terminal.backend().buffer());
    let text_rows: Vec<&String> = screen[1..22]
        .iter()
        .filter(|line| line.contains("word"))
        .collect();
    assert!(
        text_rows.len() >= 3,
        "wrapped into several rows: {text_rows:?}"
    );
    for line in text_rows {
        let start = line.find("word").unwrap();
        let end = line.trim_end().len();
        // 2 columns of margin, then the 120-column cap centered in the remaining 236.
        assert!(
            start >= 60 && end <= 180,
            "kept to a centered column, got {start}..{end}: {line}"
        );
    }
}
