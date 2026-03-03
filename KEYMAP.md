# KEYMAP.md

## Focus model

`stews` uses logical focus groups:
1. File Explorer
2. JSON Editor
3. Inspector

### Focus switching
- `Tab`: next focus group
- `Shift+Tab`: previous focus group

## Navigation and editing

- `↑` / `↓`: move selection in focused pane
- `←` / `→`:
  - Editor:
    - `←`: first switch to `KEY`, then collapse selected object/array when already on `KEY`
    - `→`: expand collapsed object/array when on `KEY`; on non-container switch back to `VALUE`
  - Inspector: collapse / expand panel
- `Enter`:
  - Start inline edit of selected target
  - In inline edit: apply changes
- `Esc`:
  - Cancel inline edit
  - Cancel prompts/add-key/type-change dialogs

### Editor-specific actions

- `a`: Add in nearest context (object => add key inline, array => append item)
- `t`: Change selected value type
- `L`: Toggle animated STEWS logo panel (bottom-left of editor)
- `N`: New JSON file (Explorer focused; opens modal dialog with full cursor editing)
- `Backspace` (plain, non-edit mode in editor):
  - **KEY** target: delete entry (object pair / array item)
  - **VALUE** target: reset only value to `null`
- `⌥+Backspace` (Option+Backspace):
  - Inline edit fields: delete previous word
  - In non-edit editor rows, same delete/reset semantics as plain `Backspace`
  - iTerm2/macOS fallback (`Ctrl+W` from Option+Backspace remap) uses the same semantics
- Copy / paste:
  - `⌘+C` or `Ctrl+Shift+C`: copy current context payload (`Ctrl+C` remains quit/interrupt)
  - `⌘+V` or `Ctrl+V` (plus bracketed paste / Shift+Insert fallback): paste current clipboard text
  - **VALUE target** copy => value text/literal
  - **KEY target** copy => row payload (`{"key": value}`) when available
  - **VALUE target** paste => smart literal parse (`object/array/number/bool/null`) else string
  - **KEY target** paste => key rename or row payload insert in current object context (duplicate-key guarded)
- `w`: Save active JSON file

## Global shortcuts

- `q`: Quit request (guarded by dirty prompt)
- `Ctrl+C`: quit request (same dirty guard; intentionally unchanged)

## Dirty prompt choices

When dirty prompt appears:
- `s`: save and continue action
- `d`: discard and continue action
- `c` or `Esc`: cancel action
