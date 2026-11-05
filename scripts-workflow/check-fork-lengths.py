#!/usr/bin/env python3
"""In-repo subset of the workspace file-length/link gate.

The authoritative gate is `sh tools/check-workspace.sh` at the workspace root,
which also checks the AGENTS/OPS-SOP docs and the docs/workspace link index.
GitHub Actions checks out only this repository, so those workspace-root files
are unavailable and the full gate cannot run in CI.

This script mirrors only the `clash-verge-rev/` entries from the workspace
file-length checker (`tools/check-file-lengths.py`): the fork-owned merge /
profile / multi-merge paths plus this repo's release-workflow checker. Keep the
two target lists in sync when fork-owned paths move. Upstream-owned files stay
ungated, matching the workspace policy.

File-length limits (soft/hard, lines):
    .md 300/500, .sh/.bash 400/600, .py/.rs 600/900, .yaml/.yml 400/600

The link half of the gate is documented as a CI limitation: docs/workspace and
the workspace-root allowlist live outside this repository. Run the full gate at
the workspace root before pushing. In-repo there are no fork-owned Markdown
docs whose links need checking, so only the length half runs here.

The same script asserts that every TARGETS pattern is covered by the
pull_request and push path filters of .github/workflows/workflow-static-check.yml,
so a newly added fork-owned target cannot silently stop running in CI.

Usage:
    python3 scripts-workflow/check-fork-lengths.py [--root PATH] [--verbose]
Exit codes: 0 pass (soft overages are reported but allowed), 1 hard violation
or trigger-coverage gap.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover - environment problem, not a check failure
    sys.exit("PyYAML is required: python3 -m pip install pyyaml")

THRESHOLDS = {
    ".md": (300, 500),
    ".sh": (400, 600),
    ".bash": (400, 600),
    ".py": (600, 900),
    ".rs": (600, 900),
    ".yaml": (400, 600),
    ".yml": (400, 600),
}

WORKFLOW = Path(".github/workflows/workflow-static-check.yml")

# Mirror of the clash-verge-rev entries in workspace tools/check-file-lengths.py.
TARGETS = [
    "src-tauri/src/cmd/merge.rs",
    "src-tauri/src/cmd/profile.rs",
    "src-tauri/src/cmd/profile_tests.rs",
    "src-tauri/src/cmd/merge_acceptance.rs",
    "src-tauri/tests/**/*.rs",
    "scripts-workflow/check-release-workflow.py",
    "src-tauri/src/enhance/multi_merge.rs",
    "src-tauri/src/enhance/multi_merge/**/*.rs",
]


def line_count(path: Path) -> int:
    data = path.read_bytes()
    if b"\0" in data[:4096]:
        return 0
    # A newline-terminated file has exactly N lines, not N + 1.
    lines = data.count(b"\n")
    return lines if data.endswith(b"\n") else lines + 1


def collect(root: Path) -> list[Path]:
    files: set[Path] = set()
    for pattern in TARGETS:
        files.update(p for p in root.glob(pattern) if p.is_file())
    return sorted(files)


def path_covers(pattern: str, target: str) -> bool:
    """Whether a workflow path filter necessarily matches a target pattern."""
    pattern = pattern.rstrip("/")
    target = target.rstrip("/")
    if pattern == target:
        return True
    if pattern.endswith("/**"):
        base = pattern[:-3]
        return target == base or target.startswith(base + "/")
    if pattern.endswith("*") and not pattern.endswith("**"):
        base = pattern[:-1]
        return target.startswith(base) and "/" not in target[len(base):]
    return False


def check_trigger_coverage(root: Path) -> int:
    workflow = root / WORKFLOW
    if not workflow.is_file():
        print(f"FAIL trigger coverage: missing {WORKFLOW}")
        return 1
    data = yaml.safe_load(workflow.read_text(encoding="utf-8"))
    on = data.get("on", data.get(True)) if isinstance(data, dict) else None
    missing = 0
    for event in ("pull_request", "push"):
        config = on.get(event) if isinstance(on, dict) else None
        paths = config.get("paths", []) if isinstance(config, dict) else []
        if isinstance(paths, str):
            paths = [paths]
        if not paths:
            print(f"FAIL trigger coverage: {event} has no paths")
            missing += 1
            continue
        for target in TARGETS:
            if not any(path_covers(str(p), target) for p in paths):
                print(
                    f"FAIL trigger coverage: {event}.paths does not cover {target}"
                )
                missing += 1
    if not missing:
        print(
            f"OK trigger coverage: {len(TARGETS)} fork targets covered by "
            "pull_request and push paths"
        )
    return missing


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Check fork-owned CVR file lengths (workspace-gate subset)."
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="repository root (default: parent of scripts-workflow/)",
    )
    parser.add_argument(
        "--verbose", action="store_true", help="print every checked file"
    )
    args = parser.parse_args()

    root = args.root.resolve()
    files = collect(root)
    if not files:
        print(f"FAIL no fork-owned targets found under {root}")
        return 1

    hard = 0
    for path in files:
        rel = path.relative_to(root).as_posix()
        limits = THRESHOLDS.get(path.suffix.lower())
        if limits is None:
            continue
        soft, hard_limit = limits
        count = line_count(path)
        if count > hard_limit:
            print(f"HARD {count:5d} > {hard_limit:4d}  {rel}")
            hard += 1
        elif count > soft:
            print(f"soft {count:5d} > {soft:4d}  {rel}")
        elif args.verbose:
            print(f"ok   {count:5d} <= {soft:4d}  {rel}")

    print(
        "SKIP link gate: docs/workspace lives at the workspace root, outside "
        "this repository; run `sh tools/check-workspace.sh` there."
    )
    coverage = check_trigger_coverage(root)
    if hard or coverage:
        print(
            f"\nFAIL: {hard} hard file-length violation(s), "
            f"{coverage} trigger-coverage gap(s)"
        )
        return 1
    print(f"OK: {len(files)} fork-owned file(s) within hard length limits")
    return 0


if __name__ == "__main__":
    sys.exit(main())
