# ROADMAP.md

## Milestone 0 — Skeleton (current)
- Rust Cargo project baseline
- `ratatui` + `crossterm` multi-pane layout scaffold
- Focus cycling (`Tab` / `Shift+Tab`)
- Arrow navigation scaffold
- Dirty-aware quit prompt placeholder
- Initial docs and architecture alignment

## Milestone 1 — Navigation alpha
- Filesystem-driven tree
- Expand/collapse arrows on hierarchical nodes
- Real quick file switcher palette
- Stable focus memory per pane

## Milestone 2 — Editing alpha
- JSON parse + format pipeline
- Structural cursor model
- Basic edits (key/value add/edit/delete)
- Undo/redo groundwork

## Milestone 3 — Validation beta
- Parse diagnostics list
- Inspector enrichment
- Optional JSON schema validation
- Jump-to-error UX

## Milestone 4 — Workflow beta
- Save/Save As atomic writes
- Unsaved-state flows hardened
- Recovery behavior
- Keymap configurability

## Milestone 5 — Release candidate
- Performance tuning for large JSON
- Theme and polish
- Documentation and examples
- Packaging/distribution
