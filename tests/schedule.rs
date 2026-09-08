mod common;

use std::path::Path;

use anki::decks::DeckId;
use anki::scheduler::answering::Rating;
use common::{add_basic, fresh_collection, json, yaac_on};
use predicates::prelude::*;
use yaac::config::Config;
use yaac::review::Reviewer;
use yaac::session::Session;

/// The scheduling view of a note's only card.
fn card(collection: &Path, id: i64) -> serde_json::Value {
    let output = yaac_on(collection)
        .args(["show", &id.to_string(), "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    json(&output)[0]["cards"][0].clone()
}

#[test]
fn due_turns_cards_into_reviews_and_reset_makes_them_new_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let id = add_basic(&path, "a", "1");

    yaac_on(&path)
        .args(["due", "3", &id.to_string()])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "1 card(s) of 1 note(s) due in 3 day(s)",
        ));
    let shown = card(&path, id);
    assert_eq!(shown["queue"], "review");
    assert_eq!(shown["due_in_days"], 3);
    assert_eq!(shown["interval_days"], 3);

    yaac_on(&path)
        .args(["reset", "-", "--json"])
        .write_stdin(format!("{id}\n"))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"cards\": 1"));
    let shown = card(&path, id);
    assert_eq!(shown["queue"], "new");
    assert_eq!(shown["interval_days"], 0);
    assert!(shown["due_in_days"].is_null());

    yaac_on(&path)
        .args(["due", "0", &id.to_string()])
        .assert()
        .success()
        .stdout(predicate::str::contains("due today"));
    let shown = card(&path, id);
    assert_eq!(shown["queue"], "review");
    assert_eq!(shown["due_in_days"], 0);
}

#[test]
fn reset_keeps_review_counts_unless_asked() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let id = add_basic(&path, "b", "2");
    let mut session = Session::open(Some(&path), &Config::default()).unwrap();
    let mut reviewer = Reviewer::start(&mut session.col, DeckId(1)).unwrap();
    reviewer.reveal();
    reviewer.answer(Rating::Easy).unwrap();
    drop(reviewer);
    session.close().unwrap();
    assert_eq!(card(&path, id)["queue"], "review");

    yaac_on(&path)
        .args(["reset", &id.to_string()])
        .assert()
        .success()
        .stdout(predicate::str::contains("reset 1 card(s) of 1 note(s)"));
    let shown = card(&path, id);
    assert_eq!(shown["queue"], "new");
    assert_eq!(
        shown["reps"], 1,
        "counts survive a plain reset, as in the desktop"
    );

    yaac_on(&path)
        .args(["reset", "--reset-counts", &id.to_string()])
        .assert()
        .success();
    assert_eq!(card(&path, id)["reps"], 0);
}

#[test]
fn due_and_reset_reject_bad_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let id = add_basic(&path, "c", "3");

    yaac_on(&path)
        .args(["due", "soon", &id.to_string()])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("\"soon\" is not a number of days"));
    assert_eq!(card(&path, id)["queue"], "new");

    yaac_on(&path)
        .args(["due", "0", "12345"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("note 12345 does not exist"));
    yaac_on(&path)
        .args(["reset", "12345"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("note 12345 does not exist"));
}
