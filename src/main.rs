mod logo;
mod theme;

use std::{
    collections::HashSet,
    env, fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};
use serde_json::{Map, Number, Value};

const APP_TITLE: &str = "STEWS :: UI V2";
const APP_SUBTITLE: &str = "Functional JSON Workbench";
const CARET: &str = "│";
const DOTENV_FILES: [&str; 2] = [".env", ".env.local"];
const JSON_ROOT_ENV_VAR: &str = "STEWS_JSON_ROOT";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusArea {
    Explorer,
    Editor,
    Inspector,
}

impl FocusArea {
    fn as_str(self) -> &'static str {
        match self {
            FocusArea::Explorer => "explorer",
            FocusArea::Editor => "editor",
            FocusArea::Inspector => "inspector",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditTarget {
    Key,
    Value,
}

impl EditTarget {
    fn as_str(self) -> &'static str {
        match self {
            EditTarget::Key => "KEY",
            EditTarget::Value => "VALUE",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ValueEditMode {
    QuotedString,
    RawLiteral,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum PathSegment {
    Key(String),
    Index(usize),
}

type NodePath = Vec<PathSegment>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JsonType {
    String,
    Number,
    Bool,
    Null,
    Object,
    Array,
}

impl JsonType {
    fn from_value(value: &Value) -> Self {
        match value {
            Value::String(_) => JsonType::String,
            Value::Number(_) => JsonType::Number,
            Value::Bool(_) => JsonType::Bool,
            Value::Null => JsonType::Null,
            Value::Object(_) => JsonType::Object,
            Value::Array(_) => JsonType::Array,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            JsonType::String => "string",
            JsonType::Number => "number",
            JsonType::Bool => "bool",
            JsonType::Null => "null",
            JsonType::Object => "object",
            JsonType::Array => "array",
        }
    }
}

#[derive(Clone, Debug)]
struct NodeRow {
    kind: RowKind,
    path: NodePath,
    depth: usize,
    key_label: Option<String>,
    key_editable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RowKind {
    Value,
    AddKeyAction { object_path: NodePath },
    AddItemAction { array_path: NodePath },
}

#[derive(Clone, Debug)]
enum AddContext {
    Object(NodePath),
    Array(NodePath),
}

#[derive(Debug)]
struct Document {
    path: PathBuf,
    root: Value,
    saved: Value,
}

impl Document {
    fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read JSON file {}", path.display()))?;
        let root: Value = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse JSON from {}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            saved: root.clone(),
            root,
        })
    }

    fn is_dirty(&self) -> bool {
        self.root != self.saved
    }

    fn save(&mut self) -> Result<()> {
        let data = serde_json::to_string_pretty(&self.root)?;
        fs::write(&self.path, format!("{data}\n"))
            .with_context(|| format!("failed to write {}", self.path.display()))?;
        self.saved = self.root.clone();
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct EditState {
    path: NodePath,
    target: EditTarget,
    input: String,
    cursor: usize,
    value_mode: ValueEditMode,
}

#[derive(Clone, Debug)]
struct AddKeyEditState {
    object_path: NodePath,
    stage: AddKeyStage,
    key_input: String,
    key_cursor: usize,
    value_input: String,
    value_cursor: usize,
}

impl AddKeyEditState {
    fn active_field_mut(&mut self) -> (&mut String, &mut usize) {
        match self.stage {
            AddKeyStage::Key => (&mut self.key_input, &mut self.key_cursor),
            AddKeyStage::Value => (&mut self.value_input, &mut self.value_cursor),
        }
    }
}

#[derive(Clone, Debug)]
struct SaveModalState {
    message: String,
}

#[derive(Clone, Debug)]
struct ErrorModalState {
    title: String,
    message: String,
}

#[derive(Clone, Debug)]
enum PendingAction {
    Quit,
    SwitchFile(usize),
    OpenOrCreateFile(PathBuf),
}

#[derive(Clone, Debug)]
enum AddKeyStage {
    Key,
    Value,
}

#[derive(Clone, Debug)]
enum PromptState {
    DirtyConfirm { action: PendingAction },
    ChangeType { path: NodePath },
    NewFile { input: String, cursor: usize },
}

struct ClipboardState {
    system: Option<arboard::Clipboard>,
    scratch: Option<String>,
}

impl ClipboardState {
    fn new() -> Self {
        let system = if cfg!(test) {
            None
        } else {
            arboard::Clipboard::new().ok()
        };

        Self {
            system,
            scratch: None,
        }
    }

    fn write_text(&mut self, text: String) {
        self.scratch = Some(text.clone());
        if let Some(clipboard) = &mut self.system {
            let _ = clipboard.set_text(text);
        }
    }

    fn read_text(&mut self) -> Result<String> {
        if let Some(clipboard) = &mut self.system {
            if let Ok(text) = clipboard.get_text() {
                self.scratch = Some(text.clone());
                return Ok(text);
            }
        }

        self.scratch
            .clone()
            .ok_or_else(|| anyhow!("clipboard text is unavailable"))
    }
}

impl std::fmt::Debug for ClipboardState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardState")
            .field("system_available", &self.system.is_some())
            .field("has_scratch", &self.scratch.is_some())
            .finish()
    }
}

#[derive(Debug)]
struct App {
    focus_order: [FocusArea; 3],
    focus_idx: usize,

    files: Vec<PathBuf>,
    explorer_cursor: usize,
    active_file_idx: usize,
    explorer_filter: String,

    document: Document,
    rows: Vec<NodeRow>,
    selected_row: usize,
    edit_target: EditTarget,
    edit_mode: Option<EditState>,
    add_key_mode: Option<AddKeyEditState>,
    collapsed_paths: HashSet<NodePath>,

    inspector_collapsed: bool,
    inspector_scroll: usize,

    prompt: Option<PromptState>,
    save_modal: Option<SaveModalState>,
    error_modal: Option<ErrorModalState>,
    status: String,
    running: bool,

    logo_visible: bool,
    launched_at: Instant,

    clipboard: ClipboardState,
}

impl App {
    fn bootstrap(paths: Vec<PathBuf>) -> Result<Self> {
        let files = if paths.is_empty() {
            let search_root = discover_root_path()?;
            discover_json_files(&search_root)?
        } else {
            paths
        };

        if files.is_empty() {
            bail!("No JSON files found. Pass files explicitly or add .json files.");
        }

        Self::from_files(files)
    }

    fn from_files(files: Vec<PathBuf>) -> Result<Self> {
        let first_path = files
            .first()
            .ok_or_else(|| anyhow!("at least one file is required"))?
            .clone();

        let document = Document::load(&first_path)?;

        let mut app = Self {
            focus_order: [FocusArea::Explorer, FocusArea::Editor, FocusArea::Inspector],
            focus_idx: 1,
            files,
            explorer_cursor: 0,
            active_file_idx: 0,
            explorer_filter: String::new(),
            document,
            rows: Vec::new(),
            selected_row: 0,
            edit_target: EditTarget::Value,
            edit_mode: None,
            add_key_mode: None,
            collapsed_paths: HashSet::new(),
            inspector_collapsed: false,
            inspector_scroll: 0,
            prompt: None,
            save_modal: None,
            error_modal: None,
            status: "Ready".to_string(),
            running: true,
            logo_visible: true,
            launched_at: Instant::now(),
            clipboard: ClipboardState::new(),
        };

        app.rebuild_rows(None);
        Ok(app)
    }

    fn focus(&self) -> FocusArea {
        self.focus_order[self.focus_idx]
    }

    fn set_focus(&mut self, area: FocusArea) {
        if let Some(idx) = self
            .focus_order
            .iter()
            .position(|candidate| *candidate == area)
        {
            self.focus_idx = idx;
        }
    }

    fn filtered_file_indices(&self) -> Vec<usize> {
        if self.explorer_filter.is_empty() {
            return (0..self.files.len()).collect();
        }

        let needle = self.explorer_filter.to_ascii_lowercase();
        self.files
            .iter()
            .enumerate()
            .filter_map(|(idx, file)| {
                let path_display = display_path(file).to_ascii_lowercase();
                if path_display.contains(&needle) {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    fn apply_explorer_filter(&mut self, next: String) {
        self.explorer_filter = next;

        let visible = self.filtered_file_indices();
        if visible.is_empty() {
            return;
        }

        if !visible.contains(&self.explorer_cursor) {
            let next_idx = if visible.contains(&self.active_file_idx) {
                self.active_file_idx
            } else {
                visible[0]
            };
            self.explorer_cursor = next_idx;
            self.request_file_switch(next_idx);
        }
    }

    fn append_explorer_filter_char(&mut self, ch: char) {
        let mut next = self.explorer_filter.clone();
        next.push(ch);
        self.apply_explorer_filter(next);
    }

    fn pop_explorer_filter_char(&mut self, modifiers: KeyModifiers) {
        if self.explorer_filter.is_empty() {
            return;
        }

        let mut next = self.explorer_filter.clone();
        let mut cursor = next.chars().count();
        if is_option_backspace(modifiers) || modifiers.contains(KeyModifiers::CONTROL) {
            delete_word_before_cursor(&mut next, &mut cursor);
        } else {
            delete_char_before_cursor(&mut next, &mut cursor);
        }

        self.apply_explorer_filter(next);
    }

    fn current_file_display(&self) -> String {
        display_path(&self.document.path)
    }

    fn current_row(&self) -> Option<&NodeRow> {
        self.rows.get(self.selected_row)
    }

    fn current_path(&self) -> NodePath {
        self.current_row()
            .map(|row| row.path.clone())
            .unwrap_or_default()
    }

    fn current_value(&self) -> Option<&Value> {
        value_at_path(&self.document.root, &self.current_path())
    }

    fn current_value_type(&self) -> Option<JsonType> {
        self.current_value().map(JsonType::from_value)
    }

    fn is_inline_input_active(&self) -> bool {
        self.edit_mode.is_some() || self.add_key_mode.is_some()
    }

    fn cycle_focus(&mut self, backward: bool) {
        if self.is_inline_input_active() || self.prompt.is_some() {
            return;
        }

        self.focus_idx = if backward {
            (self.focus_idx + self.focus_order.len() - 1) % self.focus_order.len()
        } else {
            (self.focus_idx + 1) % self.focus_order.len()
        };
    }

    fn move_up(&mut self) {
        if self.is_inline_input_active() || self.prompt.is_some() {
            return;
        }

        match self.focus() {
            FocusArea::Explorer => self.move_explorer_cursor(-1),
            FocusArea::Editor => self.move_editor_selection(-1),
            FocusArea::Inspector => self.inspector_scroll = self.inspector_scroll.saturating_sub(1),
        }
    }

    fn move_down(&mut self) {
        if self.is_inline_input_active() || self.prompt.is_some() {
            return;
        }

        match self.focus() {
            FocusArea::Explorer => self.move_explorer_cursor(1),
            FocusArea::Editor => self.move_editor_selection(1),
            FocusArea::Inspector => self.inspector_scroll += 1,
        }
    }

    fn move_left(&mut self) {
        if self.is_inline_input_active() || self.prompt.is_some() {
            return;
        }

        match self.focus() {
            FocusArea::Editor => {
                let Some(row) = self.current_row() else {
                    return;
                };

                if self.edit_target != EditTarget::Key && row_supports_key_target(row) {
                    self.edit_target = EditTarget::Key;
                    return;
                }

                let collapse_path = if self.edit_target == EditTarget::Key && row.key_editable {
                    parent_path(&row.path).unwrap_or_default()
                } else {
                    row.path.clone()
                };

                if self.is_container_path(&collapse_path) {
                    self.collapsed_paths.insert(collapse_path.clone());
                    self.rebuild_rows(Some(collapse_path));
                }
            }
            FocusArea::Inspector => self.inspector_collapsed = true,
            FocusArea::Explorer => {}
        }
    }

    fn move_right(&mut self) {
        if self.is_inline_input_active() || self.prompt.is_some() {
            return;
        }

        match self.focus() {
            FocusArea::Editor => {
                let Some(row) = self.current_row() else {
                    return;
                };

                let path = row.path.clone();
                if self.collapsed_paths.remove(&path) {
                    self.rebuild_rows(Some(path));
                    return;
                }

                if self.edit_target == EditTarget::Key {
                    self.edit_target = EditTarget::Value;
                }
            }
            FocusArea::Inspector => self.inspector_collapsed = false,
            FocusArea::Explorer => {}
        }
    }

    fn move_explorer_cursor(&mut self, delta: isize) {
        let visible = self.filtered_file_indices();
        if visible.is_empty() {
            return;
        }

        let current_visible_idx = visible
            .iter()
            .position(|idx| *idx == self.explorer_cursor)
            .unwrap_or(0);
        let next_visible_idx = wrapped_step(current_visible_idx, visible.len(), delta);
        let next = visible[next_visible_idx];

        if next != self.explorer_cursor {
            self.explorer_cursor = next;
            self.request_file_switch(next);
        }
    }

    fn move_editor_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }

        let next = wrapped_step(self.selected_row, self.rows.len(), delta);

        if next != self.selected_row {
            self.selected_row = next;
            self.edit_target = EditTarget::Value;
            self.inspector_scroll = 0;
        }
    }

    fn request_file_switch(&mut self, index: usize) {
        if index == self.active_file_idx {
            return;
        }

        if self.document.is_dirty() {
            self.prompt = Some(PromptState::DirtyConfirm {
                action: PendingAction::SwitchFile(index),
            });
            self.status = "Unsaved changes before switching file".to_string();
            return;
        }

        if let Err(err) = self.open_file(index) {
            // Keep explorer cursor where the user moved so they can continue
            // navigating to other files even if this one is unreadable/invalid.
            self.status = format!("File switch failed (skipped): {err}");
        }
    }

    fn request_open_or_create_file(&mut self, path: PathBuf) {
        if self.document.is_dirty() {
            self.prompt = Some(PromptState::DirtyConfirm {
                action: PendingAction::OpenOrCreateFile(path),
            });
            self.status = "Unsaved changes before opening new file".to_string();
            return;
        }

        if let Err(err) = self.open_or_create_file(path) {
            self.status = format!("New file failed: {err}");
            self.explorer_cursor = self.active_file_idx;
        }
    }

    fn resolve_new_file_path(&self, input: &str) -> Result<PathBuf> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            bail!("filename cannot be empty");
        }

        let mut path = PathBuf::from(trimmed);
        if path.extension().is_none() {
            path.set_extension("json");
        }

        if path.is_relative() {
            let base = self
                .document
                .path
                .parent()
                .ok_or_else(|| anyhow!("cannot resolve file directory"))?;
            path = base.join(path);
        }

        Ok(path)
    }

    fn open_or_create_file(&mut self, path: PathBuf) -> Result<()> {
        if !path.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::write(&path, "{}\n")
                .with_context(|| format!("failed to create {}", path.display()))?;
        }

        let index = if let Some(idx) = self.files.iter().position(|file| file == &path) {
            idx
        } else {
            self.files.push(path.clone());
            self.files.sort();
            self.files
                .iter()
                .position(|file| file == &path)
                .ok_or_else(|| anyhow!("new file index not found"))?
        };

        self.open_file(index)
    }

    fn open_file(&mut self, index: usize) -> Result<()> {
        let path = self
            .files
            .get(index)
            .ok_or_else(|| anyhow!("invalid file index {index}"))?
            .clone();

        let document = Document::load(&path)?;

        self.document = document;
        self.active_file_idx = index;
        self.explorer_cursor = index;
        self.edit_mode = None;
        self.add_key_mode = None;
        self.prompt = None;
        self.save_modal = None;
        self.error_modal = None;
        self.edit_target = EditTarget::Value;
        self.collapsed_paths.clear();
        self.inspector_scroll = 0;
        self.status = format!("Opened {}", display_path(&path));
        self.rebuild_rows(Some(Vec::new()));

        Ok(())
    }

    fn request_quit(&mut self) {
        if self.document.is_dirty() {
            self.prompt = Some(PromptState::DirtyConfirm {
                action: PendingAction::Quit,
            });
            return;
        }
        self.running = false;
    }

    fn save_current(&mut self) {
        match self.document.save() {
            Ok(()) => {
                let message = format!("Saved {}", self.current_file_display());
                self.status = message.clone();
                self.save_modal = Some(SaveModalState { message });
            }
            Err(err) => {
                self.save_modal = None;
                self.status = format!("Save failed: {err}");
            }
        }
    }

    fn handle_enter(&mut self) {
        if self.is_inline_input_active() || self.prompt.is_some() {
            return;
        }

        match self.focus() {
            FocusArea::Explorer => {
                self.set_focus(FocusArea::Editor);
                self.status = "Focus moved to JSON canvas".to_string();
            }
            FocusArea::Editor => self.begin_inline_edit(),
            FocusArea::Inspector => {}
        }
    }

    fn begin_inline_edit(&mut self) {
        if self.focus() != FocusArea::Editor || self.prompt.is_some() || self.add_key_mode.is_some()
        {
            return;
        }

        let Some(row) = self.current_row() else {
            return;
        };

        match &row.kind {
            RowKind::AddKeyAction { object_path } => {
                self.begin_add_key_for_object(object_path.clone());
                return;
            }
            RowKind::AddItemAction { array_path } => {
                self.add_item_to_array_context(array_path.clone());
                return;
            }
            RowKind::Value => {}
        }

        if self.edit_target == EditTarget::Key && !row.key_editable {
            self.status = "Selected row has no editable key".to_string();
            return;
        }

        let path = row.path.clone();

        let (input, value_mode) = match self.edit_target {
            EditTarget::Key => (
                row.key_label.clone().unwrap_or_default(),
                ValueEditMode::RawLiteral,
            ),
            EditTarget::Value => {
                let Some(value) = value_at_path(&self.document.root, &path) else {
                    self.status = "No value selected".to_string();
                    return;
                };
                value_to_edit_text(value)
            }
        };

        self.edit_mode = Some(EditState {
            path,
            target: self.edit_target,
            cursor: input.chars().count(),
            input,
            value_mode,
        });
    }

    fn begin_add_key(&mut self) {
        if self.focus() != FocusArea::Editor
            || self.prompt.is_some()
            || self.is_inline_input_active()
        {
            return;
        }

        match self.nearest_add_context() {
            Some(AddContext::Object(object_path)) => self.begin_add_key_for_object(object_path),
            Some(AddContext::Array(array_path)) => self.add_item_to_array_context(array_path),
            None => {
                self.status = "No object/array context available for add action".to_string();
            }
        }
    }

    fn begin_add_key_for_object(&mut self, object_path: NodePath) {
        if !value_at_path(&self.document.root, &object_path).is_some_and(Value::is_object) {
            self.status = "No object context available for add key".to_string();
            return;
        }

        self.collapsed_paths.remove(&object_path);
        self.rebuild_rows(Some(object_path.clone()));

        let Some(add_row_idx) = self.find_add_key_row_index(&object_path) else {
            self.status = "Cannot render add-key row for this object".to_string();
            return;
        };

        self.selected_row = add_row_idx;
        self.edit_target = EditTarget::Value;
        self.add_key_mode = Some(AddKeyEditState {
            object_path,
            stage: AddKeyStage::Key,
            key_input: String::new(),
            key_cursor: 0,
            value_input: String::new(),
            value_cursor: 0,
        });
        self.status = "Add key inline: enter key name then value".to_string();
    }

    fn find_add_key_row_index(&self, object_path: &NodePath) -> Option<usize> {
        self.rows.iter().position(|row| {
            matches!(
                &row.kind,
                RowKind::AddKeyAction {
                    object_path: candidate,
                } if candidate == object_path
            )
        })
    }

    fn find_add_item_row_index(&self, array_path: &NodePath) -> Option<usize> {
        self.rows.iter().position(|row| {
            matches!(
                &row.kind,
                RowKind::AddItemAction {
                    array_path: candidate,
                } if candidate == array_path
            )
        })
    }

    fn nearest_add_context(&self) -> Option<AddContext> {
        if let Some(row) = self.current_row() {
            match &row.kind {
                RowKind::AddKeyAction { object_path } => {
                    return Some(AddContext::Object(object_path.clone()));
                }
                RowKind::AddItemAction { array_path } => {
                    return Some(AddContext::Array(array_path.clone()));
                }
                RowKind::Value => {}
            }
        }

        let start = self.current_path();
        for len in (0..=start.len()).rev() {
            let candidate = start[..len].to_vec();
            if let Some(value) = value_at_path(&self.document.root, &candidate) {
                if value.is_object() {
                    return Some(AddContext::Object(candidate));
                }
                if value.is_array() {
                    return Some(AddContext::Array(candidate));
                }
            }
        }

        None
    }

    fn add_item_to_array_context(&mut self, array_path: NodePath) {
        self.collapsed_paths.remove(&array_path);
        self.rebuild_rows(Some(array_path.clone()));

        if let Some(add_row_idx) = self.find_add_item_row_index(&array_path) {
            self.selected_row = add_row_idx;
        }

        match self.add_item_at_path(&array_path, Value::Null) {
            Ok(path) => {
                if let Some(PathSegment::Index(idx)) = path.last() {
                    self.status = format!("Added array item [{}]", idx);
                } else {
                    self.status = "Added array item".to_string();
                }
            }
            Err(err) => {
                self.status = format!("Add item failed: {err}");
            }
        }
    }

    fn begin_change_type(&mut self) {
        if self.focus() != FocusArea::Editor
            || self.prompt.is_some()
            || self.is_inline_input_active()
        {
            return;
        }

        let path = self.current_path();
        self.prompt = Some(PromptState::ChangeType { path });
    }

    fn begin_new_file_prompt(&mut self) {
        if self.focus() != FocusArea::Explorer
            || self.prompt.is_some()
            || self.is_inline_input_active()
        {
            return;
        }

        self.prompt = Some(PromptState::NewFile {
            input: String::new(),
            cursor: 0,
        });
    }

    fn handle_explorer_filter_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        if self.focus() != FocusArea::Explorer
            || self.prompt.is_some()
            || self.is_inline_input_active()
        {
            return false;
        }

        match code {
            KeyCode::Esc if !self.explorer_filter.is_empty() => {
                self.apply_explorer_filter(String::new());
                true
            }
            KeyCode::Backspace if !self.explorer_filter.is_empty() => {
                self.pop_explorer_filter_char(modifiers);
                true
            }
            KeyCode::Char(ch)
                if !modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::META)
                    && !is_reserved_global_char(ch) =>
            {
                self.append_explorer_filter_char(ch);
                true
            }
            _ => false,
        }
    }

    fn handle_prompt_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        let Some(prompt) = self.prompt.take() else {
            return false;
        };

        match prompt {
            PromptState::DirtyConfirm { action } => {
                match code {
                    KeyCode::Esc | KeyCode::Char('c') | KeyCode::Char('C') => {
                        if matches!(
                            action,
                            PendingAction::SwitchFile(_) | PendingAction::OpenOrCreateFile(_)
                        ) {
                            self.explorer_cursor = self.active_file_idx;
                        }
                        self.status = "Action cancelled".to_string();
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => match self.document.save() {
                        Ok(()) => {
                            self.status = "Saved changes".to_string();
                            self.finish_pending_action(action);
                        }
                        Err(err) => {
                            self.status = format!("Save failed: {err}");
                            self.prompt = Some(PromptState::DirtyConfirm { action });
                        }
                    },
                    KeyCode::Char('d') | KeyCode::Char('D') => {
                        self.finish_pending_action(action);
                    }
                    _ => {
                        self.prompt = Some(PromptState::DirtyConfirm { action });
                    }
                }
                true
            }
            PromptState::ChangeType { path } => {
                match code {
                    KeyCode::Esc => {
                        self.status = "Type change cancelled".to_string();
                    }
                    KeyCode::Char(ch) => {
                        let target = match ch.to_ascii_lowercase() {
                            's' => Some(JsonType::String),
                            'n' => Some(JsonType::Number),
                            'b' => Some(JsonType::Bool),
                            'u' => Some(JsonType::Null),
                            'o' => Some(JsonType::Object),
                            'a' => Some(JsonType::Array),
                            _ => None,
                        };

                        if let Some(target_type) = target {
                            match self.change_value_type_at_path(&path, target_type) {
                                Ok(()) => {
                                    self.status =
                                        format!("Converted value to {}", target_type.as_str());
                                }
                                Err(err) => {
                                    self.status = format!("Type conversion failed: {err}");
                                }
                            }
                        } else {
                            self.prompt = Some(PromptState::ChangeType { path });
                        }
                    }
                    _ => {
                        self.prompt = Some(PromptState::ChangeType { path });
                    }
                }
                true
            }
            PromptState::NewFile {
                mut input,
                mut cursor,
            } => {
                match code {
                    KeyCode::Esc => {
                        self.status = "New file cancelled".to_string();
                    }
                    KeyCode::Left => {
                        cursor = cursor.min(input.chars().count()).saturating_sub(1);
                        self.prompt = Some(PromptState::NewFile { input, cursor });
                    }
                    KeyCode::Right => {
                        let len = input.chars().count();
                        cursor = (cursor.min(len) + 1).min(len);
                        self.prompt = Some(PromptState::NewFile { input, cursor });
                    }
                    KeyCode::Home => {
                        cursor = 0;
                        self.prompt = Some(PromptState::NewFile { input, cursor });
                    }
                    KeyCode::End => {
                        cursor = input.chars().count();
                        self.prompt = Some(PromptState::NewFile { input, cursor });
                    }
                    KeyCode::Backspace => {
                        if is_option_backspace(modifiers)
                            || modifiers.contains(KeyModifiers::CONTROL)
                        {
                            delete_word_before_cursor(&mut input, &mut cursor);
                        } else {
                            delete_char_before_cursor(&mut input, &mut cursor);
                        }
                        self.prompt = Some(PromptState::NewFile { input, cursor });
                    }
                    KeyCode::Char('w') if modifiers.contains(KeyModifiers::CONTROL) => {
                        delete_word_before_cursor(&mut input, &mut cursor);
                        self.prompt = Some(PromptState::NewFile { input, cursor });
                    }
                    KeyCode::Delete => {
                        delete_char_at_cursor(&mut input, &mut cursor);
                        self.prompt = Some(PromptState::NewFile { input, cursor });
                    }
                    KeyCode::Enter => {
                        if input.trim().is_empty() {
                            self.status = "Filename cannot be empty".to_string();
                            self.prompt = Some(PromptState::NewFile { input, cursor });
                        } else {
                            match self.resolve_new_file_path(&input) {
                                Ok(path) => self.request_open_or_create_file(path),
                                Err(err) => {
                                    self.status = format!("New file failed: {err}");
                                    self.prompt = Some(PromptState::NewFile { input, cursor });
                                }
                            }
                        }
                    }
                    KeyCode::Char(ch)
                        if !modifiers.intersects(
                            KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::META,
                        ) =>
                    {
                        insert_char_at_cursor(&mut input, &mut cursor, ch);
                        self.prompt = Some(PromptState::NewFile { input, cursor });
                    }
                    _ => {
                        self.prompt = Some(PromptState::NewFile { input, cursor });
                    }
                }
                true
            }
        }
    }

    fn handle_add_key_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        let Some(mut add_key) = self.add_key_mode.take() else {
            return false;
        };

        match code {
            KeyCode::Esc => {
                self.status = "Add key cancelled".to_string();
            }
            KeyCode::Enter => match add_key.stage {
                AddKeyStage::Key => {
                    if add_key.key_input.trim().is_empty() {
                        self.status = "Key name cannot be empty".to_string();
                        self.add_key_mode = Some(add_key);
                    } else {
                        add_key.stage = AddKeyStage::Value;
                        add_key.value_cursor = add_key.value_input.chars().count();
                        self.add_key_mode = Some(add_key);
                    }
                }
                AddKeyStage::Value => {
                    let key = add_key.key_input.trim().to_string();
                    let value = parse_user_value_literal(&add_key.value_input);
                    match self.add_key_at_path(&add_key.object_path, key, value) {
                        Ok(_) => {
                            self.status = "Key added".to_string();
                        }
                        Err(err) => {
                            let message = err.to_string();
                            if is_duplicate_key_error(&message) {
                                self.error_modal = Some(ErrorModalState {
                                    title: "Duplicate key".to_string(),
                                    message: format!(
                                        "This object already has a key named \"{}\".\nChoose a different key name.",
                                        sanitize_for_terminal(add_key.key_input.trim())
                                    ),
                                });
                                self.status = "Duplicate key blocked".to_string();
                            } else {
                                self.status = format!("Add key failed: {err}");
                            }
                            self.add_key_mode = Some(add_key);
                        }
                    }
                }
            },
            KeyCode::Tab => {
                add_key.stage = match add_key.stage {
                    AddKeyStage::Key => AddKeyStage::Value,
                    AddKeyStage::Value => AddKeyStage::Key,
                };
                self.add_key_mode = Some(add_key);
            }
            KeyCode::Up => {
                add_key.stage = AddKeyStage::Key;
                self.add_key_mode = Some(add_key);
            }
            KeyCode::Down => {
                if matches!(add_key.stage, AddKeyStage::Key) && add_key.key_input.trim().is_empty()
                {
                    self.status = "Enter a key name first".to_string();
                } else {
                    add_key.stage = AddKeyStage::Value;
                }
                self.add_key_mode = Some(add_key);
            }
            KeyCode::Left => {
                let (input, cursor) = add_key.active_field_mut();
                *cursor = (*cursor).min(input.chars().count()).saturating_sub(1);
                self.add_key_mode = Some(add_key);
            }
            KeyCode::Right => {
                let (input, cursor) = add_key.active_field_mut();
                let len = input.chars().count();
                *cursor = ((*cursor).min(len) + 1).min(len);
                self.add_key_mode = Some(add_key);
            }
            KeyCode::Home => {
                let (_, cursor) = add_key.active_field_mut();
                *cursor = 0;
                self.add_key_mode = Some(add_key);
            }
            KeyCode::End => {
                let (input, cursor) = add_key.active_field_mut();
                *cursor = input.chars().count();
                self.add_key_mode = Some(add_key);
            }
            KeyCode::Backspace => {
                let (input, cursor) = add_key.active_field_mut();
                if is_option_backspace(modifiers) || modifiers.contains(KeyModifiers::CONTROL) {
                    delete_word_before_cursor(input, cursor);
                } else {
                    delete_char_before_cursor(input, cursor);
                }
                self.add_key_mode = Some(add_key);
            }
            KeyCode::Char('w') if modifiers.contains(KeyModifiers::CONTROL) => {
                let (input, cursor) = add_key.active_field_mut();
                delete_word_before_cursor(input, cursor);
                self.add_key_mode = Some(add_key);
            }
            KeyCode::Delete => {
                let (input, cursor) = add_key.active_field_mut();
                delete_char_at_cursor(input, cursor);
                self.add_key_mode = Some(add_key);
            }
            KeyCode::Char(ch)
                if !modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::META) =>
            {
                let (input, cursor) = add_key.active_field_mut();
                insert_char_at_cursor(input, cursor, ch);
                self.add_key_mode = Some(add_key);
            }
            _ => {
                self.add_key_mode = Some(add_key);
            }
        }

        true
    }

    fn handle_edit_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        let Some(mut edit) = self.edit_mode.take() else {
            return false;
        };

        match code {
            KeyCode::Esc => {
                self.status = "Edit cancelled".to_string();
            }
            KeyCode::Left => {
                edit.cursor = edit
                    .cursor
                    .min(edit.input.chars().count())
                    .saturating_sub(1);
                self.edit_mode = Some(edit);
            }
            KeyCode::Right => {
                let len = edit.input.chars().count();
                edit.cursor = (edit.cursor.min(len) + 1).min(len);
                self.edit_mode = Some(edit);
            }
            KeyCode::Home => {
                edit.cursor = 0;
                self.edit_mode = Some(edit);
            }
            KeyCode::End => {
                edit.cursor = edit.input.chars().count();
                self.edit_mode = Some(edit);
            }
            KeyCode::Backspace => {
                if edit.target == EditTarget::Value
                    && edit.value_mode == ValueEditMode::QuotedString
                    && edit.input.is_empty()
                {
                    edit.value_mode = ValueEditMode::RawLiteral;
                } else if is_option_backspace(modifiers)
                    || modifiers.contains(KeyModifiers::CONTROL)
                {
                    delete_word_before_cursor(&mut edit.input, &mut edit.cursor);
                } else {
                    delete_char_before_cursor(&mut edit.input, &mut edit.cursor);
                }
                self.edit_mode = Some(edit);
            }
            KeyCode::Char('w') if modifiers.contains(KeyModifiers::CONTROL) => {
                delete_word_before_cursor(&mut edit.input, &mut edit.cursor);
                self.edit_mode = Some(edit);
            }
            KeyCode::Delete => {
                delete_char_at_cursor(&mut edit.input, &mut edit.cursor);
                self.edit_mode = Some(edit);
            }
            KeyCode::Enter => {
                let result = match edit.target {
                    EditTarget::Key => {
                        self.rename_key_at_path(&edit.path, edit.input.clone())
                            .map(|new_path| {
                                self.rebuild_rows(Some(new_path));
                            })
                    }
                    EditTarget::Value => self
                        .apply_value_edit(&edit.path, &edit.input, edit.value_mode)
                        .map(|_| {
                            self.rebuild_rows(Some(edit.path.clone()));
                        }),
                };

                match result {
                    Ok(()) => {
                        self.status = "Edit applied".to_string();
                    }
                    Err(err) => {
                        self.status = format!("Edit failed: {err}");
                    }
                }
            }
            KeyCode::Up | KeyCode::Down => {
                self.edit_mode = Some(edit);
            }
            KeyCode::Char(ch)
                if !modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::META) =>
            {
                insert_char_at_cursor(&mut edit.input, &mut edit.cursor, ch);
                self.edit_mode = Some(edit);
            }
            _ => {
                self.edit_mode = Some(edit);
            }
        }

        true
    }

    fn delete_selected_by_target(&mut self) -> bool {
        if self.focus() != FocusArea::Editor
            || self.prompt.is_some()
            || self.is_inline_input_active()
        {
            return false;
        }

        let Some(row) = self.current_row().cloned() else {
            return false;
        };

        if !matches!(row.kind, RowKind::Value) {
            self.status = "Nothing to delete on this row".to_string();
            return true;
        }

        let path = row.path;
        let result = match self.edit_target {
            EditTarget::Key => self.delete_entry_at_path(&path).map(|message| {
                self.rebuild_rows(Some(path.clone()));
                message
            }),
            EditTarget::Value => self.reset_value_at_path(&path).map(|message| {
                self.rebuild_rows(Some(path.clone()));
                message
            }),
        };

        match result {
            Ok(message) => self.status = message,
            Err(err) => self.status = format!("Delete failed: {err}"),
        }

        true
    }

    fn delete_entry_at_path(&mut self, path: &NodePath) -> Result<String> {
        let Some(last) = path.last() else {
            bail!("cannot delete root key target");
        };

        match last {
            PathSegment::Key(key) => {
                let parent_path =
                    parent_path(path).ok_or_else(|| anyhow!("missing parent path"))?;
                let parent = value_at_path_mut(&mut self.document.root, &parent_path)
                    .and_then(Value::as_object_mut)
                    .ok_or_else(|| anyhow!("parent is not an object"))?;

                parent
                    .shift_remove(key)
                    .ok_or_else(|| anyhow!("key '{}' not found", key))?;

                self.prune_collapsed_paths();
                Ok(format!(
                    "Deleted key-value pair '{}'",
                    sanitize_for_terminal(key)
                ))
            }
            PathSegment::Index(index) => {
                let parent_path =
                    parent_path(path).ok_or_else(|| anyhow!("missing parent path"))?;
                let parent = value_at_path_mut(&mut self.document.root, &parent_path)
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| anyhow!("parent is not an array"))?;

                if *index >= parent.len() {
                    bail!("array index out of bounds");
                }
                parent.remove(*index);
                self.rewrite_collapsed_paths_on_array_remove(&parent_path, *index);
                self.prune_collapsed_paths();

                Ok(format!("Removed array item [{index}]"))
            }
        }
    }

    fn reset_value_at_path(&mut self, path: &NodePath) -> Result<String> {
        let slot = value_at_path_mut(&mut self.document.root, path)
            .ok_or_else(|| anyhow!("value not found"))?;
        *slot = Value::Null;

        self.prune_collapsed_paths();

        let message = match path.last() {
            Some(PathSegment::Key(key)) => {
                format!("Reset value for '{}' to null", sanitize_for_terminal(key))
            }
            Some(PathSegment::Index(index)) => format!("Reset array item [{index}] to null"),
            None => "Reset root value to null".to_string(),
        };

        Ok(message)
    }

    fn finish_pending_action(&mut self, action: PendingAction) {
        match action {
            PendingAction::Quit => {
                self.running = false;
            }
            PendingAction::SwitchFile(index) => {
                if let Err(err) = self.open_file(index) {
                    self.status = format!("File switch failed: {err}");
                    self.explorer_cursor = self.active_file_idx;
                }
            }
            PendingAction::OpenOrCreateFile(path) => {
                if let Err(err) = self.open_or_create_file(path) {
                    self.status = format!("New file failed: {err}");
                    self.explorer_cursor = self.active_file_idx;
                }
            }
        }
    }

    fn apply_value_edit(
        &mut self,
        path: &NodePath,
        input: &str,
        value_mode: ValueEditMode,
    ) -> Result<()> {
        let target = value_at_path_mut(&mut self.document.root, path)
            .ok_or_else(|| anyhow!("missing mutable value at selected path"))?;
        *target = parse_value_input(input, value_mode);

        self.prune_collapsed_paths();
        Ok(())
    }

    fn is_container_path(&self, path: &NodePath) -> bool {
        value_at_path(&self.document.root, path)
            .map(|value| matches!(value, Value::Object(_) | Value::Array(_)))
            .unwrap_or(false)
    }

    fn prune_collapsed_paths(&mut self) {
        let root = &self.document.root;
        self.collapsed_paths.retain(|path| {
            value_at_path(root, path)
                .map(|value| matches!(value, Value::Object(_) | Value::Array(_)))
                .unwrap_or(false)
        });
    }

    fn rewrite_collapsed_paths_on_array_remove(
        &mut self,
        array_path: &NodePath,
        removed_index: usize,
    ) {
        let mut updated = HashSet::with_capacity(self.collapsed_paths.len());

        for existing in &self.collapsed_paths {
            if !path_starts_with(existing, array_path) || existing.len() <= array_path.len() {
                updated.insert(existing.clone());
                continue;
            }

            let mut rewritten = existing.clone();
            match &existing[array_path.len()] {
                PathSegment::Index(index) if *index == removed_index => {
                    continue;
                }
                PathSegment::Index(index) if *index > removed_index => {
                    rewritten[array_path.len()] = PathSegment::Index(index - 1);
                    updated.insert(rewritten);
                }
                _ => {
                    updated.insert(existing.clone());
                }
            }
        }

        self.collapsed_paths = updated;
    }

    fn rewrite_collapsed_paths_on_rename(&mut self, old_path: &NodePath, new_path: &NodePath) {
        let mut updated = HashSet::with_capacity(self.collapsed_paths.len());

        for existing in &self.collapsed_paths {
            if path_starts_with(existing, old_path) {
                let mut rewritten = new_path.clone();
                rewritten.extend(existing[old_path.len()..].iter().cloned());
                updated.insert(rewritten);
            } else {
                updated.insert(existing.clone());
            }
        }

        self.collapsed_paths = updated;
    }

    fn rename_key_at_path(&mut self, path: &NodePath, new_key: String) -> Result<NodePath> {
        if new_key.trim().is_empty() {
            bail!("key cannot be empty");
        }

        let (parent_path, old_key) =
            split_parent_key(path).ok_or_else(|| anyhow!("not an object key"))?;

        let parent = value_at_path_mut(&mut self.document.root, &parent_path)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| anyhow!("parent is not an object"))?;

        if old_key == new_key {
            return Ok(path.clone());
        }

        if parent.contains_key(&new_key) {
            bail!("key '{}' already exists", new_key);
        }

        let old_index = parent
            .keys()
            .position(|candidate| candidate == &old_key)
            .ok_or_else(|| anyhow!("key '{}' not found", old_key))?;

        let value = parent
            .shift_remove(&old_key)
            .ok_or_else(|| anyhow!("key '{}' not found", old_key))?;
        parent.shift_insert(old_index, new_key.clone(), value);

        let mut new_path = parent_path;
        new_path.push(PathSegment::Key(new_key));

        self.rewrite_collapsed_paths_on_rename(path, &new_path);
        self.prune_collapsed_paths();

        Ok(new_path)
    }

    fn add_key_at_path(
        &mut self,
        object_path: &NodePath,
        key: String,
        value: Value,
    ) -> Result<NodePath> {
        if key.trim().is_empty() {
            bail!("key cannot be empty");
        }

        let object = value_at_path_mut(&mut self.document.root, object_path)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| anyhow!("selected node is not an object"))?;

        if object.contains_key(&key) {
            bail!("key '{}' already exists", key);
        }

        object.insert(key.clone(), value);

        let mut new_path = object_path.clone();
        new_path.push(PathSegment::Key(key));

        self.collapsed_paths.remove(object_path);
        self.prune_collapsed_paths();
        self.rebuild_rows(Some(new_path.clone()));
        Ok(new_path)
    }

    fn add_item_at_path(&mut self, array_path: &NodePath, value: Value) -> Result<NodePath> {
        let array = value_at_path_mut(&mut self.document.root, array_path)
            .and_then(Value::as_array_mut)
            .ok_or_else(|| anyhow!("selected node is not an array"))?;

        let next_index = array.len();
        array.push(value);

        let mut new_path = array_path.clone();
        new_path.push(PathSegment::Index(next_index));

        self.collapsed_paths.remove(array_path);
        self.prune_collapsed_paths();
        self.rebuild_rows(Some(new_path.clone()));
        Ok(new_path)
    }

    fn change_value_type_at_path(&mut self, path: &NodePath, target_type: JsonType) -> Result<()> {
        let current = value_at_path(&self.document.root, path)
            .cloned()
            .ok_or_else(|| anyhow!("value not found"))?;

        let converted = convert_value_type(&current, target_type)?;
        let slot = value_at_path_mut(&mut self.document.root, path)
            .ok_or_else(|| anyhow!("value not found"))?;
        *slot = converted;

        self.prune_collapsed_paths();
        self.rebuild_rows(Some(path.clone()));
        Ok(())
    }

    fn copy_to_clipboard(&mut self) {
        if let Some(edit) = &self.edit_mode {
            self.clipboard.write_text(edit.input.clone());
            self.status = "Copied current edit text".to_string();
            return;
        }

        if let Some(add_key) = &self.add_key_mode {
            let text = match add_key.stage {
                AddKeyStage::Key => add_key.key_input.clone(),
                AddKeyStage::Value => add_key.value_input.clone(),
            };
            self.clipboard.write_text(text);
            self.status = "Copied add-key field text".to_string();
            return;
        }

        if let Some(PromptState::NewFile { input, .. }) = &self.prompt {
            self.clipboard.write_text(input.clone());
            self.status = "Copied new-file input".to_string();
            return;
        }

        if self.focus() != FocusArea::Editor || self.prompt.is_some() {
            self.status = "Copy is available in editor rows and active input fields".to_string();
            return;
        }

        let Some(row) = self.current_row().cloned() else {
            self.status = "Nothing selected to copy".to_string();
            return;
        };

        match (&row.kind, self.edit_target) {
            (RowKind::Value, EditTarget::Value) => {
                if let Some(value) = value_at_path(&self.document.root, &row.path) {
                    let text = value_to_clipboard_text(value);
                    self.clipboard.write_text(text);
                    self.status = format!("Copied VALUE from {}", path_to_string(&row.path));
                } else {
                    self.status = "Copy failed: selected value not found".to_string();
                }
            }
            (RowKind::Value, EditTarget::Key) => {
                if let Some((_, key)) = split_parent_key(&row.path) {
                    if let Some(value) = value_at_path(&self.document.root, &row.path) {
                        let text = row_payload_to_clipboard_text(&key, value);
                        self.clipboard.write_text(text);
                        self.status = format!(
                            "Copied KEY row payload for '{}'",
                            sanitize_for_terminal(&key)
                        );
                    } else {
                        self.status = "Copy failed: selected value not found".to_string();
                    }
                } else {
                    self.status =
                        "Copy unavailable here: KEY payload requires an object entry".to_string();
                }
            }
            (RowKind::AddKeyAction { .. }, _) => {
                self.status = "Copy unavailable on + add key row".to_string();
            }
            (RowKind::AddItemAction { .. }, _) => {
                self.status = "Copy unavailable on + add item row".to_string();
            }
        }
    }

    fn paste_from_clipboard_shortcut(&mut self) {
        match self.clipboard.read_text() {
            Ok(text) => self.apply_paste_text(text),
            Err(err) => {
                self.error_modal = Some(ErrorModalState {
                    title: "Paste failed".to_string(),
                    message: format!(
                        "Clipboard text is unavailable.\n\n{}",
                        sanitize_for_terminal(&err.to_string())
                    ),
                });
                self.status = "Paste failed: clipboard unavailable".to_string();
            }
        }
    }

    fn apply_paste_text(&mut self, text: String) {
        let text = normalize_clipboard_text(&text);

        if let Some(edit) = &mut self.edit_mode {
            insert_text_at_cursor(&mut edit.input, &mut edit.cursor, &text);
            self.status = "Pasted into edit input".to_string();
            return;
        }

        if let Some(add_key) = &mut self.add_key_mode {
            let (input, cursor) = add_key.active_field_mut();
            insert_text_at_cursor(input, cursor, &text);
            self.status = "Pasted into add-key field".to_string();
            return;
        }

        if let Some(PromptState::NewFile { input, cursor }) = self.prompt.as_mut() {
            insert_text_at_cursor(input, cursor, &text);
            self.status = "Pasted into new-file input".to_string();
            return;
        }

        if self.focus() == FocusArea::Explorer && self.prompt.is_none() {
            let mut next = self.explorer_filter.clone();
            next.push_str(&text);
            self.apply_explorer_filter(next);
            self.status = "Pasted into explorer filter".to_string();
            return;
        }

        if self.focus() != FocusArea::Editor || self.prompt.is_some() {
            self.status = "Paste is available in editor rows and active input fields".to_string();
            return;
        }

        let Some(row) = self.current_row().cloned() else {
            self.status = "Nothing selected to paste into".to_string();
            return;
        };

        let result = match (&row.kind, self.edit_target) {
            (RowKind::Value, EditTarget::Value) => self
                .apply_value_edit(&row.path, &text, ValueEditMode::RawLiteral)
                .map(|_| {
                    self.rebuild_rows(Some(row.path.clone()));
                    format!("Pasted VALUE into {}", path_to_string(&row.path))
                }),
            (RowKind::Value, EditTarget::Key) => self.paste_into_key_target(&row.path, &text),
            (RowKind::AddKeyAction { object_path }, _) => {
                self.paste_into_add_key_action(object_path, &text)
            }
            (RowKind::AddItemAction { array_path }, _) => {
                let value = parse_user_value_literal(&text);
                self.add_item_at_path(array_path, value).map(|new_path| {
                    format!("Pasted and appended item at {}", path_to_string(&new_path))
                })
            }
        };

        match result {
            Ok(message) => {
                self.error_modal = None;
                self.status = message;
            }
            Err(err) => {
                let details = err.to_string();
                if is_duplicate_key_error(&details) {
                    self.error_modal = Some(ErrorModalState {
                        title: "Paste conflict".to_string(),
                        message: format!(
                            "Cannot paste because key(s) already exist in this object.\n\n{}",
                            sanitize_for_terminal(&details)
                        ),
                    });
                    self.status = "Paste blocked by duplicate key".to_string();
                } else {
                    self.error_modal = Some(ErrorModalState {
                        title: "Paste failed".to_string(),
                        message: sanitize_for_terminal(&details),
                    });
                    self.status = format!("Paste failed: {details}");
                }
            }
        }
    }

    fn paste_into_key_target(&mut self, path: &NodePath, text: &str) -> Result<String> {
        if let Some(payload) = parse_object_payload(text) {
            let object_path = parent_path(path)
                .or_else(|| self.nearest_object_context(path))
                .ok_or_else(|| anyhow!("no object context available for row payload paste"))?;

            let first = self.insert_object_payload_at_path(&object_path, payload)?;
            return Ok(format!(
                "Pasted row payload into {}",
                path_to_string(&first)
            ));
        }

        let key_text = text.trim();
        if key_text.is_empty() {
            bail!("clipboard key text is empty");
        }

        if split_parent_key(path).is_some() {
            let new_path = self.rename_key_at_path(path, key_text.to_string())?;
            self.rebuild_rows(Some(new_path.clone()));
            return Ok(format!(
                "Renamed key to '{}'",
                sanitize_for_terminal(key_text)
            ));
        }

        let object_path = self
            .nearest_object_context(path)
            .ok_or_else(|| anyhow!("no object context available for key paste"))?;
        let new_path = self.add_key_at_path(&object_path, key_text.to_string(), Value::Null)?;
        Ok(format!("Inserted key at {}", path_to_string(&new_path)))
    }

    fn paste_into_add_key_action(&mut self, object_path: &NodePath, text: &str) -> Result<String> {
        if let Some(payload) = parse_object_payload(text) {
            let first = self.insert_object_payload_at_path(object_path, payload)?;
            return Ok(format!(
                "Pasted object payload at {}",
                path_to_string(&first)
            ));
        }

        let key = text.trim();
        if key.is_empty() {
            bail!("clipboard text is empty");
        }

        self.begin_add_key_for_object(object_path.clone());
        if let Some(add_key) = self.add_key_mode.as_mut() {
            add_key.key_input = key.to_string();
            add_key.key_cursor = add_key.key_input.chars().count();
            add_key.stage = AddKeyStage::Value;
        }

        Ok("Pasted key name into add-key flow".to_string())
    }

    fn nearest_object_context(&self, path: &NodePath) -> Option<NodePath> {
        for len in (0..=path.len()).rev() {
            let candidate = path[..len].to_vec();
            if value_at_path(&self.document.root, &candidate).is_some_and(Value::is_object) {
                return Some(candidate);
            }
        }
        None
    }

    fn insert_object_payload_at_path(
        &mut self,
        object_path: &NodePath,
        payload: Map<String, Value>,
    ) -> Result<NodePath> {
        if payload.is_empty() {
            bail!("payload object is empty");
        }

        let object = value_at_path_mut(&mut self.document.root, object_path)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| anyhow!("selected target is not an object"))?;

        let mut duplicates = Vec::new();
        for key in payload.keys() {
            if object.contains_key(key) {
                duplicates.push(key.clone());
            }
        }

        if !duplicates.is_empty() {
            bail!("duplicate key(s): {}", duplicates.join(", "));
        }

        let first_key = payload
            .keys()
            .next()
            .cloned()
            .ok_or_else(|| anyhow!("payload object is empty"))?;

        for (key, value) in payload {
            object.insert(key, value);
        }

        let mut first_path = object_path.clone();
        first_path.push(PathSegment::Key(first_key));

        self.collapsed_paths.remove(object_path);
        self.prune_collapsed_paths();
        self.rebuild_rows(Some(first_path.clone()));

        Ok(first_path)
    }

    fn rebuild_rows(&mut self, preferred_path: Option<NodePath>) {
        let previous_target = self.edit_target;
        let current_path = preferred_path.unwrap_or_else(|| self.current_path());
        self.rows = flatten_rows(&self.document.root, &self.collapsed_paths);

        if self.rows.is_empty() {
            self.selected_row = 0;
            self.edit_target = EditTarget::Value;
            self.inspector_scroll = 0;
            return;
        }

        if let Some(index) = find_best_row_index(&self.rows, &current_path) {
            self.selected_row = index;
        } else {
            self.selected_row = self.selected_row.min(self.rows.len() - 1);
        }

        let key_available = self.current_row().is_some_and(row_supports_key_target);
        self.edit_target = if previous_target == EditTarget::Key && key_available {
            EditTarget::Key
        } else {
            EditTarget::Value
        };

        self.inspector_scroll = 0;
    }

    fn animation_seconds(&self) -> f64 {
        self.launched_at.elapsed().as_secs_f64()
    }

    fn toggle_logo(&mut self) {
        self.logo_visible = !self.logo_visible;
        self.status = if self.logo_visible {
            "STEWS logo animation enabled".to_string()
        } else {
            "STEWS logo animation hidden".to_string()
        };
    }

    fn prompt_line(&self) -> String {
        if let Some(edit) = &self.edit_mode {
            return format!(
                "Editing {} at {} · Enter apply · Esc cancel",
                edit.target.as_str(),
                path_to_string(&edit.path)
            );
        }

        if self.add_key_mode.is_some() {
            return "Add key inline in canvas · Enter next/apply · Tab switch field · Esc cancel"
                .to_string();
        }

        if let Some(prompt) = &self.prompt {
            return match prompt {
                PromptState::DirtyConfirm { .. } => {
                    "Unsaved changes. Save before continue? [s]ave / [d]iscard / [c]ancel"
                        .to_string()
                }
                PromptState::ChangeType { .. } => {
                    "Change type: [s]string [n]number [b]bool [u]null [o]object [a]array (Esc cancel)"
                        .to_string()
                }
                PromptState::NewFile { .. } => {
                    "New file dialog: Enter create/open · Esc cancel".to_string()
                }
            };
        }

        self.status.clone()
    }
}

fn main() -> Result<()> {
    load_dotenv_from_current_dir()?;

    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("stews - modern JSON-focused terminal editor");
        println!("\nRun without arguments to scan JSON files recursively.");
        println!("Pass explicit files: stews file1.json file2.json");
        println!(
            "Set {JSON_ROOT_ENV_VAR} to change the default scan root (supports .env/.env.local)."
        );
        println!(
            "\nKeys: Tab/Shift+Tab focus • Arrows navigate • Enter edit • ⌫/⌥⌫ delete/reset • ⌘C/Ctrl+Shift+C copy • ⌘V/Ctrl+V paste • a add key/item • t change type • L logo • N new file • w save • q/Ctrl+C quit"
        );
        return Ok(());
    }

    let paths: Vec<PathBuf> = args.into_iter().map(PathBuf::from).collect();
    let app = App::bootstrap(paths)?;
    run_tui(app)
}

fn load_dotenv_from_current_dir() -> Result<()> {
    let cwd = env::current_dir().context("failed to determine current working directory")?;
    load_dotenv_from_dir(&cwd)
}

fn load_dotenv_from_dir(dir: &Path) -> Result<()> {
    let shell_keys = env::vars().map(|(key, _)| key).collect::<HashSet<_>>();
    let assignments = collect_dotenv_assignments(dir, &shell_keys)?;

    for (key, value) in assignments {
        // SAFETY: dotenv values are loaded once during process startup, before
        // the TUI event loop starts and before any threads are spawned.
        unsafe {
            env::set_var(key, value);
        }
    }

    Ok(())
}

fn collect_dotenv_assignments(
    dir: &Path,
    shell_keys: &HashSet<String>,
) -> Result<Vec<(String, String)>> {
    let mut assignments = Vec::new();

    for filename in DOTENV_FILES {
        collect_dotenv_assignments_from_file(&dir.join(filename), shell_keys, &mut assignments)?;
    }

    Ok(assignments)
}

fn collect_dotenv_assignments_from_file(
    file_path: &Path,
    shell_keys: &HashSet<String>,
    assignments: &mut Vec<(String, String)>,
) -> Result<()> {
    if !file_path.is_file() {
        return Ok(());
    }

    let entries = dotenvy::from_path_iter(file_path)
        .with_context(|| format!("failed to read {}", file_path.display()))?;

    for entry in entries {
        let (key, value) =
            entry.with_context(|| format!("failed to parse {}", file_path.display()))?;
        if shell_keys.contains(&key) {
            continue;
        }

        assignments.push((key, value));
    }

    Ok(())
}

fn discover_root_path() -> Result<PathBuf> {
    let cwd = env::current_dir().context("failed to determine current working directory")?;

    let configured_root = match env::var(JSON_ROOT_ENV_VAR) {
        Ok(value) => Some(value),
        Err(env::VarError::NotPresent) => None,
        Err(env::VarError::NotUnicode(_)) => {
            bail!("{JSON_ROOT_ENV_VAR} contains non-unicode data")
        }
    };

    Ok(resolve_discovery_root(&cwd, configured_root.as_deref()))
}

fn resolve_discovery_root(current_dir: &Path, configured_root: Option<&str>) -> PathBuf {
    let Some(raw_root) = configured_root else {
        return current_dir.to_path_buf();
    };

    let trimmed = raw_root.trim();
    if trimmed.is_empty() {
        return current_dir.to_path_buf();
    }

    let root = PathBuf::from(trimmed);
    if root.is_absolute() {
        root
    } else {
        current_dir.join(root)
    }
}

fn run_tui(mut app: App) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let loop_result = run_event_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;

    loop_result
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    while app.running {
        terminal.draw(|frame| render(frame, app))?;

        if event::poll(Duration::from_millis(80))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    handle_key(app, key.code, key.modifiers);
                }
                Event::Paste(text) => {
                    app.apply_paste_text(text);
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn render(frame: &mut Frame, app: &App) {
    frame.render_widget(
        Block::default().style(Style::default().bg(theme::BG)),
        frame.area(),
    );

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_top_strip(frame, root[0], app);

    let inspector_width = if app.inspector_collapsed {
        theme::INSPECTOR_COLLAPSED
    } else {
        theme::INSPECTOR_EXPANDED
    };

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(theme::EXPLORER_WIDTH),
            Constraint::Min(45),
            Constraint::Length(inspector_width),
        ])
        .split(root[1]);

    render_explorer(frame, body[0], app);
    render_editor(frame, body[1], app);
    render_inspector(frame, body[2], app);
    render_dock(frame, root[2], app);

    frame.render_widget(
        Paragraph::new(app.prompt_line())
            .style(Style::default().bg(theme::PROMPT_BG).fg(theme::TEXT_MUTED)),
        root[3],
    );

    if let Some(modal) = &app.save_modal {
        render_save_modal(frame, modal);
    }

    if let Some(modal) = &app.error_modal {
        render_error_modal(frame, modal);
    }

    if let Some(PromptState::NewFile { input, cursor }) = &app.prompt {
        render_new_file_modal(frame, input, *cursor);
    }
}

fn render_save_modal(frame: &mut Frame, modal: &SaveModalState) {
    let area = centered_rect(56, 5, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(modal.message.clone())
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .title(" Saved ")
                    .borders(Borders::ALL)
                    .style(Style::default().bg(theme::DOCK_BG).fg(theme::TEXT)),
            )
            .style(Style::default().bg(theme::DOCK_BG).fg(theme::TEXT)),
        area,
    );
}

fn render_error_modal(frame: &mut Frame, modal: &ErrorModalState) {
    let area = centered_rect(66, 8, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(format!(
            "{}\n\nPress Esc or Enter to continue",
            modal.message
        ))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .block(
            Block::default()
                .title(format!(" {} ", modal.title))
                .borders(Borders::ALL)
                .style(Style::default().bg(theme::DOCK_BG).fg(theme::TEXT)),
        )
        .style(Style::default().bg(theme::DOCK_BG).fg(theme::TEXT)),
        area,
    );
}

fn render_new_file_modal(frame: &mut Frame, input: &str, cursor: usize) {
    let area = centered_rect(66, 7, frame.area());
    let input_with_caret = with_caret(input, cursor);

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(" Enter file name/path"),
            Line::from(""),
            Line::from(Span::styled(
                input_with_caret,
                Style::default().fg(theme::TEXT),
            )),
        ])
        .block(
            Block::default()
                .title(" New file ")
                .borders(Borders::ALL)
                .style(Style::default().bg(theme::DOCK_BG).fg(theme::TEXT)),
        )
        .style(Style::default().bg(theme::DOCK_BG).fg(theme::TEXT)),
        area,
    );
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let width = (area.width.saturating_mul(percent_x) / 100).max(24);
    let width = width.min(area.width.saturating_sub(2).max(1));
    let x = area.x + (area.width.saturating_sub(width)) / 2;

    let h = height.min(area.height.saturating_sub(2).max(1));
    let y = area.y + (area.height.saturating_sub(h)) / 2;

    Rect {
        x,
        y,
        width,
        height: h,
    }
}

fn render_top_strip(frame: &mut Frame, area: Rect, app: &App) {
    frame.render_widget(
        Block::default().style(Style::default().bg(theme::TOP_STRIP_BG)),
        area,
    );

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(25),
            Constraint::Min(20),
            Constraint::Length(36),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new("  STEWS  ✦  UI V2").style(
            Style::default()
                .fg(theme::TOP_STRIP_TEXT)
                .bg(theme::TOP_STRIP_BG),
        ),
        cols[0],
    );

    let dirty = if app.document.is_dirty() {
        " ● dirty"
    } else {
        ""
    };
    frame.render_widget(
        Paragraph::new(format!(
            "{}{}   ·   {}",
            app.current_file_display(),
            dirty,
            APP_SUBTITLE
        ))
        .alignment(Alignment::Center)
        .style(
            Style::default()
                .fg(theme::TOP_STRIP_TEXT)
                .bg(theme::TOP_STRIP_BG),
        ),
        cols[1],
    );

    let target = app.current_value_type().map(|t| t.as_str()).unwrap_or("-");
    frame.render_widget(
        Paragraph::new(format!(
            "{}   focus:{}   edit:{}",
            APP_TITLE,
            app.focus().as_str(),
            app.edit_target.as_str()
        ))
        .alignment(Alignment::Right)
        .style(
            Style::default()
                .fg(theme::TOP_STRIP_MUTED)
                .bg(theme::TOP_STRIP_BG),
        ),
        cols[2],
    );

    let _ = target;
}

fn render_explorer(frame: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus() == FocusArea::Explorer;
    let bg = if focused {
        theme::EXPLORER_FOCUS_BG
    } else {
        theme::EXPLORER_BG
    };

    frame.render_widget(
        Block::default()
            .style(Style::default().bg(bg))
            .borders(Borders::RIGHT)
            .border_style(Style::default().fg(theme::BORDER)),
        area,
    );

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
        ])
        .margin(1)
        .split(area);

    frame.render_widget(
        Paragraph::new("󰙅  EXPLORER").style(
            Style::default()
                .fg(theme::TEXT)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        ),
        rows[0],
    );

    let filter_hint = if app.explorer_filter.is_empty() {
        "(type to filter)".to_string()
    } else {
        app.explorer_filter.clone()
    };
    let filter_line = if focused {
        format!(
            "  filter: {}",
            with_caret(&filter_hint, filter_hint.chars().count())
        )
    } else {
        format!("  filter: {}", sanitize_for_terminal(&filter_hint))
    };
    let filter_style = if app.explorer_filter.is_empty() {
        Style::default().fg(theme::TEXT_MUTED).bg(bg)
    } else {
        Style::default().fg(theme::TEXT).bg(bg)
    };
    frame.render_widget(Paragraph::new(filter_line).style(filter_style), rows[1]);

    let visible = app.filtered_file_indices();
    let mut items: Vec<ListItem> = vec![ListItem::new(Line::from(Span::styled(
        format!("  FILES  ({}/{})", visible.len(), app.files.len()),
        Style::default()
            .fg(theme::EXPLORER_GROUP)
            .add_modifier(Modifier::BOLD),
    )))];

    if visible.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "   no matching files",
            Style::default().fg(theme::TEXT_MUTED),
        ))));
    }

    for idx in visible {
        let file = &app.files[idx];
        let cursor = if idx == app.explorer_cursor {
            "▸"
        } else {
            " "
        };
        let active = if idx == app.active_file_idx {
            "●"
        } else {
            " "
        };
        let icon = if file
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|name| name.contains("settings"))
        {
            "󰒓"
        } else {
            "󰘦"
        };
        let line = format!(" {cursor}{active} {icon} {}", display_path(file));

        let style = if idx == app.explorer_cursor {
            Style::default()
                .fg(theme::TEXT)
                .bg(theme::EXPLORER_SELECTED_BG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::EXPLORER_FILE)
        };

        items.push(ListItem::new(Line::from(Span::styled(line, style))));
    }

    frame.render_widget(List::new(items).style(Style::default().bg(bg)), rows[2]);
}

fn render_editor(frame: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus() == FocusArea::Editor;
    let bg = if focused {
        theme::EDITOR_FOCUS_BG
    } else {
        theme::EDITOR_BG
    };

    frame.render_widget(
        Block::default()
            .style(Style::default().bg(bg))
            .borders(Borders::RIGHT)
            .border_style(Style::default().fg(theme::BORDER)),
        area,
    );

    let row_constraints = if app.logo_visible {
        vec![
            Constraint::Length(2),
            Constraint::Min(4),
            Constraint::Length(8),
            Constraint::Length(1),
        ]
    } else {
        vec![
            Constraint::Length(2),
            Constraint::Min(4),
            Constraint::Length(1),
        ]
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(row_constraints)
        .margin(1)
        .split(area);

    frame.render_widget(
        Paragraph::new(" 󰈮  JSON CANVAS     ◇ UI V2 ◇").style(
            Style::default()
                .fg(theme::TEXT)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        ),
        rows[0],
    );

    let lines: Vec<Line> = app
        .rows
        .iter()
        .enumerate()
        .map(|(idx, row)| render_editor_row(app, idx, row, bg))
        .collect();

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .style(Style::default().bg(bg))
            .wrap(Wrap { trim: false }),
        rows[1],
    );

    let status_row = if app.logo_visible {
        if let Some(logo_row) = rows.get(2).copied()
            && logo_row.height > 2
        {
            let logo_cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(46), Constraint::Min(1)])
                .split(logo_row);
            logo::render(frame, logo_cols[0], app.animation_seconds());
        }
        rows[3]
    } else {
        rows[2]
    };

    frame.render_widget(
        Paragraph::new(format!(
            "row:{}   node:{}   type:{}   file:{}",
            app.selected_row + 1,
            path_to_string(&app.current_path()),
            app.current_value_type().map(|t| t.as_str()).unwrap_or("-"),
            app.current_file_display()
        ))
        .style(Style::default().fg(theme::COMMENT).bg(bg)),
        status_row,
    );
}
fn render_editor_row(app: &App, idx: usize, row: &NodeRow, bg: Color) -> Line<'static> {
    let selected = idx == app.selected_row;
    let edit_state = app
        .edit_mode
        .as_ref()
        .filter(|state| state.path == row.path)
        .cloned();

    let mut spans: Vec<Span<'static>> = Vec::new();

    let marker_style = if selected {
        Style::default().fg(theme::INSPECTOR_ACCENT).bg(bg)
    } else {
        Style::default().fg(theme::PUNCT).bg(bg)
    };

    spans.push(Span::styled(
        if selected { "▌ " } else { "  " },
        marker_style,
    ));
    spans.push(Span::styled(
        format!("{:>3} ", idx + 1),
        Style::default().fg(theme::EDITOR_GUTTER).bg(bg),
    ));

    spans.push(Span::raw("  ".repeat(row.depth)));

    match &row.kind {
        RowKind::AddKeyAction { object_path } => {
            let add_mode = app
                .add_key_mode
                .as_ref()
                .filter(|mode| mode.object_path == *object_path);

            let mut style = Style::default().fg(theme::INSPECTOR_ACCENT).bg(bg);
            if selected {
                style = style.bg(theme::CHIP_BG).add_modifier(Modifier::BOLD);
            }

            if let Some(mode) = add_mode {
                let key_text = if matches!(mode.stage, AddKeyStage::Key) {
                    with_caret(&mode.key_input, mode.key_cursor)
                } else {
                    sanitize_for_terminal(&mode.key_input)
                };

                let value_text = if matches!(mode.stage, AddKeyStage::Value) {
                    with_caret(&mode.value_input, mode.value_cursor)
                } else {
                    sanitize_for_terminal(&mode.value_input)
                };

                spans.push(Span::styled(
                    format!("+ add key · key: \"{}\" · value: {}", key_text, value_text),
                    style,
                ));
            } else {
                spans.push(Span::styled("+ add key", style));
            }

            return Line::from(spans);
        }
        RowKind::AddItemAction { .. } => {
            let mut style = Style::default().fg(theme::INSPECTOR_ACCENT).bg(bg);
            if selected {
                style = style.bg(theme::CHIP_BG).add_modifier(Modifier::BOLD);
            }

            spans.push(Span::styled("+ add item", style));
            return Line::from(spans);
        }
        RowKind::Value => {}
    }

    if let Some(label) = &row.key_label {
        let safe_label = sanitize_for_terminal(label);
        let key_text = if let Some(edit) = &edit_state {
            if edit.target == EditTarget::Key {
                format!("\"{}\"", with_caret(&edit.input, edit.cursor))
            } else {
                format!("\"{}\"", safe_label)
            }
        } else {
            format!("\"{}\"", safe_label)
        };

        let mut key_style = Style::default().fg(theme::KEY_COLOR).bg(bg);
        if selected && app.edit_target == EditTarget::Key && row_supports_key_target(row) {
            key_style = key_style.bg(theme::CHIP_BG).add_modifier(Modifier::BOLD);
        }

        spans.push(Span::styled(key_text, key_style));
        spans.push(Span::styled(": ", Style::default().fg(theme::PUNCT).bg(bg)));
    }

    let value = value_at_path(&app.document.root, &row.path).unwrap_or(&Value::Null);

    let display_value = if let Some(edit) = &edit_state {
        if edit.target == EditTarget::Value {
            edit_input_to_display(&edit.input, edit.value_mode, edit.cursor)
        } else {
            value_preview(value, app.collapsed_paths.contains(&row.path))
        }
    } else {
        value_preview(value, app.collapsed_paths.contains(&row.path))
    };

    let mut value_style = value_style(value).bg(bg);
    if selected && app.edit_target == EditTarget::Value {
        value_style = value_style.bg(theme::CHIP_BG).add_modifier(Modifier::BOLD);
    }

    spans.push(Span::styled(display_value, value_style));

    Line::from(spans)
}

fn render_inspector(frame: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus() == FocusArea::Inspector;
    let bg = if focused {
        theme::INSPECTOR_FOCUS_BG
    } else {
        theme::INSPECTOR_BG
    };

    frame.render_widget(Block::default().style(Style::default().bg(bg)), area);

    if app.inspector_collapsed {
        frame.render_widget(
            Paragraph::new("I\nN\nS\nP\n\n▸")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme::TEXT_MUTED).bg(bg)),
            area,
        );
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3)])
        .margin(1)
        .split(area);

    frame.render_widget(
        Paragraph::new("󰋽  INSPECTOR").style(
            Style::default()
                .fg(theme::TEXT)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        ),
        rows[0],
    );

    let lines = inspector_lines(app)
        .into_iter()
        .skip(app.inspector_scroll)
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .style(Style::default().bg(bg))
            .wrap(Wrap { trim: false }),
        rows[1],
    );
}

fn inspector_lines(app: &App) -> Vec<Line<'static>> {
    let selected_path = app.current_path();
    let Some(value) = value_at_path(&app.document.root, &selected_path) else {
        return vec![Line::from(Span::styled(
            " no selection",
            Style::default().fg(theme::TEXT_MUTED),
        ))];
    };

    let mut lines = vec![
        Line::from(Span::styled(
            format!(" selection  {}", path_to_string(&selected_path)),
            Style::default().fg(theme::TEXT),
        )),
        Line::from(Span::styled(
            format!(" type       {}", JsonType::from_value(value).as_str()),
            Style::default().fg(theme::INSPECTOR_ACCENT),
        )),
        Line::from(Span::styled(
            format!(
                " dirty      {}",
                if app.document.is_dirty() { "yes" } else { "no" }
            ),
            Style::default().fg(theme::TEXT_MUTED),
        )),
    ];

    match value {
        Value::Object(map) => {
            lines.push(Line::from(Span::styled(
                format!(
                    " collapsed  {}",
                    if app.collapsed_paths.contains(&selected_path) {
                        "yes"
                    } else {
                        "no"
                    }
                ),
                Style::default().fg(theme::TEXT_MUTED),
            )));
            lines.push(Line::from(Span::styled(
                format!(" keys       {}", map.len()),
                Style::default().fg(theme::TEXT),
            )));
            lines.push(Line::from(Span::styled(
                " children",
                Style::default()
                    .fg(theme::EXPLORER_GROUP)
                    .add_modifier(Modifier::BOLD),
            )));
            for (key, child) in map {
                lines.push(Line::from(Span::styled(
                    format!("  • {}: {}", key, JsonType::from_value(child).as_str()),
                    Style::default().fg(theme::TEXT_MUTED),
                )));
            }
        }
        Value::Array(items) => {
            lines.push(Line::from(Span::styled(
                format!(
                    " collapsed  {}",
                    if app.collapsed_paths.contains(&selected_path) {
                        "yes"
                    } else {
                        "no"
                    }
                ),
                Style::default().fg(theme::TEXT_MUTED),
            )));
            lines.push(Line::from(Span::styled(
                format!(" length     {}", items.len()),
                Style::default().fg(theme::TEXT),
            )));
            lines.push(Line::from(Span::styled(
                " children",
                Style::default()
                    .fg(theme::EXPLORER_GROUP)
                    .add_modifier(Modifier::BOLD),
            )));
            for (idx, child) in items.iter().enumerate() {
                lines.push(Line::from(Span::styled(
                    format!("  • [{}]: {}", idx, JsonType::from_value(child).as_str()),
                    Style::default().fg(theme::TEXT_MUTED),
                )));
            }
        }
        _ => {
            lines.push(Line::from(Span::styled(
                format!(" value      {}", value_preview(value, false)),
                Style::default().fg(theme::TEXT_MUTED),
            )));
        }
    }

    lines
}

fn render_dock(frame: &mut Frame, area: Rect, app: &App) {
    frame.render_widget(
        Block::default().style(Style::default().bg(theme::DOCK_BG)),
        area,
    );

    let line = Line::from(vec![
        chip("Tab", "Pane"),
        Span::raw(" "),
        chip("↑↓", "Move"),
        Span::raw(" "),
        chip("←/→", "Key/Val"),
        Span::raw(" "),
        chip("Enter", "Edit/Focus"),
        Span::raw(" "),
        chip("⌫/⌥⌫", "Del/Null"),
        Span::raw(" "),
        chip("⌘C/^⇧C", "Copy"),
        Span::raw(" "),
        chip("⌘V/^V", "Paste"),
        Span::raw(" "),
        chip("Esc", "Cancel"),
        Span::raw(" "),
        chip("a", "Add key/item"),
        Span::raw(" "),
        chip("t", "Change type"),
        Span::raw(" "),
        chip("L", "Logo"),
        Span::raw(" "),
        chip("N", "New file"),
        Span::raw(" "),
        chip("w", "Save"),
        Span::raw(" "),
        chip("q", "Quit"),
        Span::raw(" "),
        Span::styled(
            format!(" [{}]", app.focus().as_str()),
            Style::default().fg(theme::TEXT_MUTED),
        ),
    ]);

    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(theme::DOCK_BG)),
        area,
    );
}

fn chip<'a>(key: &'a str, label: &'a str) -> Span<'a> {
    Span::styled(
        format!(" [{key}] {label} "),
        Style::default().fg(theme::TEXT).bg(theme::CHIP_BG),
    )
}

fn handle_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    if app.save_modal.is_some() {
        let consume = matches!(code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char(' '));
        app.save_modal = None;
        if consume {
            return;
        }
    }

    if app.error_modal.is_some() {
        let consume = matches!(code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char(' '));
        app.error_modal = None;
        if consume {
            return;
        }
    }

    if is_paste_shortcut(code, modifiers) {
        app.paste_from_clipboard_shortcut();
        return;
    }

    if is_copy_shortcut(code, modifiers) {
        app.copy_to_clipboard();
        return;
    }

    if app.handle_prompt_key(code, modifiers) {
        return;
    }

    if app.handle_add_key_key(code, modifiers) {
        return;
    }

    if app.handle_edit_key(code, modifiers) {
        return;
    }

    if app.handle_explorer_filter_key(code, modifiers) {
        return;
    }

    if is_non_edit_delete_shortcut(code, modifiers) && app.delete_selected_by_target() {
        return;
    }

    match code {
        KeyCode::BackTab => app.cycle_focus(true),
        KeyCode::Tab => app.cycle_focus(false),
        KeyCode::Up => app.move_up(),
        KeyCode::Down => app.move_down(),
        KeyCode::Left => app.move_left(),
        KeyCode::Right => app.move_right(),
        KeyCode::Enter => app.handle_enter(),
        KeyCode::Char('a') | KeyCode::Char('A') => app.begin_add_key(),
        KeyCode::Char('t') | KeyCode::Char('T') => app.begin_change_type(),
        KeyCode::Char('l') | KeyCode::Char('L') => app.toggle_logo(),
        KeyCode::Char('N') => app.begin_new_file_prompt(),
        KeyCode::Char('w') if modifiers == KeyModifiers::NONE => app.save_current(),
        KeyCode::Char('q') | KeyCode::Char('Q') => app.request_quit(),
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => app.request_quit(),
        _ => {}
    }
}

fn discover_json_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_json_files(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_json_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
            // Skip directories we cannot read instead of failing the whole app.
            return Ok(());
        }
        Err(err) => return Err(err.into()),
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) if err.kind() == io::ErrorKind::PermissionDenied => continue,
            Err(err) => return Err(err.into()),
        };

        let path = entry.path();

        if path.is_dir() {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if matches!(name, "target" | ".git" | "archive") || name.starts_with('.') {
                continue;
            }
            collect_json_files(&path, out)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            out.push(path);
        }
    }

    Ok(())
}

fn flatten_rows(root: &Value, collapsed_paths: &HashSet<NodePath>) -> Vec<NodeRow> {
    fn walk(
        value: &Value,
        path: &NodePath,
        depth: usize,
        key_label: Option<String>,
        key_editable: bool,
        collapsed_paths: &HashSet<NodePath>,
        rows: &mut Vec<NodeRow>,
    ) {
        rows.push(NodeRow {
            kind: RowKind::Value,
            path: path.clone(),
            depth,
            key_label,
            key_editable,
        });

        if collapsed_paths.contains(path) {
            return;
        }

        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    let mut child_path = path.clone();
                    child_path.push(PathSegment::Key(key.clone()));
                    walk(
                        child,
                        &child_path,
                        depth + 1,
                        Some(key.clone()),
                        true,
                        collapsed_paths,
                        rows,
                    );
                }

                rows.push(NodeRow {
                    kind: RowKind::AddKeyAction {
                        object_path: path.clone(),
                    },
                    path: path.clone(),
                    depth: depth + 1,
                    key_label: None,
                    key_editable: false,
                });
            }
            Value::Array(items) => {
                for (idx, child) in items.iter().enumerate() {
                    let mut child_path = path.clone();
                    child_path.push(PathSegment::Index(idx));
                    walk(
                        child,
                        &child_path,
                        depth + 1,
                        Some(format!("[{idx}]")),
                        false,
                        collapsed_paths,
                        rows,
                    );
                }

                rows.push(NodeRow {
                    kind: RowKind::AddItemAction {
                        array_path: path.clone(),
                    },
                    path: path.clone(),
                    depth: depth + 1,
                    key_label: None,
                    key_editable: false,
                });
            }
            _ => {}
        }
    }

    let mut rows = Vec::new();
    walk(
        root,
        &Vec::new(),
        0,
        None,
        false,
        collapsed_paths,
        &mut rows,
    );
    rows
}

fn find_best_row_index(rows: &[NodeRow], path: &NodePath) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }

    if let Some(idx) = rows.iter().position(|row| &row.path == path) {
        return Some(idx);
    }

    for truncate in (0..path.len()).rev() {
        let candidate = &path[..truncate];
        if let Some(idx) = rows.iter().position(|row| row.path.as_slice() == candidate) {
            return Some(idx);
        }
    }

    Some(0)
}

fn value_at_path<'a>(root: &'a Value, path: &[PathSegment]) -> Option<&'a Value> {
    let mut current = root;
    for segment in path {
        current = match (segment, current) {
            (PathSegment::Key(key), Value::Object(map)) => map.get(key)?,
            (PathSegment::Index(idx), Value::Array(items)) => items.get(*idx)?,
            _ => return None,
        };
    }
    Some(current)
}

fn value_at_path_mut<'a>(root: &'a mut Value, path: &[PathSegment]) -> Option<&'a mut Value> {
    let mut current = root;
    for segment in path {
        current = match (segment, current) {
            (PathSegment::Key(key), Value::Object(map)) => map.get_mut(key)?,
            (PathSegment::Index(idx), Value::Array(items)) => items.get_mut(*idx)?,
            _ => return None,
        };
    }
    Some(current)
}

fn split_parent_key(path: &[PathSegment]) -> Option<(NodePath, String)> {
    let mut parent = path.to_vec();
    match parent.pop() {
        Some(PathSegment::Key(key)) => Some((parent, key)),
        _ => None,
    }
}

fn parent_path(path: &[PathSegment]) -> Option<NodePath> {
    if path.is_empty() {
        None
    } else {
        Some(path[..path.len() - 1].to_vec())
    }
}

fn path_starts_with(path: &[PathSegment], prefix: &[PathSegment]) -> bool {
    path.len() >= prefix.len() && path[..prefix.len()] == *prefix
}

fn wrapped_step(current: usize, len: usize, delta: isize) -> usize {
    if len <= 1 || delta == 0 {
        return current.min(len.saturating_sub(1));
    }

    if delta > 0 {
        (current + 1) % len
    } else {
        (current + len - 1) % len
    }
}

fn row_supports_key_target(row: &NodeRow) -> bool {
    matches!(row.kind, RowKind::Value) && row.key_label.is_some()
}

fn is_reserved_global_char(ch: char) -> bool {
    matches!(ch, 'w' | 'W' | 'q' | 'Q' | 'N')
}

fn is_option_backspace(modifiers: KeyModifiers) -> bool {
    modifiers.intersects(KeyModifiers::ALT | KeyModifiers::META)
}

fn is_non_edit_delete_shortcut(code: KeyCode, modifiers: KeyModifiers) -> bool {
    matches!(code, KeyCode::Backspace) && modifiers == KeyModifiers::NONE
        || matches!(code, KeyCode::Backspace | KeyCode::Delete) && is_option_backspace(modifiers)
        || matches!(code, KeyCode::Char('w') | KeyCode::Char('W'))
            && modifiers.contains(KeyModifiers::CONTROL)
}

fn is_duplicate_key_error(message: &str) -> bool {
    message.contains("already exists") || message.contains("duplicate key")
}

fn is_copy_shortcut(code: KeyCode, modifiers: KeyModifiers) -> bool {
    matches!(code, KeyCode::Char('c') | KeyCode::Char('C'))
        && (modifiers.intersects(KeyModifiers::SUPER | KeyModifiers::META)
            || (modifiers.contains(KeyModifiers::CONTROL)
                && modifiers.contains(KeyModifiers::SHIFT)))
}

fn is_paste_shortcut(code: KeyCode, modifiers: KeyModifiers) -> bool {
    matches!(code, KeyCode::Char('v') | KeyCode::Char('V'))
        && modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER | KeyModifiers::META)
        || matches!(code, KeyCode::Char('\u{16}'))
        || matches!(code, KeyCode::Insert) && modifiers.contains(KeyModifiers::SHIFT)
}

fn parse_object_payload(input: &str) -> Option<Map<String, Value>> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    let parsed: Value = serde_json::from_str(trimmed).ok()?;
    match parsed {
        Value::Object(map) if !map.is_empty() => Some(map),
        _ => None,
    }
}

fn value_to_clipboard_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| value_preview(value, false)),
    }
}

fn row_payload_to_clipboard_text(key: &str, value: &Value) -> String {
    let mut map = Map::new();
    map.insert(key.to_string(), value.clone());
    serde_json::to_string(&Value::Object(map)).unwrap_or_else(|_| "{}".to_string())
}

fn normalize_clipboard_text(input: &str) -> String {
    input.trim_end_matches(['\n', '\r']).to_string()
}

fn parse_value_input(input: &str, mode: ValueEditMode) -> Value {
    match mode {
        ValueEditMode::QuotedString => Value::String(input.to_string()),
        ValueEditMode::RawLiteral => parse_user_value_literal(input),
    }
}

fn convert_value_type(current: &Value, target_type: JsonType) -> Result<Value> {
    if JsonType::from_value(current) == target_type {
        return Ok(current.clone());
    }

    match target_type {
        JsonType::String => Ok(Value::String(match current {
            Value::String(s) => s.clone(),
            _ => serde_json::to_string(current)?,
        })),
        JsonType::Number => match current {
            Value::Number(num) => Ok(Value::Number(num.clone())),
            Value::String(s) => parse_json_number(s.trim())
                .map(Value::Number)
                .ok_or_else(|| anyhow!("cannot parse '{}' as number", s)),
            Value::Bool(flag) => Ok(Value::Number(Number::from(if *flag { 1 } else { 0 }))),
            Value::Null => Ok(Value::Number(Number::from(0))),
            Value::Object(_) | Value::Array(_) => {
                bail!("cannot convert object/array directly to number")
            }
        },
        JsonType::Bool => match current {
            Value::Bool(flag) => Ok(Value::Bool(*flag)),
            Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "1" => Ok(Value::Bool(true)),
                "false" | "0" => Ok(Value::Bool(false)),
                _ => bail!("cannot parse '{}' as bool", s),
            },
            Value::Number(num) => {
                if let Some(i) = num.as_i64() {
                    Ok(Value::Bool(i != 0))
                } else if let Some(u) = num.as_u64() {
                    Ok(Value::Bool(u != 0))
                } else if let Some(f) = num.as_f64() {
                    Ok(Value::Bool(f != 0.0))
                } else {
                    bail!("cannot convert number to bool")
                }
            }
            Value::Null => Ok(Value::Bool(false)),
            Value::Object(_) | Value::Array(_) => {
                bail!("cannot convert object/array directly to bool")
            }
        },
        JsonType::Null => Ok(Value::Null),
        JsonType::Object => Ok(Value::Object(match current {
            Value::Object(map) => map.clone(),
            _ => Map::new(),
        })),
        JsonType::Array => Ok(Value::Array(match current {
            Value::Array(items) => items.clone(),
            _ => Vec::new(),
        })),
    }
}

fn parse_json_number(input: &str) -> Option<Number> {
    if let Ok(v) = input.parse::<i64>() {
        return Some(Number::from(v));
    }
    if let Ok(v) = input.parse::<u64>() {
        return Some(Number::from(v));
    }
    if let Ok(v) = input.parse::<f64>() {
        return Number::from_f64(v);
    }
    None
}

fn parse_user_value_literal(input: &str) -> Value {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Value::String(String::new());
    }

    serde_json::from_str(trimmed).unwrap_or_else(|_| Value::String(input.to_string()))
}

fn value_to_edit_text(value: &Value) -> (String, ValueEditMode) {
    match value {
        Value::String(s) => (s.clone(), ValueEditMode::QuotedString),
        Value::Null => (String::new(), ValueEditMode::RawLiteral),
        _ => (
            serde_json::to_string(value).unwrap_or_else(|_| value_preview(value, false)),
            ValueEditMode::RawLiteral,
        ),
    }
}

fn char_to_byte_index(input: &str, cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }

    input
        .char_indices()
        .nth(cursor)
        .map(|(idx, _)| idx)
        .unwrap_or(input.len())
}

fn insert_char_at_cursor(input: &mut String, cursor: &mut usize, ch: char) {
    let len = input.chars().count();
    *cursor = (*cursor).min(len);
    let idx = char_to_byte_index(input, *cursor);
    input.insert(idx, ch);
    *cursor += 1;
}

fn insert_text_at_cursor(input: &mut String, cursor: &mut usize, text: &str) {
    for ch in text.chars() {
        insert_char_at_cursor(input, cursor, ch);
    }
}

fn delete_char_before_cursor(input: &mut String, cursor: &mut usize) {
    let len = input.chars().count();
    *cursor = (*cursor).min(len);
    if *cursor == 0 {
        return;
    }

    let start = char_to_byte_index(input, *cursor - 1);
    let end = char_to_byte_index(input, *cursor);
    input.replace_range(start..end, "");
    *cursor -= 1;
}

fn delete_word_before_cursor(input: &mut String, cursor: &mut usize) {
    let len = input.chars().count();
    *cursor = (*cursor).min(len);
    if *cursor == 0 {
        return;
    }

    let chars: Vec<char> = input.chars().collect();
    let mut start = *cursor;

    while start > 0 && chars[start - 1].is_whitespace() {
        start -= 1;
    }

    if start > 0 {
        let word_group = is_word_char(chars[start - 1]);
        while start > 0 {
            let prev = chars[start - 1];
            if prev.is_whitespace() || is_word_char(prev) != word_group {
                break;
            }
            start -= 1;
        }
    }

    let byte_start = char_to_byte_index(input, start);
    let byte_end = char_to_byte_index(input, *cursor);
    input.replace_range(byte_start..byte_end, "");
    *cursor = start;
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn delete_char_at_cursor(input: &mut String, cursor: &mut usize) {
    let len = input.chars().count();
    *cursor = (*cursor).min(len);
    if *cursor >= len {
        return;
    }

    let start = char_to_byte_index(input, *cursor);
    let end = char_to_byte_index(input, *cursor + 1);
    input.replace_range(start..end, "");
}

fn sanitize_for_terminal(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn with_caret(input: &str, cursor: usize) -> String {
    let idx = char_to_byte_index(input, cursor.min(input.chars().count()));
    let (before, after) = input.split_at(idx);
    format!(
        "{}{}{}",
        sanitize_for_terminal(before),
        CARET,
        sanitize_for_terminal(after)
    )
}

fn edit_input_to_display(input: &str, mode: ValueEditMode, cursor: usize) -> String {
    match mode {
        ValueEditMode::QuotedString => format!("\"{}\"", with_caret(input, cursor)),
        ValueEditMode::RawLiteral => with_caret(input, cursor),
    }
}

fn value_preview(value: &Value, collapsed: bool) -> String {
    match value {
        Value::String(s) => format!("\"{}\"", truncate(&sanitize_for_terminal(s), 64)),
        Value::Number(n) => n.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Null => "null".to_string(),
        Value::Object(map) => {
            let marker = if collapsed { "▸" } else { "▾" };
            format!("{marker} {{…}} ({} keys)", map.len())
        }
        Value::Array(items) => {
            let marker = if collapsed { "▸" } else { "▾" };
            format!("{marker} […] ({} items)", items.len())
        }
    }
}

fn truncate(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }

    let mut out = input.chars().take(max_chars).collect::<String>();
    out.push('…');
    out
}

fn value_style(value: &Value) -> Style {
    match value {
        Value::String(_) => Style::default().fg(theme::STRING_COLOR),
        Value::Number(_) => Style::default().fg(theme::NUMBER_COLOR),
        Value::Bool(_) => Style::default().fg(theme::BOOL_COLOR),
        Value::Null => Style::default().fg(theme::PUNCT),
        Value::Object(_) | Value::Array(_) => Style::default().fg(theme::PUNCT),
    }
}

fn path_to_string(path: &[PathSegment]) -> String {
    if path.is_empty() {
        return "root".to_string();
    }

    let mut out = String::from("root");
    for segment in path {
        match segment {
            PathSegment::Key(key) => {
                out.push('.');
                out.push_str(&sanitize_for_terminal(key));
            }
            PathSegment::Index(idx) => {
                out.push('[');
                out.push_str(&idx.to_string());
                out.push(']');
            }
        }
    }
    out
}

fn display_path(path: &Path) -> String {
    if let Ok(cwd) = env::current_dir()
        && let Ok(rel) = path.strip_prefix(cwd)
    {
        return rel.display().to_string();
    }

    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::tempdir;

    fn fixture_files(entries: &[(&str, &str)]) -> Result<(tempfile::TempDir, Vec<PathBuf>)> {
        let dir = tempdir()?;
        let mut files = Vec::new();

        for (name, content) in entries {
            let path = dir.path().join(name);
            fs::write(&path, content)?;
            files.push(path);
        }

        Ok((dir, files))
    }

    fn unique_env_key(prefix: &str) -> String {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}_{id}")
    }

    fn path_key(name: &str) -> NodePath {
        vec![PathSegment::Key(name.to_string())]
    }

    fn value_row_index(app: &App, path: &NodePath) -> Option<usize> {
        app.rows
            .iter()
            .position(|row| matches!(row.kind, RowKind::Value) && row.path == *path)
    }

    fn add_key_row_index(app: &App, object_path: &NodePath) -> Option<usize> {
        app.rows.iter().position(|row| {
            matches!(
                &row.kind,
                RowKind::AddKeyAction {
                    object_path: candidate,
                } if candidate == object_path
            )
        })
    }

    fn add_item_row_index(app: &App, array_path: &NodePath) -> Option<usize> {
        app.rows.iter().position(|row| {
            matches!(
                &row.kind,
                RowKind::AddItemAction {
                    array_path: candidate,
                } if candidate == array_path
            )
        })
    }

    #[test]
    fn dotenv_assignments_apply_env_local_precedence() -> Result<()> {
        let dir = tempdir()?;
        let key = unique_env_key("STEWS_DOTENV_ORDER");

        fs::write(dir.path().join(".env"), format!("{key}=from_env\n"))?;
        fs::write(
            dir.path().join(".env.local"),
            format!("{key}=from_env_local\n"),
        )?;

        let assignments = collect_dotenv_assignments(dir.path(), &HashSet::new())?;
        let final_values = assignments.into_iter().fold(
            std::collections::HashMap::new(),
            |mut acc, (assignment_key, assignment_value)| {
                acc.insert(assignment_key, assignment_value);
                acc
            },
        );

        assert_eq!(final_values.get(&key), Some(&"from_env_local".to_string()));
        Ok(())
    }

    #[test]
    fn dotenv_assignments_skip_existing_shell_keys() -> Result<()> {
        let dir = tempdir()?;
        let key = unique_env_key("STEWS_DOTENV_SHELL");

        fs::write(dir.path().join(".env"), format!("{key}=from_env\n"))?;
        fs::write(
            dir.path().join(".env.local"),
            format!("{key}=from_env_local\n"),
        )?;

        let mut shell_keys = HashSet::new();
        shell_keys.insert(key.clone());

        let assignments = collect_dotenv_assignments(dir.path(), &shell_keys)?;
        assert!(
            assignments
                .iter()
                .all(|(assignment_key, _)| assignment_key != &key)
        );

        Ok(())
    }

    #[test]
    fn dotenv_driven_root_config_resolves_relative_and_local_override() -> Result<()> {
        let dir = tempdir()?;

        fs::write(
            dir.path().join(".env"),
            format!("{JSON_ROOT_ENV_VAR}=./from-dotenv\n"),
        )?;
        fs::write(
            dir.path().join(".env.local"),
            format!("{JSON_ROOT_ENV_VAR}=./from-dotenv-local\n"),
        )?;

        let assignments = collect_dotenv_assignments(dir.path(), &HashSet::new())?;
        let resolved_values = assignments.into_iter().fold(
            std::collections::HashMap::new(),
            |mut acc, (key, value)| {
                acc.insert(key, value);
                acc
            },
        );

        let resolved_root = resolve_discovery_root(
            dir.path(),
            resolved_values.get(JSON_ROOT_ENV_VAR).map(String::as_str),
        );

        assert_eq!(resolved_root, dir.path().join("from-dotenv-local"));

        Ok(())
    }

    #[test]
    fn logo_toggle_key_updates_visibility() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"alpha":"x"}"#)])?;
        let mut app = App::from_files(files)?;

        assert!(app.logo_visible);
        handle_key(&mut app, KeyCode::Char('L'), KeyModifiers::NONE);
        assert!(!app.logo_visible);
        handle_key(&mut app, KeyCode::Char('l'), KeyModifiers::NONE);
        assert!(app.logo_visible);

        Ok(())
    }

    #[test]
    fn key_value_focus_switching_behaves_as_expected() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"alpha":"x","beta":"y"}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        let alpha_idx = value_row_index(&app, &path_key("alpha"))
            .ok_or_else(|| anyhow!("alpha row missing"))?;

        app.selected_row = alpha_idx;
        app.edit_target = EditTarget::Value;

        app.move_left();
        assert_eq!(app.edit_target, EditTarget::Key);

        app.move_right();
        assert_eq!(app.edit_target, EditTarget::Value);

        app.move_down();
        assert_eq!(app.edit_target, EditTarget::Value);

        Ok(())
    }

    #[test]
    fn add_key_inserts_new_entry() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"alpha":"x"}"#)])?;
        let mut app = App::from_files(files)?;

        let new_path = app.add_key_at_path(&Vec::new(), "newKey".to_string(), Value::Bool(true))?;
        let added = value_at_path(&app.document.root, &new_path).cloned();

        assert_eq!(added, Some(Value::Bool(true)));
        assert!(app.document.is_dirty());
        Ok(())
    }

    #[test]
    fn change_type_converts_selected_value() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"count":"42"}"#)])?;
        let mut app = App::from_files(files)?;

        let path = path_key("count");
        app.change_value_type_at_path(&path, JsonType::Number)?;

        let converted = value_at_path(&app.document.root, &path)
            .cloned()
            .ok_or_else(|| anyhow!("converted value missing"))?;

        assert_eq!(converted, Value::Number(Number::from(42)));
        Ok(())
    }

    #[test]
    fn file_switch_loads_selected_file_content() -> Result<()> {
        let (_dir, files) = fixture_files(&[
            ("a.json", r#"{"name":"first"}"#),
            ("b.json", r#"{"name":"second"}"#),
        ])?;
        let mut app = App::from_files(files)?;

        let first = value_at_path(&app.document.root, &path_key("name")).cloned();
        assert_eq!(first, Some(Value::String("first".to_string())));

        app.request_file_switch(1);

        let second = value_at_path(&app.document.root, &path_key("name")).cloned();
        assert_eq!(second, Some(Value::String("second".to_string())));
        assert_eq!(app.active_file_idx, 1);

        Ok(())
    }

    #[test]
    fn free_form_raw_value_edit_parses_json_literal_or_falls_back_to_string() {
        assert_eq!(
            parse_value_input("{\"x\":1}", ValueEditMode::RawLiteral),
            serde_json::json!({"x": 1})
        );
        assert_eq!(
            parse_value_input("true", ValueEditMode::RawLiteral),
            Value::Bool(true)
        );
        assert_eq!(
            parse_value_input("12.5", ValueEditMode::RawLiteral),
            serde_json::json!(12.5)
        );
        assert_eq!(
            parse_value_input("not-json", ValueEditMode::RawLiteral),
            Value::String("not-json".to_string())
        );
    }

    #[test]
    fn display_uses_caret_cursor_not_underscore() {
        let displayed = edit_input_to_display("hello", ValueEditMode::QuotedString, 2);
        assert!(displayed.contains(CARET));
        assert!(!displayed.contains('_'));
    }

    #[test]
    fn inline_string_edit_supports_left_right_cursor_movement() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"value":"ace"}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        let idx = value_row_index(&app, &path_key("value"))
            .ok_or_else(|| anyhow!("value row missing"))?;

        app.selected_row = idx;
        app.edit_target = EditTarget::Value;
        app.begin_inline_edit();

        app.handle_edit_key(KeyCode::Left, KeyModifiers::NONE);
        app.handle_edit_key(KeyCode::Left, KeyModifiers::NONE);
        app.handle_edit_key(KeyCode::Char('b'), KeyModifiers::NONE);
        app.handle_edit_key(KeyCode::Enter, KeyModifiers::NONE);

        let updated = value_at_path(&app.document.root, &path_key("value")).cloned();
        assert_eq!(updated, Some(Value::String("abce".to_string())));

        Ok(())
    }

    #[test]
    fn option_backspace_deletes_previous_word_in_inline_key_and_value_edits() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"alpha beta":"hello brave world"}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        let path = path_key("alpha beta");
        let idx = value_row_index(&app, &path).ok_or_else(|| anyhow!("value row missing"))?;
        app.selected_row = idx;

        app.edit_target = EditTarget::Key;
        app.begin_inline_edit();
        app.handle_edit_key(KeyCode::Backspace, KeyModifiers::ALT);
        assert!(matches!(
            app.edit_mode.as_ref(),
            Some(EditState {
                input,
                cursor,
                target: EditTarget::Key,
                ..
            }) if input == "alpha " && *cursor == 6
        ));
        app.handle_edit_key(KeyCode::Esc, KeyModifiers::NONE);

        app.edit_target = EditTarget::Value;
        app.begin_inline_edit();
        app.handle_edit_key(KeyCode::Backspace, KeyModifiers::ALT);
        assert!(matches!(
            app.edit_mode.as_ref(),
            Some(EditState {
                input,
                cursor,
                target: EditTarget::Value,
                ..
            }) if input == "hello brave " && *cursor == 12
        ));

        Ok(())
    }

    #[test]
    fn option_backspace_on_key_target_removes_object_pair() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"a":1,"b":2}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        let b_idx =
            value_row_index(&app, &path_key("b")).ok_or_else(|| anyhow!("b row missing"))?;
        app.selected_row = b_idx;
        app.edit_target = EditTarget::Value;
        app.move_left();
        assert_eq!(app.edit_target, EditTarget::Key);

        handle_key(&mut app, KeyCode::Backspace, KeyModifiers::ALT);

        assert_eq!(value_at_path(&app.document.root, &path_key("b")), None);
        assert_eq!(
            value_at_path(&app.document.root, &path_key("a")),
            Some(&Value::Number(1.into()))
        );
        assert!(app.document.is_dirty());

        Ok(())
    }

    #[test]
    fn option_backspace_on_value_target_resets_only_value_to_null() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"a":1,"b":2}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        let b_idx =
            value_row_index(&app, &path_key("b")).ok_or_else(|| anyhow!("b row missing"))?;
        app.selected_row = b_idx;
        app.edit_target = EditTarget::Value;

        handle_key(&mut app, KeyCode::Backspace, KeyModifiers::ALT);

        assert_eq!(
            value_at_path(&app.document.root, &path_key("b")),
            Some(&Value::Null)
        );
        assert_eq!(
            value_at_path(&app.document.root, &path_key("a")),
            Some(&Value::Number(1.into()))
        );
        assert!(app.document.is_dirty());

        Ok(())
    }

    #[test]
    fn option_backspace_array_key_removes_item_and_value_resets_to_null() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"arr":[1,2,3]}"#)])?;
        let mut app = App::from_files(files.clone())?;
        app.focus_idx = 1;

        let item_path = vec![PathSegment::Key("arr".to_string()), PathSegment::Index(1)];
        let item_idx =
            value_row_index(&app, &item_path).ok_or_else(|| anyhow!("array item row missing"))?;
        app.selected_row = item_idx;
        app.edit_target = EditTarget::Value;
        app.move_left();
        assert_eq!(app.edit_target, EditTarget::Key);

        handle_key(&mut app, KeyCode::Backspace, KeyModifiers::ALT);

        assert_eq!(
            value_at_path(&app.document.root, &path_key("arr")),
            Some(&serde_json::json!([1, 3]))
        );

        let mut app = App::from_files(files)?;
        app.focus_idx = 1;
        let item_idx =
            value_row_index(&app, &item_path).ok_or_else(|| anyhow!("array item row missing"))?;
        app.selected_row = item_idx;
        app.edit_target = EditTarget::Value;

        handle_key(&mut app, KeyCode::Backspace, KeyModifiers::ALT);

        assert_eq!(
            value_at_path(&app.document.root, &path_key("arr")),
            Some(&serde_json::json!([1, null, 3]))
        );

        Ok(())
    }

    #[test]
    fn plain_backspace_on_key_target_deletes_row() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"a":1,"b":2}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        let b_idx =
            value_row_index(&app, &path_key("b")).ok_or_else(|| anyhow!("b row missing"))?;
        app.selected_row = b_idx;
        app.edit_target = EditTarget::Value;
        app.move_left();
        assert_eq!(app.edit_target, EditTarget::Key);

        handle_key(&mut app, KeyCode::Backspace, KeyModifiers::NONE);

        assert_eq!(value_at_path(&app.document.root, &path_key("b")), None);
        assert_eq!(
            value_at_path(&app.document.root, &path_key("a")),
            Some(&Value::Number(1.into()))
        );

        Ok(())
    }

    #[test]
    fn plain_backspace_on_value_target_resets_to_null() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"a":1,"b":2}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        let b_idx =
            value_row_index(&app, &path_key("b")).ok_or_else(|| anyhow!("b row missing"))?;
        app.selected_row = b_idx;
        app.edit_target = EditTarget::Value;

        handle_key(&mut app, KeyCode::Backspace, KeyModifiers::NONE);

        assert_eq!(
            value_at_path(&app.document.root, &path_key("b")),
            Some(&Value::Null)
        );
        assert_eq!(
            value_at_path(&app.document.root, &path_key("a")),
            Some(&Value::Number(1.into()))
        );

        Ok(())
    }

    #[test]
    fn plain_backspace_in_text_inputs_does_not_trigger_row_delete_semantics() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"ab":1}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        let key_path = path_key("ab");
        let key_idx = value_row_index(&app, &key_path).ok_or_else(|| anyhow!("key row missing"))?;
        app.selected_row = key_idx;
        app.edit_target = EditTarget::Value;
        app.move_left();
        assert_eq!(app.edit_target, EditTarget::Key);

        app.begin_inline_edit();
        handle_key(&mut app, KeyCode::Backspace, KeyModifiers::NONE);

        assert!(matches!(
            app.edit_mode.as_ref(),
            Some(EditState {
                input,
                cursor,
                target: EditTarget::Key,
                ..
            }) if input == "a" && *cursor == 1
        ));
        assert_eq!(
            value_at_path(&app.document.root, &key_path),
            Some(&Value::Number(1.into()))
        );

        Ok(())
    }

    #[test]
    fn left_right_parent_container_collapse_and_expand_semantics_work() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"obj":{"k":1},"leaf":2}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        let child_path = vec![
            PathSegment::Key("obj".to_string()),
            PathSegment::Key("k".to_string()),
        ];

        let child_idx =
            value_row_index(&app, &child_path).ok_or_else(|| anyhow!("child row missing"))?;
        app.selected_row = child_idx;
        app.edit_target = EditTarget::Key;

        app.move_left();
        assert!(app.collapsed_paths.contains(&path_key("obj")));
        assert!(value_row_index(&app, &child_path).is_none());

        app.move_right();
        assert!(!app.collapsed_paths.contains(&path_key("obj")));
        assert!(value_row_index(&app, &child_path).is_some());

        let leaf_idx =
            value_row_index(&app, &path_key("leaf")).ok_or_else(|| anyhow!("leaf row missing"))?;
        app.selected_row = leaf_idx;
        app.edit_target = EditTarget::Value;
        app.move_left();
        assert_eq!(app.edit_target, EditTarget::Key);

        Ok(())
    }

    #[test]
    fn add_key_action_row_is_selectable_and_enter_starts_inline_flow() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"obj":{}}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        let obj_path = path_key("obj");
        let add_row = add_key_row_index(&app, &obj_path)
            .ok_or_else(|| anyhow!("add-key row for obj missing"))?;
        app.selected_row = add_row;

        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(matches!(
            app.add_key_mode.as_ref(),
            Some(AddKeyEditState {
                object_path,
                stage: AddKeyStage::Key,
                ..
            }) if object_path == &obj_path
        ));

        for ch in "newKey".chars() {
            handle_key(&mut app, KeyCode::Char(ch), KeyModifiers::NONE);
        }
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Char('1'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        let new_path = vec![
            PathSegment::Key("obj".to_string()),
            PathSegment::Key("newKey".to_string()),
        ];
        assert_eq!(
            value_at_path(&app.document.root, &new_path),
            Some(&Value::Number(1.into()))
        );
        assert!(app.add_key_mode.is_none());

        Ok(())
    }

    #[test]
    fn add_item_action_row_is_present_for_non_empty_and_empty_arrays() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"nonEmpty":[1],"empty":[]}"#)])?;
        let app = App::from_files(files)?;

        let non_empty = path_key("nonEmpty");
        let empty = path_key("empty");

        assert!(add_item_row_index(&app, &non_empty).is_some());
        assert!(add_item_row_index(&app, &empty).is_some());

        Ok(())
    }

    #[test]
    fn enter_on_add_item_row_appends_item_and_updates_selection_and_inspector() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"arr":[1]}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        let array_path = path_key("arr");
        let add_row = add_item_row_index(&app, &array_path)
            .ok_or_else(|| anyhow!("add-item row for arr missing"))?;
        app.selected_row = add_row;

        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        let new_item_path = vec![PathSegment::Key("arr".to_string()), PathSegment::Index(1)];
        assert_eq!(
            value_at_path(&app.document.root, &array_path),
            Some(&serde_json::json!([1, null]))
        );
        assert_eq!(app.current_path(), new_item_path);
        assert_eq!(app.current_value_type(), Some(JsonType::Null));
        assert!(app.status.contains("Added array item [1]"));

        Ok(())
    }

    #[test]
    fn pressing_a_in_array_context_adds_item_to_nearest_array() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"arr":[1]}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        let item_path = vec![PathSegment::Key("arr".to_string()), PathSegment::Index(0)];
        let item_idx =
            value_row_index(&app, &item_path).ok_or_else(|| anyhow!("array item row missing"))?;
        app.selected_row = item_idx;
        app.edit_target = EditTarget::Value;

        handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);

        assert_eq!(
            value_at_path(&app.document.root, &path_key("arr")),
            Some(&serde_json::json!([1, null]))
        );
        assert_eq!(
            app.current_path(),
            vec![PathSegment::Key("arr".to_string()), PathSegment::Index(1)]
        );

        Ok(())
    }

    #[test]
    fn ctrl_w_non_edit_delete_matches_option_delete_semantics_for_object_and_array_rows()
    -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"obj":1,"arr":[1,2,3]}"#)])?;

        let mut object_key_app = App::from_files(files.clone())?;
        object_key_app.focus_idx = 1;
        let obj_idx = value_row_index(&object_key_app, &path_key("obj"))
            .ok_or_else(|| anyhow!("obj row missing"))?;
        object_key_app.selected_row = obj_idx;
        object_key_app.edit_target = EditTarget::Value;
        object_key_app.move_left();
        handle_key(
            &mut object_key_app,
            KeyCode::Char('w'),
            KeyModifiers::CONTROL,
        );
        assert_eq!(
            value_at_path(&object_key_app.document.root, &path_key("obj")),
            None
        );

        let mut object_value_app = App::from_files(files.clone())?;
        object_value_app.focus_idx = 1;
        let obj_idx = value_row_index(&object_value_app, &path_key("obj"))
            .ok_or_else(|| anyhow!("obj row missing"))?;
        object_value_app.selected_row = obj_idx;
        object_value_app.edit_target = EditTarget::Value;
        handle_key(
            &mut object_value_app,
            KeyCode::Char('w'),
            KeyModifiers::CONTROL,
        );
        assert_eq!(
            value_at_path(&object_value_app.document.root, &path_key("obj")),
            Some(&Value::Null)
        );

        let item_path = vec![PathSegment::Key("arr".to_string()), PathSegment::Index(1)];

        let mut array_key_app = App::from_files(files.clone())?;
        array_key_app.focus_idx = 1;
        let item_idx =
            value_row_index(&array_key_app, &item_path).ok_or_else(|| anyhow!("item missing"))?;
        array_key_app.selected_row = item_idx;
        array_key_app.edit_target = EditTarget::Value;
        array_key_app.move_left();
        handle_key(
            &mut array_key_app,
            KeyCode::Char('w'),
            KeyModifiers::CONTROL,
        );
        assert_eq!(
            value_at_path(&array_key_app.document.root, &path_key("arr")),
            Some(&serde_json::json!([1, 3]))
        );

        let mut array_value_app = App::from_files(files)?;
        array_value_app.focus_idx = 1;
        let item_idx =
            value_row_index(&array_value_app, &item_path).ok_or_else(|| anyhow!("item missing"))?;
        array_value_app.selected_row = item_idx;
        array_value_app.edit_target = EditTarget::Value;
        handle_key(
            &mut array_value_app,
            KeyCode::Char('w'),
            KeyModifiers::CONTROL,
        );
        assert_eq!(
            value_at_path(&array_value_app.document.root, &path_key("arr")),
            Some(&serde_json::json!([1, null, 3]))
        );

        Ok(())
    }

    #[test]
    fn pressing_a_adds_in_nearest_object_context() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"obj":{"leaf":1}}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        let leaf_path = vec![
            PathSegment::Key("obj".to_string()),
            PathSegment::Key("leaf".to_string()),
        ];
        let leaf_idx =
            value_row_index(&app, &leaf_path).ok_or_else(|| anyhow!("leaf row missing"))?;
        app.selected_row = leaf_idx;
        app.edit_target = EditTarget::Value;

        handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
        assert!(matches!(
            app.add_key_mode.as_ref(),
            Some(AddKeyEditState {
                object_path,
                stage: AddKeyStage::Key,
                ..
            }) if object_path == &path_key("obj")
        ));

        handle_key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        for ch in "true".chars() {
            handle_key(&mut app, KeyCode::Char(ch), KeyModifiers::NONE);
        }
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        let new_path = vec![
            PathSegment::Key("obj".to_string()),
            PathSegment::Key("x".to_string()),
        ];
        assert_eq!(
            value_at_path(&app.document.root, &new_path),
            Some(&Value::Bool(true))
        );

        Ok(())
    }

    #[test]
    fn save_modal_state_appears_with_filename_and_dismisses() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"alpha":"x"}"#)])?;
        let mut app = App::from_files(files)?;

        handle_key(&mut app, KeyCode::Char('w'), KeyModifiers::NONE);

        let modal_message = app
            .save_modal
            .as_ref()
            .map(|modal| modal.message.clone())
            .ok_or_else(|| anyhow!("save modal should be visible"))?;
        assert!(modal_message.contains("Saved"));
        assert!(modal_message.contains("a.json"));

        handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(app.save_modal.is_none());

        Ok(())
    }

    #[test]
    fn explorer_new_file_flow_handles_dirty_state_and_opens_created_file() -> Result<()> {
        let (dir, files) = fixture_files(&[("a.json", r#"{"name":"first"}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 0;

        app.document.root = serde_json::json!({"name": "dirty"});
        assert!(app.document.is_dirty());

        handle_key(&mut app, KeyCode::Char('N'), KeyModifiers::NONE);
        for ch in "created".chars() {
            handle_key(&mut app, KeyCode::Char(ch), KeyModifiers::NONE);
        }
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        assert!(matches!(
            app.prompt,
            Some(PromptState::DirtyConfirm {
                action: PendingAction::OpenOrCreateFile(_)
            })
        ));

        handle_key(&mut app, KeyCode::Char('d'), KeyModifiers::NONE);

        let expected = dir.path().join("created.json");
        assert!(expected.exists());
        assert_eq!(app.document.path, expected);
        assert_eq!(app.active_file_idx, app.explorer_cursor);

        Ok(())
    }

    #[test]
    fn new_file_dialog_supports_cursor_editing_without_clearing_input() -> Result<()> {
        let (dir, files) = fixture_files(&[("a.json", r#"{"name":"first"}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 0;

        handle_key(&mut app, KeyCode::Char('N'), KeyModifiers::NONE);

        for ch in "crate".chars() {
            handle_key(&mut app, KeyCode::Char(ch), KeyModifiers::NONE);
        }
        handle_key(&mut app, KeyCode::Home, KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Right, KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Right, KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Char('e'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::End, KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Char('e'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Home, KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Delete, KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);

        assert!(matches!(
            app.prompt,
            Some(PromptState::NewFile { ref input, .. }) if input == "create"
        ));

        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        let expected = dir.path().join("create.json");
        assert!(expected.exists());
        assert_eq!(app.document.path, expected);

        Ok(())
    }

    #[test]
    fn object_key_order_is_preserved_after_add_rename_edit_and_save_roundtrip() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"first":1,"second":2}"#)])?;
        let mut app = App::from_files(files)?;

        app.add_key_at_path(&Vec::new(), "third".to_string(), Value::Number(3.into()))?;
        let second_path = path_key("second");
        let renamed_path = app.rename_key_at_path(&second_path, "middle".to_string())?;
        app.apply_value_edit(&renamed_path, "20", ValueEditMode::RawLiteral)?;

        let keys_before_save = app
            .document
            .root
            .as_object()
            .ok_or_else(|| anyhow!("root should be object"))?
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(keys_before_save, vec!["first", "middle", "third"]);

        app.document.save()?;
        let reloaded = Document::load(&app.document.path)?;
        let keys_after_save = reloaded
            .root
            .as_object()
            .ok_or_else(|| anyhow!("reloaded root should be object"))?
            .keys()
            .cloned()
            .collect::<Vec<_>>();

        assert_eq!(keys_after_save, vec!["first", "middle", "third"]);
        assert_eq!(
            value_at_path(&reloaded.root, &path_key("middle")),
            Some(&Value::Number(20.into()))
        );

        Ok(())
    }

    #[test]
    fn wrap_around_navigation_works_for_explorer_and_editor() -> Result<()> {
        let (_dir, files) = fixture_files(&[
            ("a.json", r#"{"root":1}"#),
            ("b.json", r#"{"root":2}"#),
            ("c.json", r#"{"root":3}"#),
        ])?;
        let mut app = App::from_files(files)?;

        app.focus_idx = 0;
        app.explorer_cursor = 0;
        app.active_file_idx = 0;
        app.move_up();
        assert_eq!(app.explorer_cursor, app.files.len() - 1);
        assert_eq!(app.active_file_idx, app.files.len() - 1);

        app.move_down();
        assert_eq!(app.explorer_cursor, 0);
        assert_eq!(app.active_file_idx, 0);

        app.focus_idx = 1;
        app.selected_row = 0;
        app.move_up();
        assert_eq!(app.selected_row, app.rows.len() - 1);

        app.move_down();
        assert_eq!(app.selected_row, 0);

        Ok(())
    }

    #[test]
    fn enter_in_explorer_switches_focus_to_editor() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"alpha":1}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 0;

        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(app.focus(), FocusArea::Editor);
        assert!(app.edit_mode.is_none());

        Ok(())
    }

    #[test]
    fn explorer_filter_updates_and_clearing_to_empty_restores_full_list() -> Result<()> {
        let (_dir, files) = fixture_files(&[
            ("first-item.json", r#"{"v":1}"#),
            ("special-zzmatch.json", r#"{"v":2}"#),
            ("third-item.json", r#"{"v":3}"#),
        ])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 0;

        app.request_file_switch(1);
        assert_eq!(app.explorer_cursor, 1);

        for ch in "zzmatch".chars() {
            handle_key(&mut app, KeyCode::Char(ch), KeyModifiers::NONE);
        }

        assert_eq!(app.explorer_filter, "zzmatch");
        assert_eq!(app.filtered_file_indices(), vec![1]);
        assert_eq!(app.explorer_cursor, 1);

        for _ in 0.."zzmatch".len() {
            handle_key(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
        }

        assert!(app.explorer_filter.is_empty());
        assert_eq!(app.filtered_file_indices().len(), app.files.len());

        Ok(())
    }

    #[test]
    fn ctrl_w_in_edit_mode_deletes_previous_word_and_does_not_save() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"value":"hello brave world"}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        let idx = value_row_index(&app, &path_key("value"))
            .ok_or_else(|| anyhow!("value row missing"))?;
        app.selected_row = idx;
        app.edit_target = EditTarget::Value;
        app.begin_inline_edit();

        handle_key(&mut app, KeyCode::Char('w'), KeyModifiers::CONTROL);

        assert!(matches!(
            app.edit_mode.as_ref(),
            Some(EditState { input, cursor, .. }) if input == "hello brave " && *cursor == 12
        ));
        assert!(app.save_modal.is_none());

        Ok(())
    }

    #[test]
    fn duplicate_key_add_shows_blocking_error_modal() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"obj":{"dup":1}}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        app.begin_add_key_for_object(path_key("obj"));
        for ch in "dup".chars() {
            handle_key(&mut app, KeyCode::Char(ch), KeyModifiers::NONE);
        }
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        let modal = app
            .error_modal
            .as_ref()
            .ok_or_else(|| anyhow!("expected duplicate-key modal"))?;
        assert_eq!(modal.title, "Duplicate key");
        assert!(modal.message.contains("already has a key named \"dup\""));
        assert!(modal.message.contains("Choose a different key name"));

        Ok(())
    }

    #[test]
    fn value_and_key_copy_use_expected_clipboard_payloads() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"text":"hello","num":7}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        let text_idx =
            value_row_index(&app, &path_key("text")).ok_or_else(|| anyhow!("text row missing"))?;
        app.selected_row = text_idx;
        app.edit_target = EditTarget::Value;
        handle_key(
            &mut app,
            KeyCode::Char('c'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(app.clipboard.scratch.as_deref(), Some("hello"));

        let num_idx =
            value_row_index(&app, &path_key("num")).ok_or_else(|| anyhow!("num row missing"))?;
        app.selected_row = num_idx;
        app.edit_target = EditTarget::Value;
        app.move_left();
        assert_eq!(app.edit_target, EditTarget::Key);
        handle_key(
            &mut app,
            KeyCode::Char('c'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(app.clipboard.scratch.as_deref(), Some("{\"num\":7}"));

        Ok(())
    }

    #[test]
    fn ctrl_v_pastes_and_ctrl_c_still_requests_quit() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"alpha":0}"#)])?;
        let mut app = App::from_files(files.clone())?;
        app.focus_idx = 1;

        let alpha_idx = value_row_index(&app, &path_key("alpha"))
            .ok_or_else(|| anyhow!("alpha row missing"))?;
        app.selected_row = alpha_idx;
        app.edit_target = EditTarget::Value;
        app.clipboard.scratch = Some("42".to_string());

        handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::CONTROL);

        assert_eq!(
            value_at_path(&app.document.root, &path_key("alpha")),
            Some(&Value::Number(42.into()))
        );

        let mut quit_app = App::from_files(files)?;
        handle_key(&mut quit_app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(!quit_app.running);

        Ok(())
    }

    #[test]
    fn key_target_paste_supports_row_payload_and_plain_key_text() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"obj":{"a":1}}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        let a_path = vec![
            PathSegment::Key("obj".to_string()),
            PathSegment::Key("a".to_string()),
        ];
        let a_idx = value_row_index(&app, &a_path).ok_or_else(|| anyhow!("a row missing"))?;
        app.selected_row = a_idx;
        app.edit_target = EditTarget::Value;
        app.move_left();

        app.clipboard.scratch = Some("{\"b\":2}".to_string());
        handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::CONTROL);

        assert_eq!(
            value_at_path(&app.document.root, &path_key("obj")),
            Some(&serde_json::json!({"a": 1, "b": 2}))
        );

        let a_idx = value_row_index(&app, &a_path).ok_or_else(|| anyhow!("a row missing"))?;
        app.selected_row = a_idx;
        app.edit_target = EditTarget::Key;

        app.clipboard.scratch = Some("renamedA".to_string());
        handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::CONTROL);

        let renamed = vec![
            PathSegment::Key("obj".to_string()),
            PathSegment::Key("renamedA".to_string()),
        ];
        assert_eq!(
            value_at_path(&app.document.root, &renamed),
            Some(&Value::Number(1.into()))
        );

        app.clipboard.scratch = Some("{\"b\":3}".to_string());
        handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::CONTROL);

        let modal = app
            .error_modal
            .as_ref()
            .ok_or_else(|| anyhow!("expected duplicate paste conflict modal"))?;
        assert_eq!(modal.title, "Paste conflict");
        assert_eq!(
            value_at_path(&app.document.root, &path_key("obj")),
            Some(&serde_json::json!({"renamedA": 1, "b": 2}))
        );

        Ok(())
    }

    #[test]
    fn paste_on_add_action_rows_prefills_key_or_appends_item() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"obj":{},"arr":[]}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        let obj_path = path_key("obj");
        let add_key_idx =
            add_key_row_index(&app, &obj_path).ok_or_else(|| anyhow!("add-key row missing"))?;
        app.selected_row = add_key_idx;
        app.clipboard.scratch = Some("pastedKey".to_string());
        handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::CONTROL);

        assert!(matches!(
            app.add_key_mode.as_ref(),
            Some(AddKeyEditState {
                stage: AddKeyStage::Value,
                key_input,
                ..
            }) if key_input == "pastedKey"
        ));

        handle_key(&mut app, KeyCode::Char('1'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(
            value_at_path(
                &app.document.root,
                &vec![
                    PathSegment::Key("obj".to_string()),
                    PathSegment::Key("pastedKey".to_string())
                ]
            ),
            Some(&Value::Number(1.into()))
        );

        let arr_path = path_key("arr");
        let add_item_idx =
            add_item_row_index(&app, &arr_path).ok_or_else(|| anyhow!("add-item row missing"))?;
        app.selected_row = add_item_idx;
        app.clipboard.scratch = Some("true".to_string());
        handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::CONTROL);

        assert_eq!(
            value_at_path(&app.document.root, &arr_path),
            Some(&serde_json::json!([true]))
        );

        Ok(())
    }

    #[test]
    fn entering_edit_on_null_starts_with_empty_buffer() -> Result<()> {
        let (_dir, files) = fixture_files(&[("a.json", r#"{"n":null}"#)])?;
        let mut app = App::from_files(files)?;
        app.focus_idx = 1;

        let n_idx =
            value_row_index(&app, &path_key("n")).ok_or_else(|| anyhow!("n row missing"))?;
        app.selected_row = n_idx;
        app.edit_target = EditTarget::Value;

        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        assert!(matches!(
            app.edit_mode.as_ref(),
            Some(EditState {
                input,
                cursor,
                target: EditTarget::Value,
                ..
            }) if input.is_empty() && *cursor == 0
        ));

        Ok(())
    }
}
