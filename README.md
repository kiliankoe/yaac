# yaac - Yet Another Anki CLI

A terminal Anki client, written in Rust, because that allows embedding Anki's own Rust backend (rslib). Search, scheduling, card generation, AnkiWeb sync, ..., it's all Anki's own stuff, this is just a different frontend.

The main motivation for creating this was that I wanted something that opens quicker, doesn't bring me out of the terminal and is easy to script against.

There's quite a few alternatives to this, I tried several, but nothing quite matched what I was looking for. Please do have a look around if there's a better fit for your needs though, many people have spent a lot of time interfacing with Anki and there might be a great fit for your specific use-case elsewhere.

Full disclosure: This was mostly implemented with the help of agentic coding tools. It is very much what I wanted to have for my needs and it wouldn't exist otherwise. I'm sorry if that's a dealbreaker for you. This readme however is not written by AI, maybe that helps at least.

## Installation

### Nix

```nix
nix run github:kiliankoe/yaac -- info
```

Just add the flake input and do what you need - you're using Nix, you know how your setup works.

```nix
inputs.yaac.url = "github:kiliankoe/yaac"
```

The repo provides a devshell as well if you want to contribute.

### Homebrew

```sh
brew tap kiliankoe/yaac https://github.com/kiliankoe/yaac
brew install kiliankoe/yaac/yaac
```

### From Source

```sh
git clone https://github.com/kiliankoe/yaac
cd yaac
cargo build --release
./target/release/yaac --help
```

The build needs Rust 1.86 or newer and `protoc`.

## Usage

Anki has to be closed while yaac runs.

```sh
# Open the deck list and review mode TUI
yaac

# Browse notes, can also be opened from deck liste via `b`
yaac browse

yaac stats

# Search uses the same syntax as Anki
yaac search deck:Spanish is:due
# output only note ids, one per line, for piping
yaac search tag:todo --ids
# all note details, can also read from stdin
yaac show [note-id]

# Managing notes
yaac add -n Basic -d Spanish -t vocab Front="el gato/la gata" Back="cat"
yaac add -n Basic -d Spanish "bare values" "in field order"
yaac edit [note-id] Back="better answer"
# opens in $EDITOR
yaac edit [note-id] --editor
yaac tag add "todo review" [note-id]
yaac delete [note-id]

# piping composes nicely with `-` to read from stdin
yaac search [query] --ids | yaac tag add later -

yaac login [username]
yaac sync
```

The help output is extensive and should help you navigate the CLI. The TUI shows available commands in the status line at the bottom or everything when pressing `?`.

Every command accepts `--json` for machine-readable output.

### Review

As mentioned above, use `yaac`/`yaac review` to open the review TUI and use it like normal Anki. It schedules the same as Anki's own backend, intervals, learning steps, daily limits, everything matches Anki. yaac tries its best to match your notes formatting, but please open an issue if something looks off, rendering HTML/CSS on the terminal is interesting. Things like alignment, bold, italic, underline, small text, colours, lists, and cloze deletions should work. Audio doesn't, but images (SVGs as well) and LaTeX should render inline. The latter is supported in two different modes, simple formulas should render inline as unicode (`\(\alpha^2\)` becomes `α²`), more complicated stuff as an image.

Images, they work if your terminal supports it. There's a few different modes and oh boy is this complicated. See config below for how to control this.

### Sync

`login` gets a session key from AnkiWeb and stores that in `~/.local/share/yaac/auth.toml` (or `$XDG_DATA_HOME/yaac/auth.toml` or `$YAAC_AUTH`).

`sync` runs a normal sync, then a media sync. If a full sync is required, yaac asks which side wins and you'll have to pass `--full-upload` or `--full-download`.

### Config

`$XDG_CONFIG_HOME/yaac/config.toml` / `~/.config/yaac/config.toml` / `$YAAC_CONFIG`:

```toml
# does not have to be specified, but can point to a different collection if you want to
collection = "/Users/me/Library/Application Support/Anki2/User 1/collection.anki2"
default_notetype = "Basic"
default_deck = "Inbox"
auto_sync = false
# for self-hosted servers
sync_endpoint = "https://sync.example.org/"
# auto, kitty, sixel, iterm2, halfblocks, or off
images = "auto"
# ink for formulas, default follows the background
latex_colour = "#ffffff"
```

All keys are optional.

## Development

```sh
cargo build --release
cargo test
```

Notes:
- `anki` (rslib) is a git dependency that should be pinned to the tag of whatever version of Anki you have installed. Bump this when upgrading Anki and expect API changes. The flake fetches the Anki tree once more for the package build, because Anki's build scripts read `proto/`, `ftl/`, and `.version` from the workspace root and the vendored crates come without it. It's tag and hash in `flake.nix` must follow. `nix build` checks the package.
- `tokio` is a direct dependency only to enable its `io-util` feature, rslib relies on feature unification from Anki's workspace and does not compile without it.
- Tests never touch a real profile, they create throwaway collections through rslib and drive the built binary.
- To see how a card comes out of the HTML and CSS converter without opening the TUI, use `cargo run --example render_card -- [PATH/collection.anki2] [card-id]`, it prints both sides line by line. Card ids are in `yaac show [note-id] --json`.

## License

AGPL-3.0-or-later, because rslib is AGPL.
