#!/usr/bin/env python3
"""Static dry-run validation for the fork release workflow.

This check runs without Docker, network, tags or GitHub credentials. It covers
the failure modes that are cheap to catch locally:

  * every workflow YAML under .github/workflows parses;
  * every `needs:` entry names an existing job and the graph has no cycle;
  * every `needs.<job>` expression refers to a direct dependency;
  * every `${{ ... }}` expression has balanced quotes/parentheses;
  * every local script referenced from `run:` exists (`scripts/`,
    `scripts-workflow/`), and every local action in `uses:` exists;
  * every `pnpm <script>` / `pnpm run <script>` alias exists in package.json;
  * the release workflow triggers on a tag glob that matches `v<package version>`,
    that tag has a Changelog.md heading, and prepare-release.sh's tag regex
    accepts it.

Usage:
    python3 scripts-workflow/check-release-workflow.py              # all workflows
    python3 scripts-workflow/check-release-workflow.py --no-act     # CI: no act
    python3 scripts-workflow/check-release-workflow.py --self-test  # mutation test
    python3 scripts-workflow/check-release-workflow.py --help

Requires PyYAML (already used by the workspace tooling) for YAML parsing.
When the `act` binary is installed the checker also runs `act --list` for the
release workflow and compares the listed jobs with the parsed jobs; this needs
no Docker daemon. Pass `--no-act` when the environment must not depend on act.
Exit codes: 0 pass, 1 check failure, 2 usage/environment error.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover - environment problem, not a check failure
    sys.exit("PyYAML is required: python3 -m pip install pyyaml")

WORKFLOW_DIR = Path(".github/workflows")
RELEASE_WORKFLOW = Path(".github/workflows/release.yml")

# pnpm subcommands that are CLI commands rather than package.json scripts.
PNPM_BUILTINS = {
    "i", "install", "add", "remove", "rm", "update", "up", "exec", "dlx",
    "why", "list", "ls", "link", "unlink", "import", "patch", "deploy",
    "config", "store", "prune", "publish", "pack", "outdated", "audit",
    "licenses", "sbom", "root", "bin", "env", "setup", "doctor", "rebuild",
}

SCRIPT_REF_RE = re.compile(
    r"\b(?:bash|sh|node|python3?|deno|bun)\s+"
    r"(?:--?[A-Za-z0-9][\w=.-]*\s+)*"
    r"((?:\./)?(?:scripts|scripts-workflow)/[A-Za-z0-9._/-]+)"
)
DIRECT_REF_RE = re.compile(
    r"(?m)^\s*(?:\./)((?:scripts|scripts-workflow)/[A-Za-z0-9._/-]+)"
)
NEEDS_REF_RE = re.compile(r"\bneeds\.([A-Za-z0-9_-]+)\b")
PNPM_RE = re.compile(r"\bpnpm\s+(?:run\s+)?([A-Za-z0-9:_-]+)\b")


class Checker:
    def __init__(self, root: Path, verbose: bool = False, use_act: bool = True) -> None:
        self.root = root
        self.verbose = verbose
        self.use_act = use_act
        self.failures: list[str] = []
        self.passes = 0
        self.skipped: list[str] = []

    def ok(self, message: str) -> None:
        self.passes += 1
        if self.verbose:
            print(f"PASS {message}")

    def fail(self, message: str) -> None:
        self.failures.append(message)
        print(f"FAIL {message}")

    def skip(self, message: str) -> None:
        self.skipped.append(message)
        print(f"SKIP {message}")

    def run(self, workflow_paths: list[Path], release_path: Path) -> int:
        workflows = self.load_workflows(workflow_paths)
        if workflows:
            self.check_graphs(workflows)
            self.check_expressions(workflows)
            self.check_local_refs(workflows)
            self.check_pnpm_scripts(workflows)
        self.check_release_tag(release_path)
        print(
            f"\n{self.passes} checks passed, {len(self.failures)} failed, "
            f"{len(self.skipped)} skipped"
        )
        return 0 if not self.failures else 1

    # -- loading -----------------------------------------------------------
    def load_workflows(self, paths: list[Path]) -> dict[Path, dict]:
        workflows: dict[Path, dict] = {}
        for path in paths:
            full = self.root / path
            if not full.is_file():
                self.fail(f"workflow missing: {path}")
                continue
            try:
                data = yaml.safe_load(full.read_text(encoding="utf-8"))
            except yaml.YAMLError as exc:
                self.fail(f"{path}: YAML parse error: {exc}")
                continue
            if not isinstance(data, dict):
                self.fail(f"{path}: top level is not a mapping")
                continue
            jobs = data.get("jobs")
            if not isinstance(jobs, dict) or not jobs:
                self.fail(f"{path}: no jobs mapping")
                continue
            workflows[path] = data
            self.ok(f"{path}: valid YAML with {len(jobs)} job(s)")
        return workflows

    # -- needs graph and expressions --------------------------------------
    @staticmethod
    def needs_of(job: dict) -> list[str]:
        needs = job.get("needs", [])
        if isinstance(needs, str):
            return [needs]
        if isinstance(needs, list):
            return [str(n) for n in needs]
        return []

    def check_graphs(self, workflows: dict[Path, dict]) -> None:
        for path, data in workflows.items():
            jobs = data["jobs"]
            for job_id, job in jobs.items():
                if not isinstance(job, dict):
                    self.fail(f"{path}: job {job_id} is not a mapping")
                    continue
                for need in self.needs_of(job):
                    if need not in jobs:
                        self.fail(
                            f"{path}: job {job_id} needs missing job '{need}'"
                        )
                    else:
                        self.ok(f"{path}: {job_id} -> {need}")
            for cycle in self.find_cycles(jobs):
                self.fail(f"{path}: needs cycle: {' -> '.join(cycle)}")
            if jobs and not self.find_cycles(jobs):
                self.ok(f"{path}: needs graph is acyclic")

    def find_cycles(self, jobs: dict) -> list[list[str]]:
        state: dict[str, int] = {}
        stack: list[str] = []
        cycles: list[list[str]] = []

        def visit(job_id: str) -> None:
            if not isinstance(jobs.get(job_id), dict):
                return
            state[job_id] = 1
            stack.append(job_id)
            for need in self.needs_of(jobs[job_id] or {}):
                if need not in jobs:
                    continue
                if state.get(need) == 1:
                    start = stack.index(need)
                    cycles.append(stack[start:] + [need])
                elif state.get(need, 0) == 0:
                    visit(need)
            stack.pop()
            state[job_id] = 2

        for job_id in jobs:
            if state.get(job_id, 0) == 0:
                visit(job_id)
        return cycles

    @staticmethod
    def expression_errors(text: str) -> list[str]:
        errors: list[str] = []
        pos = 0
        while True:
            start = text.find("${{", pos)
            if start < 0:
                break
            end = text.find("}}", start + 3)
            if end < 0:
                errors.append("unterminated ${{")
                break
            expr = text[start + 3 : end]
            depth = 0
            quote: str | None = None
            for char in expr:
                if quote:
                    if char == quote:
                        quote = None
                elif char in "'\"":
                    quote = char
                elif char == "(":
                    depth += 1
                elif char == ")":
                    depth -= 1
                    if depth < 0:
                        errors.append("unbalanced ')'")
            if quote:
                errors.append("unterminated quote")
            if depth:
                errors.append("unbalanced parentheses")
            pos = end + 2
        return errors

    def check_expressions(self, workflows: dict[Path, dict]) -> None:
        for path, data in workflows.items():
            for job_id, job in data["jobs"].items():
                if not isinstance(job, dict):
                    continue
                needs = set(self.needs_of(job))
                for key, value in self.walk(job):
                    if not isinstance(value, str):
                        continue
                    if key == "if" or "${{" in value:
                        for error in self.expression_errors(value):
                            self.fail(f"{path}: job {job_id} {key}: {error}")
                        for ref in NEEDS_REF_RE.findall(value):
                            if ref not in needs:
                                self.fail(
                                    f"{path}: job {job_id} references "
                                    f"needs.{ref} but needs={sorted(needs)}"
                                )
                self.ok(f"{path}: job {job_id} expressions balanced")

    def walk(self, value, key: str = ""):
        if isinstance(value, dict):
            for child_key, child in value.items():
                child_key = str(child_key)
                if child_key == "needs":
                    continue  # job-name list, not an expression
                yield from self.walk(child, child_key)
        elif isinstance(value, list):
            for child in value:
                yield from self.walk(child, key)
        else:
            yield key, value

    # -- local files and package scripts ----------------------------------
    def check_local_refs(self, workflows: dict[Path, dict]) -> None:
        for path, data in workflows.items():
            for job_id, job in data["jobs"].items():
                if not isinstance(job, dict):
                    continue
                for key, value in self.walk(job):
                    if not isinstance(value, str):
                        continue
                    if key == "run":
                        refs = set(SCRIPT_REF_RE.findall(value))
                        refs |= set(DIRECT_REF_RE.findall(value))
                        for ref in refs:
                            full = self.root / ref.removeprefix("./")
                            if full.is_file():
                                self.ok(f"{path}: {job_id} uses {ref}")
                            else:
                                self.fail(
                                    f"{path}: job {job_id} references missing "
                                    f"local script: {ref}"
                                )
                    elif key == "uses" and value.startswith("./"):
                        full = self.root / value.removeprefix("./")
                        if full.is_dir() and any(
                            (full / name).is_file()
                            for name in ("action.yml", "action.yaml", "Dockerfile")
                        ):
                            self.ok(f"{path}: {job_id} uses local action {value}")
                        else:
                            self.fail(
                                f"{path}: job {job_id} local action not found: {value}"
                            )

    def check_pnpm_scripts(self, workflows: dict[Path, dict]) -> None:
        pkg_path = self.root / "package.json"
        try:
            package = json.loads(pkg_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            self.fail(f"package.json unreadable: {exc}")
            return
        scripts = package.get("scripts", {})
        for path, data in workflows.items():
            for job_id, job in data["jobs"].items():
                if not isinstance(job, dict):
                    continue
                for key, value in self.walk(job):
                    if not isinstance(value, str) or key != "run":
                        continue
                    for name in set(PNPM_RE.findall(value)):
                        if name in PNPM_BUILTINS or name.startswith("-"):
                            continue
                        if name in scripts:
                            self.ok(f"{path}: {job_id} pnpm script '{name}' exists")
                        else:
                            self.fail(
                                f"{path}: job {job_id} references missing "
                                f"package.json script: {name}"
                            )

    # -- release tag / version --------------------------------------------
    @staticmethod
    def tag_matches(pattern: str, tag: str) -> bool:
        regex = "^" + "".join(
            "[^/]*" if char == "*" else "[^/]" if char == "?" else re.escape(char)
            for char in pattern
        ) + "$"
        return re.match(regex, tag) is not None

    def check_release_tag(self, release_path: Path) -> None:
        full = self.root / release_path
        if not full.is_file():
            self.fail(f"release workflow missing: {release_path}")
            return
        try:
            data = yaml.safe_load(full.read_text(encoding="utf-8"))
        except yaml.YAMLError as exc:
            self.fail(f"{release_path}: YAML parse error: {exc}")
            return
        on = data.get("on")
        if on is None and True in data:  # YAML 1.1 parses `on:` as boolean True
            on = data[True]
        push = on.get("push", {}) if isinstance(on, dict) else {}
        tag_patterns = push.get("tags", []) if isinstance(push, dict) else []
        if isinstance(tag_patterns, str):
            tag_patterns = [tag_patterns]
        if not tag_patterns:
            self.fail(f"{release_path}: push.tags trigger is missing")
            return

        pkg_path = self.root / "package.json"
        try:
            version = json.loads(pkg_path.read_text(encoding="utf-8"))["version"]
        except (OSError, json.JSONDecodeError, KeyError) as exc:
            self.fail(f"package.json version unreadable: {exc}")
            return
        tag = f"v{version}"
        matching = [p for p in tag_patterns if self.tag_matches(p, tag)]
        if matching:
            self.ok(f"{release_path}: tag {tag} matches trigger {matching[0]}")
        else:
            self.fail(
                f"{release_path}: tag {tag} matches none of {tag_patterns}"
            )

        first = next(iter(data.get("jobs", {})), "")
        if first in {"check_tag_version", "prepare_release"}:
            self.ok(f"{release_path}: first job {first} present")
        else:
            self.fail(f"{release_path}: unexpected first job {first!r}")

        changelog = self.root / "Changelog.md"
        if changelog.is_file():
            heading = f"## {tag}"
            if any(
                line.startswith(heading)
                for line in changelog.read_text(encoding="utf-8").splitlines()
            ):
                self.ok(f"Changelog.md: has heading '{heading}'")
            else:
                self.fail(
                    f"Changelog.md: missing heading '{heading}' "
                    "(prepare_release would fail)"
                )
        else:
            self.fail("Changelog.md missing")

        prepare = self.root / "scripts" / "prepare-release.sh"
        if prepare.is_file():
            text = prepare.read_text(encoding="utf-8")
            candidates = re.findall(r"grep -qE '([^']+)'", text)
            pattern = next(
                (p for p in candidates if p.startswith("^v") and "[0-9]" in p),
                None,
            )
            if pattern is None:
                self.fail("scripts/prepare-release.sh: tag regex not found")
            elif re.match(pattern, tag):
                self.ok(f"prepare-release.sh accepts tag {tag}")
            else:
                self.fail(f"prepare-release.sh rejects tag {tag} ({pattern})")
        else:
            self.fail("scripts/prepare-release.sh missing")

        tauri = self.root / "src-tauri" / "tauri.conf.json"
        if tauri.is_file():
            try:
                tauri_version = json.loads(
                    tauri.read_text(encoding="utf-8")
                ).get("version")
            except json.JSONDecodeError as exc:
                self.fail(f"src-tauri/tauri.conf.json parse error: {exc}")
            else:
                if tauri_version == version:
                    self.ok(f"tauri.conf.json version matches {version}")
                else:
                    print(
                        f"WARN src-tauri/tauri.conf.json version "
                        f"{tauri_version!r} != package.json {version!r} "
                        "(release.yml patches both from the tag)"
                    )

        # act --list must agree with the parsed job set.
        act = shutil.which("act") if self.use_act else None
        if not self.use_act:
            self.skip("act cross-check disabled (--no-act)")
        elif not act:
            self.skip("act not installed; skipped act --list cross-check")
        else:
            proc = subprocess.run(
                [act, "--list", "-W", str(release_path)],
                cwd=self.root,
                capture_output=True,
                text=True,
            )
            if proc.returncode != 0:
                self.fail(
                    "act --list failed: "
                    + (proc.stderr.strip().splitlines()[-1] if proc.stderr else "")
                )
            else:
                listed = set(
                    re.findall(r"^\d+\s+(\S+)\s", proc.stdout, re.MULTILINE)
                )
                expected = set(data.get("jobs", {}))
                if listed == expected:
                    self.ok(f"act --list lists all {len(expected)} release jobs")
                else:
                    self.fail(
                        f"act --list mismatch: missing={sorted(expected - listed)} "
                        f"extra={sorted(listed - expected)}"
                    )


def default_workflows(root: Path) -> list[Path]:
    paths = []
    for pattern in ("*.yml", "*.yaml"):
        paths.extend(
            sorted(p.relative_to(root) for p in (root / WORKFLOW_DIR).glob(pattern))
        )
    return paths


def self_test() -> int:
    """Mutate a throwaway copy and prove each class of breakage is caught."""
    repo = Path(__file__).resolve().parent.parent
    source = repo / RELEASE_WORKFLOW
    mutations = [
        ("broken needs reference",
         lambda p: p.replace("needs: [prepare_release]",
                             "needs: [definitely_missing_job]", 1),
         "definitely_missing_job"),
        ("missing local script",
         lambda p: p.replace("bash ./scripts-workflow/group_platforms.sh",
                             "bash ./scripts-workflow/absent-check.sh", 1),
         "absent-check.sh"),
    ]
    version_mutation = (
        "version without changelog entry",
        None,  # handled by rewriting package.json
        "Changelog.md",
    )

    failures = 0
    with tempfile.TemporaryDirectory(prefix="release-workflow-selftest-") as tmp:
        tmp_root = Path(tmp)

        def build_fixture() -> None:
            if tmp_root.exists():
                for child in tmp_root.iterdir():
                    if child.is_dir():
                        shutil.rmtree(child)
                    else:
                        child.unlink()
            workflow = tmp_root / RELEASE_WORKFLOW
            workflow.parent.mkdir(parents=True)
            shutil.copy2(source, workflow)
            for name in ("package.json", "Changelog.md"):
                shutil.copy2(repo / name, tmp_root / name)
            for rel in (
                "src-tauri/tauri.conf.json",
                "scripts-workflow/group_platforms.sh",
                "scripts/extract_update_logs.sh",
                "scripts/verify-macos-signing.sh",
                "scripts/prepare-release.sh",
            ):
                dest = tmp_root / rel
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(repo / rel, dest)

        def invoke() -> subprocess.CompletedProcess:
            return subprocess.run(
                [
                    sys.executable,
                    str(Path(__file__).resolve()),
                    "--root",
                    str(tmp_root),
                    "--workflow",
                    str(RELEASE_WORKFLOW),
                ],
                capture_output=True,
                text=True,
            )

        build_fixture()
        baseline = invoke()
        if baseline.returncode == 0:
            print("PASS self-test baseline: unmodified release workflow passes")
        else:
            failures += 1
            print("FAIL self-test baseline: unmodified workflow failed")
            print(baseline.stdout)
            print(baseline.stderr)

        for label, mutate, expected in mutations:
            build_fixture()
            path = tmp_root / RELEASE_WORKFLOW
            path.write_text(mutate(path.read_text(encoding="utf-8")), encoding="utf-8")
            proc = invoke()
            if proc.returncode != 0 and expected in proc.stdout:
                print(f"PASS self-test caught {label}")
            else:
                failures += 1
                print(f"FAIL self-test missed {label} (rc={proc.returncode})")
                print(proc.stdout)

        build_fixture()
        pkg_path = tmp_root / "package.json"
        pkg = json.loads(pkg_path.read_text(encoding="utf-8"))
        pkg["version"] = "9.9.9"
        pkg_path.write_text(json.dumps(pkg, indent=2) + "\n", encoding="utf-8")
        proc = invoke()
        if proc.returncode != 0 and version_mutation[2] in proc.stdout:
            print(f"PASS self-test caught {version_mutation[0]}")
        else:
            failures += 1
            print(f"FAIL self-test missed {version_mutation[0]} (rc={proc.returncode})")
            print(proc.stdout)

    return 0 if failures == 0 else 1


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Static dry-run validation for the fork release workflow."
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="repository root (default: parent of scripts-workflow/)",
    )
    parser.add_argument(
        "--workflow",
        action="append",
        default=[],
        help="workflow path relative to root (repeatable; default: all)",
    )
    parser.add_argument(
        "--release-workflow",
        type=Path,
        default=RELEASE_WORKFLOW,
        help=f"release workflow path (default: {RELEASE_WORKFLOW})",
    )
    parser.add_argument("--verbose", action="store_true", help="print each passed check")
    parser.add_argument("--no-act", action="store_true", help="skip the optional act cross-check")
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="mutate a temp copy and prove breakages are caught",
    )
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    root = args.root.resolve()
    workflows = (
        [Path(p) for p in args.workflow]
        if args.workflow
        else default_workflows(root)
    )
    if not workflows:
        print(f"FAIL no workflows found under {root / WORKFLOW_DIR}")
        return 1
    return Checker(root, verbose=args.verbose, use_act=not args.no_act).run(workflows, args.release_workflow)


if __name__ == "__main__":
    sys.exit(main())
