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

yaac review [DECK]                         # review in the terminal; picks a deck when none is given

yaac login [USERNAME]                      # asks for the password; stores AnkiWeb's session key
yaac sync                                  # collection and media; asks if a full sync is needed
yaac sync --full-upload | --full-download  # pick a side explicitly; --yes skips the question
yaac logout
```

Every command accepts `--json` for machine-readable output. `--collection PATH` (or `YAAC_COLLECTION`) picks a specific `collection.anki2`; otherwise the config value is used, and failing that the single profile folder under the Anki data directory (`~/Library/Application Support/Anki2` on macOS, `~/.local/share/Anki2` on Linux, or `$ANKI_BASE`). yaac refuses to guess when there are several profiles and never creates a collection.

Field values are stored as HTML, exactly as given. `add` runs Anki's own checks: an empty first field, a duplicate first field (override with `--allow-duplicate`), and cloze markers that do not match the notetype are rejected. Decks are never created implicitly.

Ids for `show`, `tag`, and `delete` can be passed as arguments or read from stdin with `-`, so `yaac search ... --ids | yaac tag add later -` works.

After any change yaac takes a backup into the profile's `backups/` folder, following the collection's own backup settings, the same way the desktop does on exit.

### Review

`yaac review` opens a deck picker with today's counts, or goes straight into a deck named on the command line. In the picker, `/` filters decks by name, `s` runs a normal and media sync without leaving the screen (a full sync is left to `yaac sync`, where the direction is confirmed), enter starts reviewing, and `q` quits. The review screen shows the deck and remaining new, learning, and review counts at the top, the card centered, and the keys at the bottom:

| Key          | Action                                                                  |
| ------------ | ----------------------------------------------------------------------- |
| space, enter | show the answer                                                         |
| 1 2 3 4      | Again, Hard, Good, Easy, labelled with the interval Anki would schedule |
| u            | undo the last answer or change                                          |
| s            | suspend the card                                                        |
| b            | bury the card until tomorrow                                            |
| f            | cycle the card's flag colour                                            |
| esc          | back to the deck list, with refreshed counts                            |
| q            | quit; the session summary is printed afterwards                         |

Scheduling is done by Anki's own backend, so intervals, learning steps, daily limits, and sibling burying match the desktop, and the review log syncs like any other. Cards are rendered as text: formatting, lists, and cloze deletions are kept, images and audio appear as labels until image support lands.

### Sync

`login` exchanges your AnkiWeb credentials for a session key and stores only the key, in `~/.local/share/yaac/auth.toml` (or `$XDG_DATA_HOME/yaac/auth.toml`, or `$YAAC_AUTH`), readable by you alone. When stdin is not a terminal the password is read from it, so scripts never put it on a command line. `--endpoint URL` or `sync_endpoint` in the config points at a self-hosted sync server.

`sync` runs a normal sync, then a media sync. When the collections have diverged (first sync of a new profile, or after a schema change) Anki requires a full sync in one direction. yaac asks which side wins on a terminal and refuses when piped unless `--full-upload` or `--full-download` is given, together with `--yes`. A forced backup is taken before a full download.

With `auto_sync = true` in the config, every command that changed something syncs before it exits. A failed auto sync is a warning, not an error; the change stays local and the next sync picks it up.

Exit codes: 0 ok, 1 error, 2 usage, 3 collection locked.

### JSON

Notes are objects with `id`, `guid`, `notetype`, `deck`, `tags`, `modified`, `sort_field`, `fields` (a name-to-HTML map in notetype order), and `cards` (each with `id`, `template`, `deck`, `queue`, `due_in_days`, `interval_days`, `reps`, `lapses`, `flag`). `search` and `add` print an array of them; `show` and `edit` too.

`add --from-json` reads the same shape it prints, reduced to what matters:

```json
[
  {
    "fields": { "Front": "el gato/la gata", "Back": "cat" },
    "tags": ["vocab"]
  },
  {
    "notetype": "Cloze",
    "deck": "Spanish",
    "fields": { "Text": "{{c1::el gato}} means cat" }
  }
]
```

Missing `notetype`, `deck`, or `tags` fall back to the flags, then to the config.

### Config

`$XDG_CONFIG_HOME/yaac/config.toml` (or `~/.config/yaac/config.toml`, or the file named by `YAAC_CONFIG`):

```toml
collection = "/Users/me/Library/Application Support/Anki2/User 1/collection.anki2"
default_notetype = "Basic"
default_deck = "Inbox"
auto_sync = false
sync_endpoint = "https://sync.example.org/"   # only for self-hosted servers
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
- rslib has no TLS backend unless a feature asks for one, and without it every HTTPS request fails with a bare network error. yaac enables its `rustls` feature, which needs no system libraries.
- The first build clones the Anki repository and compiles rslib, which takes a few minutes.
- Tests never touch a real profile: they create throwaway collections through rslib and drive the built binary.
- To see how a card comes out of the HTML and CSS converter without opening the TUI, `cargo run --example render_card -- PATH/collection.anki2 CARD_ID` prints both sides line by line with alignment and styles. Card ids are in `yaac show NOTE_ID --json`.

## License

AGPL-3.0-or-later, because rslib is AGPL.
