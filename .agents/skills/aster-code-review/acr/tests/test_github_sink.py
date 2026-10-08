from __future__ import annotations

import subprocess
import unittest
from pathlib import Path
from unittest.mock import patch

from acr.sinks.github import publish


class GitHubSinkTests(unittest.TestCase):
    @patch("acr.sinks.github.subprocess.run")
    def test_publish_forwards_arguments_to_the_acr_script(self, run) -> None:
        review = Path("review.md")

        publish(
            review,
            repo="asterinas/asterinas",
            pr=42,
            head_sha="deadbeef",
            finalize=True,
            event="approve",
        )

        script = (
            Path(__file__).resolve().parents[1]
            / "scripts"
            / "post_reviews_to_github.sh"
        )
        run.assert_called_once_with(
            [
                str(script),
                "--repo",
                "asterinas/asterinas",
                "--pr",
                "42",
                "--head-sha",
                "deadbeef",
                "--event",
                "approve",
                "--finalize",
                str(review),
            ],
            check=True,
        )

    @patch("acr.sinks.github.subprocess.run")
    def test_publish_propagates_script_failure(self, run) -> None:
        run.side_effect = subprocess.CalledProcessError(1, ["publisher"])

        with self.assertRaises(subprocess.CalledProcessError):
            publish(
                Path("review.md"),
                repo="asterinas/asterinas",
                pr="42",
                head_sha="deadbeef",
            )


if __name__ == "__main__":
    unittest.main()
