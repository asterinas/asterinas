from __future__ import annotations

import io
import json
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

from acr.benchmark.grader import GraderError, calculate_recall
from acr.benchmark.parse_grade import normalize
from acr.benchmark.runner import (
    BenchmarkError,
    BenchmarkProblem,
    benchmark_run_prefix,
    load_problems,
    overlay_package,
    print_recall,
)
from acr.core.contracts import GraderEnvelope


def benchmark_defect(path: str = "foo.rs", **overrides):
    value = {
        "target": {"kind": "file", "path": path},
        "persona": "development",
        "grounding": "checked-arithmetic",
        "severity": "major",
        "desc": "A benchmark defect.",
        "fix": "Fix the defect.",
        "expectation": f"find the defect in {path}",
    }
    value.update(overrides)
    return value


def benchmark_problem(problem_id: str, **overrides):
    value = {
        "problem_id": problem_id,
        "source": "Test fixture.",
        "commit": "HEAD",
        "review_mode": {"files": ["README.md"]},
        "defects": [benchmark_defect()],
    }
    value.update(overrides)
    return value


class GradeTests(unittest.TestCase):
    def test_benchmark_run_prefix_uses_numeric_problem_id_without_timezone(self):
        problem = BenchmarkProblem.from_mapping(benchmark_problem("0001-example"))
        self.assertEqual(
            benchmark_run_prefix("20260903T164010+0800", [problem]),
            "run-20260903T164010-0001-",
        )

    def test_benchmark_run_prefix_labels_multiple_problems_compactly(self):
        problems = [
            BenchmarkProblem.from_mapping(benchmark_problem("0001-first-example")),
            BenchmarkProblem.from_mapping(benchmark_problem("0200-second-example")),
        ]
        self.assertEqual(
            benchmark_run_prefix("20260903T164010+0800", problems),
            "run-20260903T164010-multi-0001_0200-",
        )

    def test_diff_problem_uses_configured_remote(self):
        configured_remote = "https://github.com/asterinas/asterinas"
        problem = BenchmarkProblem.from_mapping(
            benchmark_problem(
                "0100-test",
                commit="a" * 40,
                review_mode={"diff": {"base": "HEAD^"}},
            ),
            default_remote=configured_remote,
        )
        self.assertEqual(problem.remote, configured_remote)

        custom = BenchmarkProblem.from_mapping(
            benchmark_problem(
                "0101-test",
                commit="b" * 40,
                remote="https://example.invalid/fork",
                review_mode={"diff": {"base": "HEAD^"}},
            )
        )
        self.assertEqual(custom.remote, "https://example.invalid/fork")

    def test_diff_problem_requires_configured_remote(self):
        with self.assertRaisesRegex(BenchmarkError, "benchmark.remote"):
            BenchmarkProblem.from_mapping(
                benchmark_problem(
                    "0102-test",
                    commit="c" * 40,
                    review_mode={"diff": {"base": "HEAD^"}},
                )
            )

    def test_files_problem_remains_local_only_without_remote(self):
        problem = BenchmarkProblem.from_mapping(
            benchmark_problem(
                "0102-test",
                commit="HEAD^",
                review_mode={"files": ["README.md"]},
            )
        )
        self.assertIsNone(problem.remote)

    def test_problem_schema_preserves_optional_defect_fields(self):
        negative = benchmark_defect(
            target={"kind": "whole_change"},
            is_negative=True,
        )
        negative.pop("fix")
        problem = BenchmarkProblem.from_mapping(
            benchmark_problem(
                "0103-test",
                defects=[benchmark_defect(), negative],
            )
        )
        self.assertFalse(problem.defects[0].is_negative)
        self.assertIsNone(problem.defects[1].target.path)
        self.assertIsNone(problem.defects[1].fix)

    def test_problem_schema_rejects_invalid_review_and_defect_fields(self):
        invalid_values = (
            ("problem id", benchmark_problem("invalid-id")),
            ("source", benchmark_problem("0104-test", source="")),
            ("commit", benchmark_problem("0104-test", commit="")),
            ("obsolete base", benchmark_problem("0104-test", base_commit="HEAD^")),
            ("remote", benchmark_problem("0104-test", remote="local-path")),
            (
                "review mode cardinality",
                benchmark_problem("0104-test", review_mode={"diff": {}, "files": []}),
            ),
            (
                "diff SHA",
                benchmark_problem(
                    "0104-test", review_mode={"diff": {"base": "HEAD^"}}
                ),
            ),
            (
                "diff base",
                benchmark_problem(
                    "0104-test",
                    commit="a" * 40,
                    review_mode={"diff": {"base": ""}},
                ),
            ),
            (
                "diff extra key",
                benchmark_problem(
                    "0104-test",
                    commit="a" * 40,
                    review_mode={"diff": {"base": "HEAD^", "extra": True}},
                ),
            ),
            ("files", benchmark_problem("0104-test", review_mode={"files": []})),
            ("defects", benchmark_problem("0104-test", defects=[])),
            (
                "target path required",
                benchmark_problem(
                    "0104-test",
                    defects=[benchmark_defect(target={"kind": "file"})],
                ),
            ),
            (
                "target path forbidden",
                benchmark_problem(
                    "0104-test",
                    defects=[
                        benchmark_defect(
                            target={"kind": "whole_change", "path": "foo.rs"}
                        )
                    ],
                ),
            ),
            (
                "persona",
                benchmark_problem(
                    "0104-test",
                    defects=[benchmark_defect(persona="unknown")],
                ),
            ),
            (
                "grounding",
                benchmark_problem(
                    "0104-test",
                    defects=[benchmark_defect(grounding="")],
                ),
            ),
            (
                "severity",
                benchmark_problem(
                    "0104-test",
                    defects=[benchmark_defect(severity="unknown")],
                ),
            ),
            (
                "description",
                benchmark_problem(
                    "0104-test",
                    defects=[benchmark_defect(desc="")],
                ),
            ),
            (
                "expectation",
                benchmark_problem(
                    "0104-test",
                    defects=[benchmark_defect(expectation="")],
                ),
            ),
            (
                "positive fix",
                benchmark_problem(
                    "0104-test",
                    defects=[benchmark_defect(fix=None)],
                ),
            ),
            (
                "negative fix",
                benchmark_problem(
                    "0104-test",
                    defects=[benchmark_defect(is_negative=True)],
                ),
            ),
        )
        for label, value in invalid_values:
            with self.subTest(label=label), self.assertRaises(BenchmarkError):
                BenchmarkProblem.from_mapping(
                    value,
                    default_remote="https://github.com/asterinas/asterinas",
                )

    def test_load_problems_rejects_duplicate_problem_ids(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "problems.yaml"
            duplicate = benchmark_problem("0105-duplicate")
            path.write_text(json.dumps([duplicate, duplicate]), encoding="utf-8")
            with self.assertRaisesRegex(BenchmarkError, "duplicate problem_id"):
                load_problems(path, default_remote=None)

    def test_load_problems_rejects_non_mapping_items(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "problems.yaml"
            path.write_text("[invalid]", encoding="utf-8")
            with self.assertRaises(BenchmarkError):
                load_problems(path, default_remote=None)

    def test_load_problems_rejects_duplicate_numeric_ids(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "problems.yaml"
            path.write_text(
                json.dumps(
                    [
                        benchmark_problem("0105-first"),
                        benchmark_problem("0105-second"),
                    ]
                ),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(BenchmarkError, "numeric id"):
                load_problems(path, default_remote=None)

    def test_overlay_removes_benchmark_answers_from_historical_checkout(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "source" / "acr"
            source.mkdir(parents=True)
            (source / "runtime.py").write_text("trusted runtime\n", encoding="utf-8")
            (source / "benchmark").mkdir()
            (source / "benchmark" / "problems.yaml").write_text(
                "controller answers\n", encoding="utf-8"
            )

            worktree = root / "worktree"
            stale_outer = (
                worktree
                / ".agents"
                / "skills"
                / "aster-code-review"
                / "benchmark"
            )
            stale_outer.mkdir(parents=True)
            (stale_outer / "problems.yaml").write_text(
                "historical answers\n", encoding="utf-8"
            )
            stale_inner = stale_outer.parent / "acr" / "benchmark"
            stale_inner.mkdir(parents=True)
            (stale_inner / "problems.yaml").write_text(
                "historical inner answers\n", encoding="utf-8"
            )
            stale_claude = worktree / ".claude" / "answers"
            stale_claude.mkdir(parents=True)
            (stale_claude / "problems.yaml").write_text(
                "historical Claude answers\n", encoding="utf-8"
            )

            destination = overlay_package(source, worktree)
            self.assertEqual(
                (destination / "runtime.py").read_text(encoding="utf-8"),
                "trusted runtime\n",
            )
            self.assertFalse((destination / "benchmark").exists())
            self.assertFalse(stale_outer.exists())
            self.assertFalse((worktree / ".claude").exists())

    def test_normalize_grade(self):
        with tempfile.TemporaryDirectory() as temp:
            expected = Path(temp) / "expected.txt"
            output = Path(temp) / "output.json"
            expected.write_text("1.   location: foo.rs\n2.   location: bar.rs\n")
            output.write_text(json.dumps([
                {"defect": 2, "status": "miss", "reason": "not found"},
                {"defect": 1, "status": "caught", "reason": "found"},
            ]))
            result = normalize(expected, output)
            self.assertEqual((result["caught"], result["miss"]), (1, 1))
            self.assertEqual([item["defect"] for item in result["results"]], [1, 2])

    def test_calculate_recall_is_strict_and_partial_is_not_caught(self):
        expected = (
            "# Expected defects\n\n"
            "1. location: foo.rs\n   MATCH IF: first\n\n"
            "2. location: bar.rs\n   MATCH IF: second\n"
        )
        result = GraderEnvelope.model_validate({"results": [
            {"defect": 2, "status": "partial", "reason": "too vague"},
            {"defect": 1, "status": "caught", "reason": "found"},
        ]})
        report = calculate_recall(expected, result)
        self.assertEqual(report.recall, 0.5)
        self.assertEqual((report.caught, report.partial, report.miss), (1, 1, 0))
        self.assertEqual([item.defect for item in report.not_caught], [2])

        duplicate = GraderEnvelope.model_validate({"results": [
            {"defect": 1, "status": "caught", "reason": "found"},
            {"defect": 1, "status": "miss", "reason": "duplicate"},
        ]})
        with self.assertRaises(GraderError):
            calculate_recall(expected, duplicate)

    def test_print_recall_names_each_not_caught_defect(self):
        problem = BenchmarkProblem.from_mapping(
            benchmark_problem(
                "0106-test-problem",
                review_mode={"files": ["foo.rs"]},
                defects=[
                    benchmark_defect(
                        "foo.rs", expectation="find the first defect"
                    ),
                    benchmark_defect(
                        "bar.rs", expectation="find the second defect"
                    ),
                ],
            )
        )
        result = GraderEnvelope.model_validate({"results": [
            {"defect": 1, "status": "caught", "reason": "found"},
            {"defect": 2, "status": "miss", "reason": "not found"},
        ]})
        report = calculate_recall(problem.expected_text(), result)
        output = io.StringIO()
        with redirect_stdout(output):
            print_recall(problem, report)
        text = output.getvalue()
        self.assertIn("50.00% (1/2 caught", text)
        self.assertIn("defect 2 [miss] bar.rs: find the second defect", text)


if __name__ == "__main__":
    unittest.main()
