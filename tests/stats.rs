mod common;

use std::path::Path;

use anki::decks::DeckId;
use anki::scheduler::answering::Rating;
use common::{add_basic, fresh_collection, json, yaac_on};
use predicates::prelude::*;
use yaac::config::Config;
use yaac::review::Reviewer;
use yaac::session::Session;

/// Answers the next due cards of the default deck with `ratings`, one each.
fn study(path: &Path, ratings: &[Rating]) {
    let mut session = Session::open(Some(path), &Config::default()).unwrap();
    let mut reviewer = Reviewer::start(&mut session.col, DeckId(1)).unwrap();
    for rating in ratings {
        assert!(!reviewer.done(), "ran out of cards");
        reviewer.reveal();
        reviewer.answer(*rating).unwrap();
    }
    drop(reviewer);
    session.close().unwrap();
}

fn stats_json(path: &Path, args: &[&str]) -> serde_json::Value {
    let output = yaac_on(path)
        .arg("stats")
        .args(args)
        .arg("--json")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    json(&output)
}

#[test]
fn stats_of_a_fresh_collection_are_all_zero() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());

    let stats = stats_json(&path, &[]);
    assert_eq!(stats["search"], "");
    assert_eq!(stats["all_history"], false);
    assert_eq!(stats["fsrs"], false);
    assert_eq!(stats["today"]["cards"], 0);
    assert_eq!(stats["card_counts"]["new"], 0);
    assert_eq!(stats["reviews"]["month"]["count"], 0);
    assert_eq!(stats["reviews"]["month"]["period_days"], 31);
    assert_eq!(stats["reviews"]["year"]["period_days"], 365);
    assert!(
        stats["reviews"]["all_time"].is_null(),
        "all time only with --all"
    );
    assert!(stats["retention"]["all_time"].is_null());
    assert_eq!(stats["calendar"]["days"].as_object().unwrap().len(), 0);
    assert_eq!(stats["calendar"]["current_streak"], 0);
    assert_eq!(stats["calendar"]["longest_streak"], 0);
    assert_eq!(
        stats["future_due"]["due_by_day"].as_array().unwrap().len(),
        31
    );
    assert_eq!(stats["hours"].as_array().unwrap().len(), 24);
    assert!(stats["ease"].is_object(), "ease is shown while FSRS is off");
    assert!(stats["difficulty"].is_null());

    yaac_on(&path)
        .arg("stats")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("Collection, last 12 months\n"))
        .stdout(predicate::str::contains(
            "No cards have been studied today.",
        ))
        .stdout(predicate::str::contains("Current streak  0 days"))
        .stdout(predicate::str::contains("Total             0"));
}

#[test]
fn todays_reviews_show_up_in_every_section() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    add_basic(&path, "uno", "one");
    add_basic(&path, "dos", "two");
    add_basic(&path, "tres", "three");
    study(&path, &[Rating::Good, Rating::Again, Rating::Easy]);

    let stats = stats_json(&path, &[]);
    assert_eq!(stats["today"]["cards"], 3);
    assert_eq!(stats["today"]["correct"], 2);
    assert_eq!(stats["today"]["learn"], 3);
    assert_eq!(stats["reviews"]["month"]["count"], 3);
    assert_eq!(stats["reviews"]["month"]["days_studied"], 1);
    assert_eq!(stats["reviews"]["year"]["count"], 3);
    assert_eq!(
        stats["reviews"]["by_day"]
            .as_array()
            .unwrap()
            .last()
            .unwrap(),
        3,
        "today is the last entry"
    );
    let today = stats["calendar"]["today"].as_str().unwrap();
    let days = stats["calendar"]["days"].as_object().unwrap();
    assert_eq!(days.len(), 1);
    assert_eq!(days[today], 3);
    assert_eq!(stats["calendar"]["current_streak"], 1);
    assert_eq!(stats["calendar"]["longest_streak"], 1);
    assert_eq!(
        stats["buttons"]["learning"],
        serde_json::json!([1, 0, 1, 1])
    );
    let hourly: u64 = stats["hours"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hour| hour["reviews"].as_u64().unwrap())
        .sum();
    assert_eq!(hourly, 3);
    // Easy graduates a new card straight to review; Good and Again keep theirs in learning.
    assert_eq!(stats["card_counts"]["new"], 0);
    assert_eq!(stats["card_counts"]["learning"], 2);
    assert_eq!(stats["card_counts"]["young"], 1);
    assert_eq!(stats["added"]["month"]["cards"], 3);
    assert_eq!(
        stats["added"]["by_day"].as_array().unwrap().last().unwrap(),
        3
    );

    yaac_on(&path)
        .arg("stats")
        .assert()
        .success()
        .stdout(predicate::str::contains("Studied 3 cards in"))
        .stdout(predicate::str::contains("Again count: 1 (33.3%)"))
        .stdout(predicate::str::contains(
            "Learn: 3, Review: 0, Relearn: 0, Filtered: 0",
        ))
        .stdout(predicate::str::contains("Current streak  1 day\n"))
        .stdout(predicate::str::contains("Days studied"))
        .stdout(predicate::str::contains("1 of 31 (3%)"))
        .stdout(predicate::str::contains("Learning          2"));
}

#[test]
fn a_query_limits_the_stats_and_all_adds_the_all_time_columns() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    add_basic(&path, "uno", "one");
    add_basic(&path, "dos", "two");

    let stats = stats_json(&path, &["front:uno"]);
    assert_eq!(stats["search"], "front:uno");
    assert_eq!(stats["card_counts"]["new"], 1);

    let stats = stats_json(&path, &["--all"]);
    assert_eq!(stats["all_history"], true);
    assert_eq!(stats["reviews"]["all_time"]["count"], 0);
    assert!(stats["retention"]["all_time"].is_object());
    assert_eq!(stats["added"]["all_time"]["cards"], 2);

    yaac_on(&path)
        .args(["stats", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("Collection, all history\n"))
        .stdout(predicate::str::contains("All time"));

    yaac_on(&path)
        .args(["stats", "is:"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("computing statistics"));
}
