import unittest

from stews.state import AppState, FocusArea


class TestState(unittest.TestCase):
    def test_focus_cycles_forward_and_backward(self) -> None:
        s = AppState()
        self.assertEqual(s.focus, FocusArea.FILE_TREE)
        s.cycle_focus()
        self.assertEqual(s.focus, FocusArea.EDITOR)
        s.cycle_focus(backward=True)
        self.assertEqual(s.focus, FocusArea.FILE_TREE)

    def test_quit_prompts_when_dirty(self) -> None:
        s = AppState(dirty=True)
        s.request_quit()
        self.assertTrue(s.running)
        self.assertIsNotNone(s.prompt)
        s.resolve_prompt("d")
        self.assertFalse(s.running)

    def test_file_switch_prompts_when_dirty_and_can_cancel(self) -> None:
        s = AppState(dirty=True)
        before = s.current_file
        s.request_file_switch()
        self.assertIsNotNone(s.prompt)
        s.resolve_prompt("c")
        self.assertEqual(s.current_file, before)
        self.assertIsNone(s.prompt)

    def test_file_switch_when_clean_changes_file(self) -> None:
        s = AppState(dirty=False)
        before = s.current_file
        s.request_file_switch()
        self.assertNotEqual(s.current_file, before)


if __name__ == "__main__":
    unittest.main()
