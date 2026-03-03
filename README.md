# stews

`stews` is a Rust + `ratatui` JSON terminal editor with a modern UI and functional structural editing.

## What works now

- JSON files are discovered (`*.json`) and opened from the Explorer.
- Explorer supports `N` to create/open a new JSON file with dirty-state confirmation.
- New-file entry uses a centered modal dialog with caret cursor and full in-place editing (`←/→/Home/End`, insert, `Backspace`, `Delete`).
- File switching actually loads/parses/render selected file content.
- Real tree document model with stable selection path (`root.foo[0].bar`).
- Inline edit mode:
  - `Enter` starts editing selected target
  - `Esc` cancels
  - `Enter` applies
  - String values edit inside quotes by default
  - `null` starts with an empty edit buffer for fresh typing
  - Double-backspace on empty string edit switches to raw literal mode
  - Raw apply parses JSON literals (`object/array/number/bool/null`) and falls back to string
- Key/value target behavior in editor:
  - selecting a row defaults target to **VALUE**
  - first `←` switches target to **KEY** (when applicable)
  - `←` on **KEY** collapses selected object/array
  - `→` on **KEY** expands collapsed object/array; non-containers switch back to **VALUE**
- Functional add-key flow for object nodes (`a`).
- `Backspace` (plain, non-edit mode in editor) deletion semantics:
  - Row **KEY** target: delete whole object pair / remove array item
  - Row **VALUE** target: reset value to `null` (keeps key/index slot)
- `Option+Backspace` / `Ctrl+W` semantics:
  - Inline key/value edit: delete previous word
  - In non-edit editor rows, same delete/reset behavior as plain `Backspace`
- Clipboard workflows:
  - `⌘C` / `Ctrl+Shift+C` copy current context payload (`Ctrl+C` stays quit/interrupt)
  - `⌘V` / `Ctrl+V` and bracketed paste apply clipboard text reliably in iTerm2/macOS-style terminals
  - VALUE copy/paste operates on value literals
  - KEY copy/paste operates on row payloads and object-key actions with duplicate-key guard
  - Add-key / add-item action rows accept paste prefill/insert behavior
- Functional type conversion for selected value (`t`).
- Dirty-state prompt is preserved for file switch and quit.
- Inspector shows selected structure with child keys/types/counts.
- Animated neon STEWS logo panel is visible in the bottom-left of the editor panel.
- `L` toggles the logo animation on/off.
- Startup auto-loads local `.env` and `.env.local` files.
- `STEWS_JSON_ROOT` can set the default directory scanned when no file paths are passed.

## Shortcuts

- `Tab` / `Shift+Tab`: cycle focus panes
- `↑` / `↓`: move in focused pane
- `←` / `→`: in editor switch KEY/VALUE and collapse/expand containers; in inspector collapse/expand
- `Enter`: inline edit selected target
- `a`: add key on selected object
- `t`: change selected value type
- `L`: toggle animated STEWS logo panel
- `N`: create/open new JSON file (Explorer focus; modal input)
- `Backspace` (non-edit editor rows): KEY deletes row, VALUE resets to `null`
- `Option+Backspace` / `Ctrl+W`: word delete in text inputs; same row delete/reset semantics in non-edit editor rows
- `⌘C` / `Ctrl+Shift+C`: copy context payload (`Ctrl+C` remains quit)
- `⌘V` / `Ctrl+V`: paste clipboard text
- `w`: save file
- `q` or `Ctrl+C`: quit (dirty-safe prompt; unchanged terminal default)

## Run

```bash
cd /path/to/stews
cargo run
```

Installed binary usage:

```bash
stews ./examples/sample.json
```

## Environment (`.env`) support

At startup, `stews` automatically loads environment variables from the current working directory in this order:

1. `.env`
2. `.env.local`

Precedence is:

1. Existing shell environment variables (highest; never overwritten)
2. `.env.local`
3. `.env`

So `.env.local` can override values from `.env`, but neither file overrides variables already set in your shell/session.

Example `.env`:

```dotenv
# default scan root when running just `stews`
STEWS_JSON_ROOT=./examples
```

Example `.env.local` (developer-machine override):

```dotenv
STEWS_JSON_ROOT=/absolute/path/to/my/json-workspace
```

With that in place, running `stews` with no arguments scans `STEWS_JSON_ROOT`; passing explicit file paths still takes priority.

## Help

```bash
stews --help
```

## Build / test

```bash
cd /path/to/stews
cargo fmt
cargo build
cargo test
```
