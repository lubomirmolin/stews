from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum


class FocusArea(str, Enum):
    FILE_TREE = "file-tree"
    EDITOR = "editor"
    INSPECTOR = "inspector"
    COMMAND = "command"


@dataclass
class AppState:
    focus_order: list[FocusArea] = field(
        default_factory=lambda: [
            FocusArea.FILE_TREE,
            FocusArea.EDITOR,
            FocusArea.INSPECTOR,
            FocusArea.COMMAND,
        ]
    )
    focus_idx: int = 0
    file_cursor: int = 0
    editor_row: int = 0
    editor_col: int = 0
    inspector_cursor: int = 0
    files: list[str] = field(
        default_factory=lambda: [
            "examples/sample.json",
            "examples/user.json",
            "examples/settings.json",
        ]
    )
    current_file: str = "examples/sample.json"
    dirty: bool = False
    pending_action: str | None = None
    prompt: str | None = None
    running: bool = True

    @property
    def focus(self) -> FocusArea:
        return self.focus_order[self.focus_idx]

    def cycle_focus(self, backward: bool = False) -> None:
        if backward:
            self.focus_idx = (self.focus_idx - 1) % len(self.focus_order)
        else:
            self.focus_idx = (self.focus_idx + 1) % len(self.focus_order)

    def move_up(self) -> None:
        if self.focus == FocusArea.FILE_TREE:
            self.file_cursor = max(0, self.file_cursor - 1)
        elif self.focus == FocusArea.EDITOR:
            self.editor_row = max(0, self.editor_row - 1)
        elif self.focus == FocusArea.INSPECTOR:
            self.inspector_cursor = max(0, self.inspector_cursor - 1)

    def move_down(self) -> None:
        if self.focus == FocusArea.FILE_TREE:
            self.file_cursor = min(len(self.files) - 1, self.file_cursor + 1)
        elif self.focus == FocusArea.EDITOR:
            self.editor_row = min(200, self.editor_row + 1)
        elif self.focus == FocusArea.INSPECTOR:
            self.inspector_cursor = min(50, self.inspector_cursor + 1)

    def move_left(self) -> None:
        if self.focus == FocusArea.EDITOR:
            self.editor_col = max(0, self.editor_col - 1)

    def move_right(self) -> None:
        if self.focus == FocusArea.EDITOR:
            self.editor_col = min(200, self.editor_col + 1)

    def request_quit(self) -> None:
        if self.dirty:
            self.pending_action = "quit"
            self.prompt = "Unsaved changes. Save before quit? [s]ave / [d]iscard / [c]ancel"
            return
        self.running = False

    def request_file_switch(self) -> None:
        if self.dirty:
            self.pending_action = "switch-file"
            self.prompt = "Unsaved changes. Save before switching file? [s]ave / [d]iscard / [c]ancel"
            return
        self._do_switch_file()

    def resolve_prompt(self, key: str) -> None:
        if not self.prompt:
            return
        if key.lower() == "c":
            self.prompt = None
            self.pending_action = None
            return
        if key.lower() == "s":
            self.dirty = False
            self.prompt = None
            self._apply_pending_action()
            return
        if key.lower() == "d":
            self.dirty = False
            self.prompt = None
            self._apply_pending_action()

    def mark_dirty(self) -> None:
        self.dirty = True

    def delete_prev_word(self) -> None:
        # Placeholder behavior for macOS Option+Backspace (⌥⌫).
        # Real text buffer integration lands in later slices.
        self.mark_dirty()

    def _apply_pending_action(self) -> None:
        action = self.pending_action
        self.pending_action = None
        if action == "quit":
            self.running = False
        elif action == "switch-file":
            self._do_switch_file()

    def _do_switch_file(self) -> None:
        self.file_cursor = (self.file_cursor + 1) % len(self.files)
        self.current_file = self.files[self.file_cursor]
