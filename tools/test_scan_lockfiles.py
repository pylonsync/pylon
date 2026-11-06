"""Check that source scans include nested lockfiles and exclude generated files."""

import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch
from types import SimpleNamespace
from contextlib import redirect_stderr
from io import StringIO

spec = importlib.util.spec_from_file_location("scan_lockfiles", Path(__file__).with_name("scan-lockfiles.py"))
scan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scan)


class LockfilesTest(unittest.TestCase):
    def test_all_tracked_scopes(self):
        paths = [
            "Cargo.lock", "bun.lock", "crates/studio_api/web/bun.lock",
            "docker/framework-deps/bun.lock", "benchmarks/search/Cargo.lock",
            "vendor/samael/Cargo.lock", "examples/game/shards/zone/Cargo.lock",
            "Package.resolved", "future/app/pnpm-lock.yaml",
        ]
        self.assertEqual(scan.tracked_lockfiles(paths), sorted(paths))

    def test_non_lockfiles(self):
        self.assertEqual(scan.tracked_lockfiles([
            "crates/core/src/clock.rs", "vendor/samael/flake.lock", "package.json", "",
        ]), [])

    def test_scanner_receives_each_path_and_preserves_errors(self):
        with patch("sys.argv", ["scan-lockfiles.py", "--scanner", "./osv-scanner"]), \
             patch.object(scan.subprocess, "check_output", return_value=b"Cargo.lock\0docker/framework deps/bun.lock\0"), \
             patch.object(scan.subprocess, "run", return_value=SimpleNamespace(returncode=2)) as run, \
             redirect_stderr(StringIO()):
            self.assertEqual(scan.main(), 2)
        args = run.call_args.args[0]
        self.assertEqual(args[0], "./osv-scanner")
        self.assertEqual(args[-4:], ["--lockfile", "Cargo.lock", "--lockfile", "docker/framework deps/bun.lock"])


if __name__ == "__main__":
    unittest.main()
