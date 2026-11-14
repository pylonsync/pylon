"""Check Docker context exclusions with synthetic files and a local builder.

Run: python3 -m unittest discover -s tools -p 'test_docker_context.py' -v
Requires Docker with BuildKit. No base image or application secret is used.
"""

from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent


@unittest.skipUnless(shutil.which("docker"), "Docker is required")
class DockerContextTest(unittest.TestCase):
    def check_context(self, rules):
        with tempfile.TemporaryDirectory(prefix="pylon-context-test-") as tmp:
            context = Path(tmp) / "context"
            output = Path(tmp) / "output"
            context.mkdir()
            (context / ".dockerignore").write_text(rules)
            (context / "Dockerfile").write_text("FROM scratch\nCOPY . /\n")
            private = [
                ".env", ".env.production", "app.db", "app.db-wal",
                "app.db-shm", "app.db-journal", ".pylon/sessions.db",
                ".git/config", "node_modules/private-package/.env",
            ]
            private = [prefix + name for prefix in ("", "packages/app/")
                       for name in private]
            public = ["app.ts", "package.json", "bun.lock",
                      "packages/app/functions/hello.ts"]
            for name in private + public:
                path = context / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("synthetic fixture\n")
            result = subprocess.run(
                ["docker", "build", "--network=none", "--no-cache",
                 "--output", f"type=local,dest={output}", str(context)],
                capture_output=True, text=True, timeout=120,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            leaked = [name for name in private if (output / name).exists()]
            self.assertEqual(leaked, [], "Private files entered the image")
            for name in public:
                self.assertTrue((output / name).is_file(), name)

    def test_repository_context(self):
        self.check_context((ROOT / ".dockerignore").read_text())

    def test_generated_app_context(self):
        source = (ROOT / "crates/cli/src/commands/deploy.rs").read_text()
        # Read the literal used by the generator. Fail if it changes shape.
        match = re.search(
            r'fn generate_dockerignore\(\) -> String \{\s*format!\(\s*r#"(.*?)"#',
            source, re.DOTALL,
        )
        self.assertIsNotNone(match, "Cannot read generated Docker ignore rules")
        self.check_context(match.group(1).replace("{GENERATED_MARKER}", "test"))


if __name__ == "__main__":
    unittest.main()
