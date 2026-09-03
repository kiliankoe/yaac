# yaac - Yet Another Anki CLI

A terminal Anki client in Rust. It embeds Anki's own Rust backend (rslib), so search, scheduling, card generation, and AnkiWeb sync are Anki's, not reimplementations.

## Requirements

- Nix with flakes and direnv. `direnv allow` drops you into a shell with the Rust toolchain and `protoc`.
- Anki desktop must be closed while yaac runs. Anki's backend takes an exclusive lock on the collection, in both directions.

## Usage

```
yaac info                                  # location, counts, cards due today
yaac decks                                 # decks with today's due counts
yaac notetypes                             # notetypes with fields and card templates

yaac search deck:Spanish is:due            # Anki search syntax, words are joined
yaac search tag:todo --ids                 # only ids, one per line, for piping
yaac show NOTE_ID...                       # all fields and cards; "-" reads ids from stdin

yaac add -n Basic -d Spanish -t vocab Front="el gato/la gata" Back="cat"
yaac add -n Basic -d Default "bare values" "in field order"
yaac add --from-json notes.json            # one object or an array; "-" for stdin
yaac edit NOTE_ID Back="better answer"     # unmentioned fields keep their content
yaac tag add "todo review" NOTE_ID...      # or: yaac tag remove TAGS NOTE_ID...
yaac delete NOTE_ID... [--yes]             # lists the notes, then asks; --yes when piped
```

Every command accepts `--json` for machine-readable output. `--collection PATH` (or `YAAC_COLLECTION`) picks a specific `collection.anki2`; otherwise the config value is used, and failing that the single profile folder under the Anki data directory (`~/Library/Application Support/Anki2` on macOS, `~/.local/share/Anki2` on Linux, or `$ANKI_BASE`). yaac refuses to guess when there are several profiles and never creates a collection.

Field values are stored as HTML, exactly as given. `add` runs Anki's own checks: an empty first field, a duplicate first field (override with `--allow-duplicate`), and cloze markers that do not match the notetype are rejected. Decks are never created implicitly.

Ids for `show`, `tag`, and `delete` can be passed as arguments or read from stdin with `-`, so `yaac search ... --ids | yaac tag add later -` works.

After any change yaac takes a backup into the profile's `backups/` folder, following the collection's own backup settings, the same way the desktop does on exit.

Exit codes: 0 ok, 1 error, 2 usage, 3 collection locked.

### JSON

Notes are objects with `id`, `guid`, `notetype`, `deck`, `tags`, `modified`, `sort_field`, `fields` (a name-to-HTML map in notetype order), and `cards` (each with `id`, `template`, `deck`, `queue`, `due_in_days`, `interval_days`, `reps`, `lapses`, `flag`). `search` and `add` print an array of them; `show` and `edit` too.

`add --from-json` reads the same shape it prints, reduced to what matters:

```json
[
  {"fields": {"Front": "el gato/la gata", "Back": "cat"}, "tags": ["vocab"]},
  {"notetype": "Cloze", "deck": "Spanish", "fields": {"Text": "{{c1::el gato}} means cat"}}
]
```

Missing `notetype`, `deck`, or `tags` fall back to the flags, then to the config.

### Config

`$XDG_CONFIG_HOME/yaac/config.toml` (or `~/.config/yaac/config.toml`, or the file named by `YAAC_CONFIG`):

```toml
collection = "/Users/me/Library/Application Support/Anki2/User 1/collection.anki2"
default_notetype = "Basic"
default_deck = "Inbox"
```

All keys are optional.

## Development

```
cargo build --release
cargo test
```

Notes:

- `anki` (rslib) is a git dependency pinned to the tag of the installed Anki desktop version. Bump the tag when upgrading Anki and expect API changes.
- `tokio` is a direct dependency only to enable its `io-util` feature; rslib relies on feature unification from Anki's workspace and does not compile without it.
- The first build clones the Anki repository and compiles rslib, which takes a few minutes.
- Tests never touch a real profile: they create throwaway collections through rslib and drive the built binary.

## License

AGPL-3.0-or-later, because rslib is AGPL.
