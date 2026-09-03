# yaac - Yet Another Anki CLI

A terminal Anki client in Rust. It embeds Anki's own Rust backend (rslib), so search, scheduling, card generation, and AnkiWeb sync are Anki's, not reimplementations.

## Requirements

- Nix with flakes and direnv. `direnv allow` drops you into a shell with the Rust toolchain and `protoc`.
- Anki desktop must be closed while yaac runs. Anki's backend takes an exclusive lock on the collection, in both directions.

## Usage

```
yaac info                       # location, counts, cards due today
yaac info --json
yaac --collection PATH info     # explicit collection.anki2, also via YAAC_COLLECTION
```

Without `--collection`, yaac looks for exactly one profile folder under the Anki data directory (`~/Library/Application Support/Anki2` on macOS, `~/.local/share/Anki2` on Linux, or `$ANKI_BASE`) and refuses to guess when there are several.

Exit codes: 0 ok, 1 error, 2 usage, 3 collection locked.

## Development

```
cargo build --release
cargo test
```

Notes:

- `anki` (rslib) is a git dependency pinned to the tag of the installed Anki desktop version. Bump the tag when upgrading Anki and expect API changes.
- `tokio` is a direct dependency only to enable its `io-util` feature; rslib relies on feature unification from Anki's workspace and does not compile without it.
- The first build clones the Anki repository and compiles rslib, which takes a few minutes.

## License

AGPL-3.0-or-later, because rslib is AGPL.
