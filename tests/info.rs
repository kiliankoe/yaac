use std::path::{Path, PathBuf};

use anki::collection::CollectionBuilder;
use assert_cmd::Command;
use predicates::prelude::*;

/// A brand-new collection with Anki's stock notetypes, closed again so the binary can open it.
fn fresh_collection(dir: &Path) -> PathBuf {
    let path = dir.join("collection.anki2");
    CollectionBuilder::new(&path)
        .build()
        .expect("create collection")
        .close(None)
        .expect("close collection");
    path
}

fn yaac() -> Command {
    let mut cmd = Command::cargo_bin("yaac").expect("binary built");
    cmd.env_remove("YAAC_COLLECTION").env_remove("ANKI_BASE");
    cmd
}

#[test]
fn info_reports_counts_of_a_fresh_collection() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());

    let output = yaac()
        .args(["--collection", path.to_str().unwrap(), "info", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let info: serde_json::Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(info["collection"], path.to_str().unwrap());
    assert_eq!(info["notes"], 0);
    assert_eq!(info["cards"], 0);
    assert_eq!(info["decks"], 1, "only the Default deck");
    assert!(
        info["notetypes"].as_u64().unwrap() >= 4,
        "stock notetypes present"
    );
    assert_eq!(info["due"]["new"], 0);
    assert!(info["backend_version"].as_str().unwrap().starts_with("26."));
}

#[test]
fn info_prints_a_human_summary_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());

    yaac()
        .args(["--collection", path.to_str().unwrap(), "info"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Notes       0"))
        .stdout(predicate::str::contains(
            "Due today   new 0, learn 0, review 0",
        ));
}

#[test]
fn info_exits_with_3_while_another_process_holds_the_collection() {
    let dir = tempfile::tempdir().unwrap();
    let path = fresh_collection(dir.path());
    // Holding the collection open here is what Anki desktop does.
    let _held = CollectionBuilder::new(&path).build().unwrap();

    yaac()
        .args(["--collection", path.to_str().unwrap(), "info"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("Anki desktop is probably running"));
}

#[test]
fn info_refuses_to_create_a_missing_collection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("typo.anki2");

    yaac()
        .args(["--collection", path.to_str().unwrap(), "info"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("collection not found"));
    assert!(
        !path.exists(),
        "must not create a collection as a side effect"
    );
}

#[test]
fn info_discovers_the_only_profile_under_anki_base() {
    let base = tempfile::tempdir().unwrap();
    let profile = base.path().join("User 1");
    std::fs::create_dir(&profile).unwrap();
    let path = fresh_collection(&profile);

    let output = yaac()
        .env("ANKI_BASE", base.path())
        .args(["info", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let info: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(info["collection"], path.to_str().unwrap());
}

#[test]
fn info_refuses_to_guess_between_several_profiles() {
    let base = tempfile::tempdir().unwrap();
    for name in ["User 1", "Work"] {
        let profile = base.path().join(name);
        std::fs::create_dir(&profile).unwrap();
        fresh_collection(&profile);
    }

    yaac()
        .env("ANKI_BASE", base.path())
        .arg("info")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("several profiles"))
        .stderr(predicate::str::contains("User 1"))
        .stderr(predicate::str::contains("Work"));
}
