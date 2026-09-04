# yaac - Yet Another Anki CLI

A terminal Anki client in Rust. It embeds Anki's own Rust backend (rslib), so search, scheduling, card generation, and AnkiWeb sync are Anki's, not reimplementations.

## Installation

### From source

```
git clone https://github.com/kiliankoe/yaac
cd yaac
cargo build --release
./target/release/yaac --help
```

The build needs a Rust toolchain (1.86 or newer) and `protoc`, and the first one compiles Anki's backend, which takes a few minutes. With nix, `direnv allow` (or `nix develop`) provides the toolchain and `protoc`.

### With nix

The flake exposes `packages.yaac` (also `default`) and `overlays.default`:

```
nix run github:kiliankoe/yaac -- info      # try it
```

In a NixOS, nix-darwin, or home-manager configuration:

```nix
# flake input
inputs.yaac.url = "github:kiliankoe/yaac";
# then either consume the overlay …
nixpkgs.overlays = [ inputs.yaac.overlays.default ];   # → pkgs.yaac
# or reference the package directly
environment.systemPackages = [ inputs.yaac.packages.${system}.yaac ];
```

`cargo install` and a Homebrew formula are planned.

## Usage

Anki desktop must be closed while yaac runs. Anki's backend takes an exclusive lock on the collection, in both directions.

```
yaac                                       # review; the default when no command is given
yaac info                                  # location, counts, cards due today
yaac decks                                 # decks with today's due counts
yaac notetypes                             # notetypes with fields and card templates
yaac stats [QUERY] [--all]                 # today, calendar, retention, and the rest of the desktop's stats screen

yaac search deck:Spanish is:due            # Anki search syntax, words are joined
yaac search tag:todo --ids                 # only ids, one per line, for piping
yaac show NOTE_ID...                       # all fields and cards; "-" reads ids from stdin

yaac add -n Basic -d Spanish -t vocab Front="el gato/la gata" Back="cat"
yaac add -n Basic -d Default "bare values" "in field order"
yaac add --from-json notes.json            # one object or an array; "-" for stdin
yaac edit NOTE_ID Back="better answer"     # unmentioned fields keep their content
yaac edit NOTE_ID --editor                 # the note as a text file in $VISUAL or $EDITOR
yaac tag add "todo review" NOTE_ID...      # or: yaac tag remove TAGS NOTE_ID...
yaac delete NOTE_ID... [--yes]             # lists the notes, then asks; --yes when piped

yaac review [DECK]                         # review in the terminal; shows deck picker when none is given
yaac browse [QUERY]                        # search, read, and edit notes

yaac login [USERNAME]                      # asks for the password; stores AnkiWeb's session key
yaac sync                                  # collection and media; asks if a full sync is needed
yaac sync --full-upload | --full-download  # pick a side explicitly; --yes skips the question
yaac logout
```

Every command accepts `--json` for machine-readable output. `--collection PATH` (or `YAAC_COLLECTION`) picks a specific `collection.anki2`; otherwise the config value is used, and failing that the single profile folder under the Anki data directory (`~/Library/Application Support/Anki2` on macOS, `~/.local/share/Anki2` on Linux, or `$ANKI_BASE`). yaac refuses to guess when there are several profiles and never creates a collection.

Field values are stored as HTML, exactly as given. `add` runs Anki's own checks: an empty first field, a duplicate first field (override with `--allow-duplicate`), and cloze markers that do not match the notetype are rejected.

Ids for `show`, `tag`, and `delete` can be passed as arguments or read from stdin with `-`, so `yaac search ... --ids | yaac tag add later -` works.

After any change yaac takes a backup into the profile's `backups/` folder, following the collection's own backup settings, the same way the desktop does on exit.

### Review

`yaac review` (or a bare `yaac`) opens a deck picker with today's counts, or goes straight into a deck named on the command line. In the picker, `/` filters decks by name, `s` runs a normal and media sync without leaving the screen (a full sync is left to `yaac sync`, where the direction is confirmed), `a` adds a note to the highlighted deck (a notetype chooser, then the editor, see below), `A` does the same with the notetype used last (or `default_notetype` from the config) and skips the chooser, `b` opens browse on the highlighted deck's notes, enter starts reviewing, and `q` quits. The review screen shows the deck and remaining new, learning, and review counts at the top, the card centered, and the keys at the bottom:

| Key          | Action                                                                  |
| ------------ | ----------------------------------------------------------------------- |
| space, enter | show the answer, or the question again                                  |
| 1 2 3 4      | Again, Hard, Good, Easy, labelled with the interval Anki would schedule |
| u            | undo the last answer or change                                          |
| s            | suspend the card                                                        |
| b            | bury the card until tomorrow                                            |
| f            | cycle the card's flag colour                                            |
| m            | mark or unmark the note (Anki's `marked` tag)                           |
| e            | edit the note in `$EDITOR` (see below), then show the card again        |
| r            | re-transmit and redraw the card's images                                |
| esc          | back to the deck list, with refreshed counts                            |
| q            | quit; the session summary is printed afterwards                         |
| ?            | list the keys; every screen has this                                    |

Scheduling is done by Anki's own backend, so intervals, learning steps, daily limits, and sibling burying match the desktop, and the review log syncs like any other. Cards are rendered as text with the notetype's formatting where a terminal can show it: alignment, bold, italic, underline, small text, colours, lists, cloze deletions. Audio appears as a label. Text wraps at 120 columns and sits centered, so a wide terminal gets margins rather than long lines.

Images are drawn inline. yaac asks the terminal which graphics protocol it supports (Kitty, Sixel, or iTerm2) and falls back to half-block characters everywhere else, including Terminal.app and Alacritty. Kitty graphics work inside tmux when `allow-passthrough on` is set. tmux needs care there: it forwards placeholder cells without the marks that tell the terminal which part of the image a cell shows, so yaac sends those cells to the outer terminal directly whenever an image appears or moves (and on the two frames after, in case tmux dropped one), and it drops pane output that arrives faster than it can forward, so images are sent as PNG in paced bursts. `r` re-sends the current card's images if one still got lost. SVG files are rasterised with system fonts. Cards of Anki's built-in Image Occlusion notetype get their masks painted in: hidden shapes are covered on the question side and outlined on the answer side, with "hide all, guess one" respected. Set `images` in the config to `kitty`, `sixel`, `iterm2`, or `halfblocks` to skip the probe, or `off` for labels only.

### Browse and edit

`yaac browse` shows a search box, the matching notes sorted by their sort field below it, and the selected note's fields, tags, and cards under those, wrapped at 120 columns. A query on the command line runs right away; without one the search box is focused. The search runs as you type. Enter or esc leaves the box so that j/k move through the results, the arrow keys move either way, and `/` returns to the box. An empty box lists nothing; `deck:*` lists every note. Images in fields are drawn the same way as in review.

| Key                          | Action                                                         |
| ---------------------------- | -------------------------------------------------------------- |
| /                            | focus the search box; enter or esc leaves it, ctrl-u clears it |
| j/k, arrows, g/G             | move through the results; arrows also work while typing        |
| ctrl-d, ctrl-u, page down/up | scroll the note                                                |
| e                            | edit the note in `$VISUAL` or `$EDITOR`                        |
| d                            | delete the note and its cards                                  |
| u                            | undo the last edit or deletion                                 |
| r                            | re-transmit and redraw images                                  |
| esc                          | back to the deck list when opened from it, otherwise quit      |
| q                            | quit                                                           |
| ?                            | list the keys                                                  |

Editing, from here, from the review screen with `e`, or with `yaac edit NOTE_ID --editor`, writes the note to a temporary markdown file and opens the editor on it:

```
<!-- yaac: note 1693526400000 (Basic). Save and quit to apply, empty the file to abort. -->
tags: vocab animals

# Front

el gato

# Back

cat
```

Fields are HTML as Anki stores them, with `<br>` shown as a line break and turned back into `<br>` on save. Everything else (images, styling, cloze markers) stays untouched. A field whose text you did not change is written back exactly as it was, a field whose heading you delete keeps its value, and `tags:` takes space-separated tags. Emptying the file aborts. A file that does not parse, because of an unknown heading say, is reopened with the error at the top. The change goes through Anki's own update path, so cards are regenerated and `u` undoes it. `$VISUAL` is tried before `$EDITOR`, either may include arguments (`code --wait`), and `vi` is the fallback.

Adding a note with `a` in the deck picker opens the same file with empty fields; quitting the editor without typing anything aborts. On save, Anki's checks run as they do for `yaac add`: an empty first field or cloze markers that do not fit the notetype reopen the file with the problem at the top, and so does a duplicate first field, once; saving a duplicate again unchanged adds it anyway.

### Stats

`yaac stats` prints what the desktop's stats screen shows, in the same order: today's answers, cards due over the next month, a calendar heatmap of the last year with the current and longest streak, review counts and time, card counts, median interval and ease (difficulty, stability, and retrievability instead when FSRS is on), retention, the hourly breakdown, answer buttons, and cards added. Graphs become sparklines, and the tables put the desktop's default periods side by side, the last 31 days and the last 12 months. A query limits the numbers to matching cards, `yaac stats deck:Spanish`, and `--all` loads the whole history, which adds an all-time column, one calendar per year, and the all-time retention row. The counting is done by Anki's own statistics code, so the numbers agree with the desktop.

### Sync

`login` exchanges your AnkiWeb credentials for a session key and stores only the key, in `~/.local/share/yaac/auth.toml` (or `$XDG_DATA_HOME/yaac/auth.toml`, or `$YAAC_AUTH`), readable by you alone. When stdin is not a terminal the password is read from it, so scripts never put it on a command line. `--endpoint URL` or `sync_endpoint` in the config points at a self-hosted sync server.

`sync` runs a normal sync, then a media sync. When the collections have diverged (first sync of a new profile, or after a schema change) Anki requires a full sync in one direction. yaac asks which side wins on a terminal and refuses when piped unless `--full-upload` or `--full-download` is given, together with `--yes`. A forced backup is taken before a full download.

With `auto_sync = true` in the config, every command that changed something syncs before it exits. A failed auto sync is a warning, not an error; the change stays local and the next sync picks it up.

Exit codes: 0 ok, 1 error, 2 usage, 3 collection locked.

### JSON

Notes are objects with `id`, `guid`, `notetype`, `deck`, `tags`, `modified`, `sort_field`, `fields` (a name-to-HTML map in notetype order), and `cards` (each with `id`, `template`, `deck`, `queue`, `due_in_days`, `interval_days`, `reps`, `lapses`, `flag`). `search` and `add` print an array of them; `show` and `edit` too.

`stats --json` prints the numbers behind every section, the calendar as a map from date to review count.

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
images = "auto"                                # kitty, sixel, iterm2, halfblocks, or off
```

All keys are optional.

## Development

```
cargo build --release
cargo test
```

Notes:

- `anki` (rslib) is a git dependency pinned to the tag of the installed Anki desktop version. Bump the tag when upgrading Anki and expect API changes. The flake fetches the Anki tree once more for the package build, because Anki's build scripts read `proto/`, `ftl/`, and `.version` from the workspace root and the vendored crates come without it; its tag and hash in `flake.nix` must follow. `nix build` checks the package.
- `tokio` is a direct dependency only to enable its `io-util` feature; rslib relies on feature unification from Anki's workspace and does not compile without it.
- rslib has no TLS backend unless a feature asks for one, and without it every HTTPS request fails with a bare network error. yaac enables its `rustls` feature, which needs no system libraries.
- The first build clones the Anki repository and compiles rslib, which takes a few minutes.
- Tests never touch a real profile: they create throwaway collections through rslib and drive the built binary.
- To see how a card comes out of the HTML and CSS converter without opening the TUI, `cargo run --example render_card -- PATH/collection.anki2 CARD_ID` prints both sides line by line with alignment and styles. Card ids are in `yaac show NOTE_ID --json`.

## License

AGPL-3.0-or-later, because rslib is AGPL.
