mod common;

use common::{add_basic, fresh_collection, json, yaac, yaac_on};
use predicates::prelude::*;

#[test]
fn add_creates_a_note_with_one_card_and_show_prints_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());

    let output = yaac_on(&path)
        .args([
            "add",
            "--notetype",
            "Basic",
            "--deck",
            "Default",
            "--tag",
            "geo",
            "--tag",
            "capitals",
            "--json",
            "Front=Capital of France?",
            "Back=Paris",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let added = json(&output);
    let id = added[0]["id"].as_i64().unwrap();
    assert_eq!(added[0]["notetype"], "Basic");
    assert_eq!(added[0]["deck"], "Default");
    assert_eq!(added[0]["tags"], serde_json::json!(["capitals", "geo"]));

    let output = yaac_on(&path)
        .args(["show", &id.to_string(), "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let shown = json(&output);
    assert_eq!(shown[0]["fields"]["Front"], "Capital of France?");
    assert_eq!(shown[0]["fields"]["Back"], "Paris");
    assert_eq!(shown[0]["sort_field"], "Capital of France?");
    assert_eq!(shown[0]["cards"].as_array().unwrap().len(), 1);
    assert_eq!(shown[0]["cards"][0]["queue"], "new");
    assert_eq!(shown[0]["cards"][0]["template"], "Card 1");

    yaac_on(&path)
        .args(["show", &id.to_string()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Front     Capital of France?"))
        .stdout(predicate::str::contains("tags      capitals geo"));
}

#[test]
fn add_accepts_bare_values_in_field_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());

    let output = yaac_on(&path)
        .args([
            "add", "-n", "Basic", "-d", "Default", "--json", "2+2=?", "4",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let added = json(&output);
    assert_eq!(added[0]["fields"]["Front"], "2+2=?");
    assert_eq!(added[0]["fields"]["Back"], "4");
}

#[test]
fn add_rejects_bad_notetype_deck_and_field_names() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());

    yaac_on(&path)
        .args(["add", "-n", "Nope", "-d", "Default", "Front=x"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("notetype \"Nope\" does not exist"));
    yaac_on(&path)
        .args(["add", "-n", "Basic", "-d", "Nope", "Front=x"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("deck \"Nope\" does not exist"));
    yaac_on(&path)
        .args(["add", "-n", "Basic", "-d", "Default", "Front=x", "Side=y"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("mix of NAME=VALUE"));
    yaac_on(&path)
        .args(["add", "-n", "Basic", "-d", "Default", "a", "b", "c"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("only 2 fields"));
    yaac_on(&path)
        .args(["add", "-n", "Basic", "-d", "Default"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("no field values"));
}

#[test]
fn add_runs_ankis_field_checks() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());

    yaac_on(&path)
        .args(["add", "-n", "Basic", "-d", "Default", "Back=only"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("first field is empty"));

    add_basic(&path, "same", "one");
    yaac_on(&path)
        .args(["add", "-n", "Basic", "-d", "Default", "same", "two"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("already exists"));
    yaac_on(&path)
        .args([
            "add",
            "-n",
            "Basic",
            "-d",
            "Default",
            "--allow-duplicate",
            "same",
            "two",
        ])
        .assert()
        .success();

    yaac_on(&path)
        .args(["add", "-n", "Cloze", "-d", "Default", "no markers here"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("cloze notetype but no field"));
    yaac_on(&path)
        .args(["add", "-n", "Basic", "-d", "Default", "{{c1::marker}}", "x"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("not a cloze notetype"));
}

#[test]
fn add_cloze_note_generates_a_card_per_cloze() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());

    let output = yaac_on(&path)
        .args([
            "add",
            "-n",
            "Cloze",
            "-d",
            "Default",
            "--json",
            "Text={{c1::Paris}} is the capital of {{c2::France}}",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let cards = json(&output)[0]["cards"].as_array().unwrap().clone();
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[0]["template"], "Cloze 1");
    assert_eq!(cards[1]["template"], "Cloze 2");
}

#[test]
fn add_reads_several_notes_from_json_on_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let input = serde_json::json!([
        {"fields": {"Front": "one", "Back": "1"}, "tags": ["n"]},
        {"notetype": "Cloze", "fields": {"Text": "{{c1::two}}"}, "deck": "Default"}
    ]);

    let output = yaac_on(&path)
        .args([
            "add",
            "-n",
            "Basic",
            "-d",
            "Default",
            "--from-json",
            "-",
            "--json",
        ])
        .write_stdin(input.to_string())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let added = json(&output);
    assert_eq!(added.as_array().unwrap().len(), 2);
    assert_eq!(added[0]["notetype"], "Basic");
    assert_eq!(added[0]["tags"], serde_json::json!(["n"]));
    assert_eq!(added[1]["notetype"], "Cloze");

    yaac_on(&path)
        .args(["add", "-n", "Basic", "-d", "Default", "--from-json", "-"])
        .write_stdin(r#"{"fields": {"Nope": "x"}}"#)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("has no field \"Nope\""));
}

#[test]
fn add_falls_back_to_config_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        format!(
            "collection = {:?}\ndefault_notetype = \"Basic\"\ndefault_deck = \"Default\"\n",
            path.to_str().unwrap()
        ),
    )
    .unwrap();

    yaac()
        .env("YAAC_CONFIG", &config)
        .args(["add", "hello", "world"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Basic"));

    yaac_on(&path)
        .args(["add", "hello", "world"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("no notetype given"));
}

#[test]
fn search_lists_matching_notes_and_can_print_only_ids() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let first = add_basic(&path, "apple", "fruit");
    let second = add_basic(&path, "carrot", "vegetable");

    let output = yaac_on(&path)
        .args(["search", "back:fruit", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let found = json(&output);
    assert_eq!(found.as_array().unwrap().len(), 1);
    assert_eq!(found[0]["id"], first);

    yaac_on(&path)
        .args(["search", "deck:Default", "--ids"])
        .assert()
        .success()
        .stdout(format!("{first}\n{second}\n"));

    yaac_on(&path)
        .args(["search", "deck:Default", "--limit", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("apple"))
        .stdout(predicate::str::contains("carrot").not());

    yaac_on(&path)
        .args(["search", "prop:nonsense"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("searching"));
}

#[test]
fn edit_replaces_only_the_given_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let id = add_basic(&path, "question", "answer");

    let output = yaac_on(&path)
        .args(["edit", &id.to_string(), "Back=better answer", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let edited = json(&output);
    assert_eq!(edited[0]["fields"]["Front"], "question");
    assert_eq!(edited[0]["fields"]["Back"], "better answer");

    yaac_on(&path)
        .args(["edit", "12345", "Back=x"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("note 12345 does not exist"));
}

#[test]
fn tags_can_be_added_and_removed_with_ids_from_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let first = add_basic(&path, "a", "1");
    let second = add_basic(&path, "b", "2");

    yaac_on(&path)
        .args(["tag", "add", "todo review", "-"])
        .write_stdin(format!("{first}\n{second}\n"))
        .assert()
        .success()
        .stdout(predicate::str::contains("on 2 note(s)"));
    yaac_on(&path)
        .args(["search", "tag:todo", "--ids"])
        .assert()
        .success()
        .stdout(format!("{first}\n{second}\n"));

    yaac_on(&path)
        .args(["tag", "remove", "todo", &first.to_string(), "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"notes\": 1"));
    yaac_on(&path)
        .args(["search", "tag:todo", "--ids"])
        .assert()
        .success()
        .stdout(format!("{second}\n"));
}

#[test]
fn delete_needs_confirmation_and_removes_notes_with_their_cards() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let id = add_basic(&path, "gone", "soon");

    yaac_on(&path)
        .args(["delete", &id.to_string()])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--yes"))
        .stderr(predicate::str::contains("gone"));

    yaac_on(&path)
        .args(["delete", "--yes", "12345"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("note 12345 does not exist"));

    yaac_on(&path)
        .args(["delete", "--yes", &id.to_string(), "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"notes\": 1"))
        .stdout(predicate::str::contains("\"cards\": 1"));

    let output = yaac_on(&path)
        .args(["info", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(json(&output)["notes"], 0);
    assert_eq!(json(&output)["cards"], 0);
}

#[test]
fn decks_and_notetypes_are_listed() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    add_basic(&path, "x", "y");

    let output = yaac_on(&path)
        .args(["decks", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let decks = json(&output);
    assert_eq!(decks[0]["name"], "Default");
    assert_eq!(decks[0]["total"], 1);
    assert_eq!(decks[0]["new"], 1);

    let output = yaac_on(&path)
        .args(["notetypes", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let notetypes = json(&output);
    let basic = notetypes
        .as_array()
        .unwrap()
        .iter()
        .find(|nt| nt["name"] == "Basic")
        .expect("Basic listed");
    assert_eq!(basic["kind"], "normal");
    assert_eq!(basic["fields"], serde_json::json!(["Front", "Back"]));
    assert_eq!(basic["templates"], serde_json::json!(["Card 1"]));
    let cloze = notetypes
        .as_array()
        .unwrap()
        .iter()
        .find(|nt| nt["name"] == "Cloze")
        .expect("Cloze listed");
    assert_eq!(cloze["kind"], "cloze");

    yaac_on(&path)
        .arg("decks")
        .assert()
        .success()
        .stdout(predicate::str::contains("Default"));
}

#[test]
fn mutations_leave_a_backup_next_to_the_collection() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    add_basic(&path, "backup", "me");

    let backups: Vec<_> = std::fs::read_dir(dir.path().join("backups"))
        .expect("backups folder created")
        .collect();
    assert_eq!(backups.len(), 1, "one backup after the first change");
}

#[test]
fn search_help_doubles_as_a_syntax_cheat_sheet() {
    yaac()
        .args(["search", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("is:due"))
        .stdout(predicate::str::contains("prop:ivl>=30"))
        .stdout(predicate::str::contains("rated:30:1"))
        .stdout(predicate::str::contains("docs.ankiweb.net/searching"));
}

#[test]
fn edit_with_the_editor_round_trips_the_note_through_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    let id = add_basic(&path, "question", "answer<br>two");
    // A stand-in editor that rewrites the file the way a person would. Its first run
    // breaks a heading, so the second run sees the error yaac put at the top.
    let script = dir.path().join("editor.sh");
    std::fs::write(
        &script,
        r#"set -e
f="$1"
if grep -q '^<!-- yaac error:' "$f"; then
  sed 's/^# Bakc$/# Back/' "$f" > "$f.tmp"
else
  sed -e 's/^# Back$/# Bakc/' -e 's/^two$/three/' -e 's/^tags: *$/tags: edited/' "$f" > "$f.tmp"
fi
mv "$f.tmp" "$f"
"#,
    )
    .unwrap();
    let editor = format!("sh {}", script.display());

    let output = yaac_on(&path)
        .env("EDITOR", &editor)
        .args(["edit", "--editor", &id.to_string(), "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let edited = json(&output);
    assert_eq!(edited[0]["fields"]["Front"], "question");
    assert_eq!(edited[0]["fields"]["Back"], "answer<br>three");
    assert_eq!(edited[0]["tags"], serde_json::json!(["edited"]));

    std::fs::write(&script, ": > \"$1\"\n").unwrap();
    yaac_on(&path)
        .env("EDITOR", &editor)
        .args(["edit", "-e", &id.to_string()])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("aborted"));
    yaac_on(&path)
        .env("EDITOR", "false")
        .args(["edit", "-e", &id.to_string()])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("exited"));
    yaac_on(&path)
        .args(["edit", "-e", &id.to_string(), "Back=x"])
        .assert()
        .code(2);
}
