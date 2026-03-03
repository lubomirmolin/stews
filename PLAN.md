# PLAN.md

## Product vision
Build `stews` into a fast, keyboard-first JSON editor for terminal-native workflows (devops, config editing, API payload debugging) with structural editing ergonomics and safe save/quit semantics.

## Principles
1. JSON-first, not generic text-first
2. Keyboard-only flow must be delightful
3. Safe by default (dirty tracking, prompts, validation)
4. Great terminal portability (macOS/Linux, tmux, SSH)
5. Rust-first implementation (`ratatui` + `crossterm`)

## Full plan (beyond v0)

## Phase A — Foundations
- [x] Project bootstrap
- [x] Initial docs + architecture
- [x] Rust `ratatui` skeleton with stateful focus/navigation loop
- [x] Dirty prompt placeholder in TUI event flow
- [ ] Extract shared app state into dedicated modules

## Phase B — Core navigation + focus system
- [ ] Filesystem-driven tree model
- [ ] Expand/collapse nodes with arrows
- [ ] Fast file switcher palette (fuzzy)
- [ ] Focus persistence across context switches

## Phase C — JSON editing engine
- [ ] Parse + AST model with stable node ids
- [ ] Structural cursor (node/key/value)
- [ ] Insert/delete/move node operations
- [ ] String/number/boolean/null edit workflows
- [ ] Undo/redo history

## Phase D — Validation + inspector
- [ ] Inline JSON parse diagnostics
- [ ] Optional JSON Schema validation
- [ ] Inspector metadata (path, type, inferred schema)
- [ ] Jump-to-error and error list panel

## Phase E — File workflows + safety
- [ ] Dirty tracking at operation granularity
- [ ] Save / Save As / atomic write
- [ ] Robust dirty prompts on switch/quit
- [ ] Autosave and recovery journal (optional)

## Phase F — UX polish
- [ ] Theming
- [ ] Configurable keymap profiles
- [ ] Better status messaging + command line
- [ ] Help screen and discoverability

## Phase G — Packaging + distribution
- [ ] Binary packaging
- [ ] Homebrew tap (macOS)
- [ ] Linux package scripts
- [ ] Versioned release flow + changelog

## Immediate next engineering tasks
1. Split `src/main.rs` into `app`, `ui`, and `input` modules
2. Add real file scanning + file tree state model
3. Add first JSON parser integration in editor pane
4. Implement proper dirty mutation hooks from editor operations
5. Introduce CI checks (`cargo fmt`, `cargo clippy`, `cargo test`)
