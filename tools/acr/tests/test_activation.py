from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from acr.core.activation import ActivationError, DeterministicActivationPolicy


class ActivationTests(unittest.TestCase):
    def setUp(self):
        self.policy = DeterministicActivationPolicy()
        self.general = ("maintainability", "development", "security")

    def test_general_personas_always_run_for_nonempty_targets(self):
        for path in ("notes.txt", "LICENSE", "image.png", "kernel/src/foo.rs"):
            with self.subTest(path=path):
                self.assertEqual(self.policy.for_paths([path]), self.general)

    def test_documentation_is_added_by_path(self):
        for path in ("README.md", "compat.scml", "book/notes.txt", "kernel/src/syscall/read.rs"):
            with self.subTest(path=path):
                self.assertEqual(self.policy.for_paths([path]), self.general + ("documentation",))

    def test_hardware_is_added_by_path(self):
        for path in ("trap.S", "trap.s", "trap.asm", "arch/entry.rs", "ostd/src/arch/x86/trap.rs"):
            with self.subTest(path=path):
                self.assertEqual(self.policy.for_paths([path]), self.general + ("hardware",))

    def test_selection_ignores_inline_assembly_in_file_contents(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "inline.rs").write_text("asm!(); global_asm!();", encoding="utf-8")
            self.assertEqual(self.policy.for_paths(["inline.rs"], repo_root=root), self.general)

    def test_mixed_targets_preserve_persona_order_without_duplicates(self):
        self.assertEqual(
            self.policy.for_paths(["README.md", "trap.S", "notes.txt", "README.md"]),
            self.general + ("hardware", "documentation"),
        )

    def test_empty_target_list_is_rejected(self):
        with self.assertRaises(ActivationError):
            self.policy.for_paths([])
