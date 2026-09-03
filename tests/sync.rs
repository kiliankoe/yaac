//! End to end against rslib's own sync server, started in-process on a random port.
//! One sequential test: the server reads its users from the environment, which must
//! be set before any other thread exists.

mod common;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anki::sync::http_server::{SimpleServer, SyncServerConfig, default_ip_header};
use assert_cmd::Command;
use common::{add_basic, fresh_collection, json, yaac, yaac_on};
use predicates::prelude::*;

fn start_server(base_folder: PathBuf) -> SocketAddr {
    // SAFETY: this is the only test in the binary and no other thread is running yet.
    unsafe { std::env::set_var("SYNC_USER1", "user:pass") };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async move {
            let (addr, server) = SimpleServer::make_server(SyncServerConfig {
                host: "127.0.0.1".parse().unwrap(),
                port: 0,
                base_folder,
                ip_header: default_ip_header(),
            })
            .await
            .expect("sync server");
            tx.send(addr).unwrap();
            let _ = server.await;
        });
    });
    rx.recv().expect("server address")
}

fn collection_in(dir: &Path, name: &str) -> PathBuf {
    let folder = dir.join(name);
    std::fs::create_dir(&folder).unwrap();
    fresh_collection(&folder)
}

fn note_count(collection: &Path) -> u64 {
    let output = yaac_on(collection)
        .args(["info", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    json(&output)["notes"].as_u64().unwrap()
}

#[test]
fn syncs_between_two_collections_through_a_local_server() {
    let dir = tempfile::tempdir().unwrap();
    let server_base = dir.path().join("server");
    std::fs::create_dir(&server_base).unwrap();
    let endpoint = format!("http://{}/", start_server(server_base));
    let auth_file = dir.path().join("auth.toml");
    let a = collection_in(dir.path(), "a");
    let b = collection_in(dir.path(), "b");
    let with_auth = |mut cmd: Command| {
        cmd.env("YAAC_AUTH", &auth_file);
        cmd
    };

    with_auth(yaac_on(&a))
        .arg("sync")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("not logged in"));

    with_auth(yaac())
        .args(["login", "user", "--endpoint", &endpoint])
        .write_stdin("wrong\n")
        .assert()
        .code(1);
    assert!(!auth_file.exists(), "no credentials stored on failure");

    with_auth(yaac())
        .args(["login", "user", "--endpoint", &endpoint])
        .write_stdin("pass\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("logged in as user"));
    let stored = std::fs::read_to_string(&auth_file).unwrap();
    assert!(stored.contains("hkey"), "session key stored");
    assert!(!stored.contains("pass"), "password never stored");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&auth_file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // The first real sync of a new account must pick a side, as on AnkiWeb.
    add_basic(&a, "hola", "hello");
    with_auth(yaac_on(&a))
        .arg("sync")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--full-upload"));
    with_auth(yaac_on(&a))
        .args(["sync", "--full-upload"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--yes"));
    let output = with_auth(yaac_on(&a))
        .args(["sync", "--full-upload", "--yes", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(json(&output)["collection"], "uploaded");
    assert_eq!(json(&output)["media"], "synced");

    add_basic(&a, "hasta luego", "see you");
    let output = with_auth(yaac_on(&a))
        .args(["sync", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(json(&output)["collection"], "synced");
    with_auth(yaac_on(&a))
        .arg("sync")
        .assert()
        .success()
        .stdout(predicate::str::contains("already up to date"));

    // A collection created independently has diverged: Anki demands a full sync.
    with_auth(yaac_on(&b))
        .arg("sync")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--full-upload"));
    with_auth(yaac_on(&b))
        .args(["sync", "--full-download"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--yes"));
    let output = with_auth(yaac_on(&b))
        .args(["sync", "--full-download", "--yes", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(json(&output)["collection"], "downloaded");
    assert_eq!(note_count(&b), 2, "the notes travelled through the server");
    assert!(
        std::fs::read_dir(dir.path().join("b/backups"))
            .unwrap()
            .count()
            >= 1,
        "a backup precedes the full download"
    );

    // A full upload replaces the server's collection, so the others diverge again.
    let c = collection_in(dir.path(), "c");
    add_basic(&c, "gato", "cat");
    add_basic(&c, "perro", "dog");
    let output = with_auth(yaac_on(&c))
        .args(["sync", "--full-upload", "--yes", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(json(&output)["collection"], "uploaded");
    with_auth(yaac_on(&a))
        .arg("sync")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--full-download"));
    with_auth(yaac_on(&a))
        .args(["sync", "--full-download", "--yes"])
        .assert()
        .success();
    assert_eq!(
        note_count(&a),
        2,
        "the upload replaced the server's collection"
    );

    // auto_sync: a change made through one collection reaches another on its next sync.
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        format!("collection = {:?}\nauto_sync = true\n", a.to_str().unwrap()),
    )
    .unwrap();
    with_auth(yaac())
        .env("YAAC_CONFIG", &config)
        .args(["add", "-n", "Basic", "-d", "Default", "adios", "bye"])
        .assert()
        .success();
    let output = with_auth(yaac_on(&c))
        .args(["sync", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(json(&output)["collection"], "synced");
    assert_eq!(note_count(&c), 3);

    with_auth(yaac())
        .arg("logout")
        .assert()
        .success()
        .stdout(predicate::str::contains("removed"));
    with_auth(yaac_on(&a))
        .arg("sync")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("not logged in"));
}
