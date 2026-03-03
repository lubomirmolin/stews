from __future__ import annotations

import curses
from curses import ascii

from .state import AppState, FocusArea

HELP = "Tab/Shift+Tab focus • Arrows navigate • Ctrl+P switch file • ⌥⌫ delete prev word • q quit"


def _safe_addstr(win: curses.window, y: int, x: int, text: str, attr: int = 0) -> None:
    h, w = win.getmaxyx()
    if y < 0 or y >= h:
        return
    trimmed = text[: max(0, w - x - 1)]
    if not trimmed:
        return
    try:
        win.addstr(y, x, trimmed, attr)
    except curses.error:
        pass


def _draw_box(win: curses.window, title: str, focused: bool) -> None:
    attr = curses.A_BOLD if focused else curses.A_NORMAL
    win.box()
    _safe_addstr(win, 0, 2, f" {title} ", attr)


def render(stdscr: curses.window, state: AppState) -> None:
    stdscr.erase()
    h, w = stdscr.getmaxyx()

    body_h = h - 3
    tree_w = max(24, w // 4)
    inspector_w = max(24, w // 4)
    editor_w = max(20, w - tree_w - inspector_w)

    tree = stdscr.subwin(body_h, tree_w, 0, 0)
    editor = stdscr.subwin(body_h, editor_w, 0, tree_w)
    inspector = stdscr.subwin(body_h, inspector_w, 0, tree_w + editor_w)

    _draw_box(tree, "Files", state.focus == FocusArea.FILE_TREE)
    _draw_box(editor, "Editor", state.focus == FocusArea.EDITOR)
    _draw_box(inspector, "Inspector", state.focus == FocusArea.INSPECTOR)

    for idx, f in enumerate(state.files[: body_h - 2]):
        marker = ">" if idx == state.file_cursor else " "
        _safe_addstr(tree, idx + 1, 1, f"{marker} {f}")

    editor_lines = [
        "{",
        '  "stews": {',
        '    "status": "skeleton",',
        f'    "focusedPane": "{state.focus.value}",',
        f'    "cursor": {{"row": {state.editor_row}, "col": {state.editor_col}}}',
        "  }",
        "}",
    ]
    for i, line in enumerate(editor_lines[: body_h - 2]):
        _safe_addstr(editor, i + 1, 1, line)

    inspector_lines = [
        f"Current file: {state.current_file}",
        f"Dirty: {'yes' if state.dirty else 'no'}",
        "Schema: (placeholder)",
        "Node type: object",
        "Validation: pending",
        "",
        "Arrows: nav/collapse scaffold",
    ]
    for i, line in enumerate(inspector_lines[: body_h - 2]):
        _safe_addstr(inspector, i + 1, 1, line)

    status = stdscr.subwin(1, w, h - 3, 0)
    cmd = stdscr.subwin(1, w, h - 2, 0)
    prompt = stdscr.subwin(1, w, h - 1, 0)

    status_txt = f"file={state.current_file} focus={state.focus.value} dirty={'*' if state.dirty else '-'}"
    _safe_addstr(status, 0, 0, status_txt, curses.A_REVERSE)
    _safe_addstr(cmd, 0, 0, HELP)
    _safe_addstr(prompt, 0, 0, state.prompt or "Ready")

    stdscr.refresh()


def _handle_key(ch: int, state: AppState) -> None:
    if state.prompt and ch in (ord("s"), ord("d"), ord("c"), ord("S"), ord("D"), ord("C")):
        state.resolve_prompt(chr(ch))
        return

    if ch == curses.KEY_BTAB:
        state.cycle_focus(backward=True)
    elif ch == ascii.TAB:
        state.cycle_focus()
    elif ch == curses.KEY_UP:
        state.move_up()
    elif ch == curses.KEY_DOWN:
        state.move_down()
    elif ch == curses.KEY_LEFT:
        state.move_left()
    elif ch == curses.KEY_RIGHT:
        state.move_right()
    elif ch == ascii.DLE:  # Ctrl+P
        state.request_file_switch()
    elif ch in (ord("q"), ord("Q")):
        state.request_quit()
    elif ch in (ord("i"), ord("I")):
        state.mark_dirty()
    elif ch == curses.KEY_BACKSPACE or ch == 127:
        # Plain backspace; future slice will distinguish char-vs-word deletes.
        state.mark_dirty()
    elif ch == 23:  # Ctrl+W fallback for delete previous word in many terminals.
        state.delete_prev_word()


def run() -> int:
    state = AppState()

    def _inner(stdscr: curses.window) -> None:
        curses.curs_set(0)
        stdscr.nodelay(False)
        stdscr.keypad(True)

        while state.running:
            render(stdscr, state)
            ch = stdscr.getch()
            _handle_key(ch, state)

    curses.wrapper(_inner)
    return 0
