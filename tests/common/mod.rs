#![allow(dead_code)]

use std::path::{Path, PathBuf};

use anki::collection::CollectionBuilder;
use anki::prelude::I18n;
use assert_cmd::Command;

/// A brand-new collection with Anki's stock notetypes, closed again so the binary can open
/// it. English translations so the notetypes get their real names ("Basic", "Cloze").
pub fn fresh_collection(dir: &Path) -> PathBuf {
    let path = dir.join("collection.anki2");
    CollectionBuilder::new(&path)
        .set_tr(I18n::new(&["en"]))
        .build()
        .expect("create collection")
        .close(None)
        .expect("close collection");
    path
}

/// The binary with every environment override cleared, so tests only see what they set.
pub fn yaac() -> Command {
    let mut cmd = Command::cargo_bin("yaac").expect("binary built");
    cmd.env_remove("YAAC_COLLECTION")
        .env_remove("YAAC_CONFIG")
        .env_remove("ANKI_BASE")
        .env_remove("VISUAL")
        .env_remove("EDITOR")
        .env("XDG_CONFIG_HOME", "/nonexistent");
    cmd
}

/// The binary pointed at `collection`.
pub fn yaac_on(collection: &Path) -> Command {
    let mut cmd = yaac();
    cmd.arg("--collection").arg(collection);
    cmd
}

pub fn json(output: &[u8]) -> serde_json::Value {
    serde_json::from_slice(output).expect("valid JSON output")
}

/// Adds a Basic note and returns its id.
pub fn add_basic(collection: &Path, front: &str, back: &str) -> i64 {
    let output = yaac_on(collection)
        .args(["add", "--notetype", "Basic", "--deck", "Default", "--json"])
        .arg(format!("Front={front}"))
        .arg(format!("Back={back}"))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    json(&output)[0]["id"].as_i64().expect("note id")
}
