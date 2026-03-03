# UX_FLOWS.md

## 1) Quit flow (with dirty prompt)

### Trigger
- User presses `q` or `Ctrl+C`.

### If buffer is clean
1. Exit immediately.

### If buffer is dirty
1. Show prompt: `Save before quit? [s]/[d]/[c]`
2. Handle choice:
   - `s`: save placeholder + exit
   - `d`: discard + exit
   - `c`: return to app

## 2) Focus flow

- `Tab` rotates focus forward among panes.
- `Shift+Tab` rotates focus backward.
- Arrow behavior is scoped by active focus area.

## 3) Navigation scaffold flow

- File tree: up/down changes selected file index.
- JSON pane: up/down/left/right changes cursor row/column placeholder.
- Inspector: up/down changes inspector cursor placeholder.

## 4) Save flow (planned expansion)

Current slice has prompt placeholder semantics; future implementation:
1. Validate JSON parse state.
2. Attempt atomic write to temp file + rename.
3. If successful: clear dirty, notify in status bar.
4. If failed: keep dirty, show actionable error.

## 5) Planned file switch flow

Future command palette (`Ctrl+P`) behavior:
- Clean buffer: switch immediately.
- Dirty buffer: same guarded prompt pattern as quit.
