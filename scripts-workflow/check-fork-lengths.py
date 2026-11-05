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

Usage:
    python3 scripts-workflow/check-fork-lengths.py [--root PATH] [--verbose]
Exit codes: 0 pass (soft overages are reported but allowed), 1 hard violation.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

THRESHOLDS = {
    ".md": (300, 500),
    ".sh": (400, 600),
    ".bash": (400, 600),
    ".py": (600, 900),
    ".rs": (600, 900),
    ".yaml": (400, 600),
    ".yml": (400, 600),
}

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
    if hard:
        print(f"\nFAIL: {hard} hard file-length violation(s)")
        return 1
    print(f"OK: {len(files)} fork-owned file(s) within hard length limits")
    return 0


if __name__ == "__main__":
    sys.exit(main())
