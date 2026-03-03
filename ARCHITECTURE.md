# ARCHITECTURE.md

## High-level architecture

`stews` is currently implemented as a Rust terminal app (`ratatui` + `crossterm`) split logically into three layers inside `src/main.rs`:

1. **Domain state + actions** (`App`, `FocusArea`, transition methods)
   - Pure-ish logic for focus, cursor movement, dirty handling, and guarded quit action
2. **Input mapping** (`handle_key`)
   - Terminal key events mapped into app actions
3. **Rendering** (`render`)
   - Draws panes + status/help/prompt based on current state

This keeps behavior ready for extraction into testable modules as the codebase grows.

## Current module map

- `main` — entrypoint + `--help`
- `run_tui` — terminal lifecycle + event loop
- `handle_key` — key-to-command mapping
- `render` — pane and bar layout

## Data model (current)

- `FocusArea`: FILE TREE, JSON, INSPECTOR
- `App`:
  - focus ordering and active focus index
  - pane cursors (tree/json/inspector)
  - file list + current file
  - dirty flag
  - pending action + prompt message
  - running flag

## Key flows

- **Quit request**
  - If clean: stop loop
  - If dirty: open confirmation prompt, await save/discard/cancel placeholder handling

- **Focus request**
  - `Tab`/`Shift+Tab` rotates active pane

- **Navigation request**
  - Arrow keys update pane-local cursor placeholders

## Planned evolution

### Near-term
- Extract command system (`enum Command` + dispatcher)
- Replace placeholder file list with filesystem-backed tree model
- Add editor buffer abstraction and JSON parser integration

### Mid-term
- JSON AST model + structural operations
- Validation subsystem (parser + schema)
- Persistent config + keymap profiles
