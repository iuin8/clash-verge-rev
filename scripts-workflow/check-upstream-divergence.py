#!/usr/bin/env python3
"""Measure and gate divergence from the upstream clash-verge-rev baseline.

This is the executable companion to
`docs/workspace/reference/cvr-upstream-merge.md`. It compares the current HEAD
(and working tree) against an upstream ref, prints the conflict surface, checks
that the manual-sync watchlist still carries fork logic, and fails when a
configured divergence budget regresses.

Usage:
    python3 scripts-workflow/check-upstream-divergence.py
    python3 scripts-workflow/check-upstream-divergence.py --report-only
    python3 scripts-workflow/check-upstream-divergence.py --max-modified-hunks 100

The baseline tag must exist locally (fetch it from the `upstream` remote first):
    git fetch upstream --tags

Exit codes: 0 within budgets and watchlist intact, 1 regression/watchlist
failure, 2 missing baseline ref or git error.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

DEFAULT_UPSTREAM = "v2.5.10"
RELEASE_WORKFLOW = ".github/workflows/release.yml"

# Recorded at HEAD 84638263 on branch fa/v2.5.10-rebuild, after this checker was
# added. Update deliberately after an intentional divergence change and record
# the new reading in the SOP. CLI --max-* flags override individual budgets.
BASELINE = {
    "modified_files": 31,
    "modified_added_lines": 1050,
    "modified_deleted_lines": 278,
    "modified_hunks": 125,
    "fork_only_files": 66,
    "modified_locales": 0,
    "release_hunks": 1,
}

# Files where an upstream merge can silently undo fork behavior. Every entry
# must still exist, be divergent from upstream, and keep its markers.
WATCHLIST = [
    {
        "path": "src-tauri/src/cmd/bundle_delete.rs",
        "why": "copied upstream delete transaction; snapshot/apply/rollback stays atomic",
        "markers": ["snapshot_artifacts", "restore_artifacts", "PROFILE_WRITE_LOCK"],
    },
    {
        "path": "src-tauri/src/cmd/bundle_save.rs",
        "why": "save/validate/apply hook for bundle members",
        "markers": ["snapshot_artifacts", "restore_artifacts"],
    },
    {
        "path": "src-tauri/src/feat/bundle_update.rs",
        "why": "manual/auto update wrapper; automatic updates defer materialization",
        "markers": ["defer_bundle_materialize", "mark_bundle_dirty"],
    },
    {
        "path": "src/locales/*/bundle.json",
        "why": "fork-only locale namespace, loaded by src/services/i18n.ts",
        "json_keys": ["page", "components"],
        "min_files": 14,
    },
    {
        "path": "src/services/i18n.ts",
        "why": "loads the fork bundle namespace at startup",
        "markers": ["'bundle'"],
    },
    {
        "path": ".github/workflows/release.yml",
        "why": "upstream release workflow guard skips -fa. tags",
        "markers": ["-fa."],
    },
    {
        "path": ".github/workflows/release-fa.yml",
        "why": "fork-only release workflow triggers on v*-fa.*",
        "markers": ["v*-fa.*"],
    },
]


class Git:
    def __init__(self, root: Path) -> None:
        self.root = root

    def run(self, *args: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            ["git", "-C", str(self.root), *args],
            capture_output=True,
            text=True,
        )

    def text(self, *args: str) -> str:
        proc = self.run(*args)
        if proc.returncode != 0:
            raise RuntimeError(proc.stderr.strip() or f"git {' '.join(args)} failed")
        return proc.stdout


def name_status(git: Git, ref: str) -> dict[str, str]:
    status: dict[str, str] = {}
    for line in git.text(
        "diff", "--name-status", "--no-renames", ref, "--", "."
    ).splitlines():
        if not line.strip():
            continue
        state, path = line.split("\t", 1)
        status[path] = state[0]
    return status


def numstat(git: Git, ref: str, *filters: str) -> tuple[int, int]:
    added = deleted = 0
    for line in git.text(
        "diff", "--numstat", "--no-renames", *filters, ref, "--", "."
    ).splitlines():
        parts = line.split("\t")
        if len(parts) < 3:
            continue
        added += 0 if parts[0] == "-" else int(parts[0])
        deleted += 0 if parts[1] == "-" else int(parts[1])
    return added, deleted


def file_numstat(git: Git, ref: str) -> dict[str, tuple[int, int]]:
    stats: dict[str, tuple[int, int]] = {}
    for line in git.text(
        "diff", "--numstat", "--no-renames", ref, "--", "."
    ).splitlines():
        parts = line.split("\t")
        if len(parts) < 3:
            continue
        added = 0 if parts[0] == "-" else int(parts[0])
        deleted = 0 if parts[1] == "-" else int(parts[1])
        stats[parts[2]] = (added, deleted)
    return stats


def hunks_per_file(git: Git, ref: str) -> dict[str, int]:
    hunks: dict[str, int] = {}
    current: str | None = None
    for line in git.text(
        "diff", "-U0", "--no-color", "--no-ext-diff", "--no-renames", ref, "--", "."
    ).splitlines():
        if line.startswith("diff --git "):
            current = line.split(" b/", 1)[1] if " b/" in line else None
        elif line.startswith("@@") and current:
            hunks[current] = hunks.get(current, 0) + 1
    return hunks


def check_watchlist(root: Path, status: dict[str, str]) -> list[str]:
    failures: list[str] = []
    files_checked = 0
    for entry in WATCHLIST:
        entry_failures = len(failures)
        matches = sorted(root.glob(entry["path"]))
        if not matches:
            failures.append(f"watchlist missing: {entry['path']} ({entry['why']})")
            continue
        minimum = entry.get("min_files", 1)
        if len(matches) < minimum:
            failures.append(
                f"watchlist incomplete: {entry['path']} has {len(matches)} "
                f"file(s), expected >= {minimum}"
            )
        for path in matches:
            rel = path.relative_to(root).as_posix()
            state = status.get(rel)
            if state not in {"A", "M"}:
                failures.append(
                    f"watchlist not divergent: {rel} is {state or 'unchanged'} "
                    f"vs upstream ({entry['why']})"
                )
            if "json_keys" in entry:
                try:
                    data = json.loads(path.read_text(encoding="utf-8"))
                except (OSError, json.JSONDecodeError) as exc:
                    failures.append(f"watchlist invalid JSON: {rel}: {exc}")
                    continue
                missing = [key for key in entry["json_keys"] if key not in data]
                if missing:
                    failures.append(
                        f"watchlist missing namespace keys in {rel}: {missing}"
                    )
            for marker in entry.get("markers", []):
                if marker not in path.read_text(encoding="utf-8"):
                    failures.append(
                        f"watchlist marker missing in {rel}: {marker!r}"
                    )
            files_checked += 1
        if len(failures) == entry_failures:
            print(f"PASS watchlist {entry['path']}: {len(matches)} file(s)")
    print(f"{files_checked} watchlist file(s) checked")
    return failures


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Measure fork/upstream divergence and enforce budgets."
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="repository root (default: parent of scripts-workflow/)",
    )
    parser.add_argument(
        "--upstream-ref",
        default=DEFAULT_UPSTREAM,
        help=f"upstream ref to compare against (default: {DEFAULT_UPSTREAM})",
    )
    parser.add_argument(
        "--report-only",
        action="store_true",
        help="print readings but do not fail on budget regressions",
    )
    parser.add_argument("--top", type=int, default=12, help="hotspot rows (default: 12)")
    for name in BASELINE:
        parser.add_argument(
            f"--max-{name.replace('_', '-')}",
            type=int,
            default=None,
            help=f"override baseline budget for {name} (current: {BASELINE[name]})",
        )
    args = parser.parse_args()

    root = args.root.resolve()
    git = Git(root)
    if git.run("rev-parse", "--git-dir").returncode != 0:
        print(f"FAIL {root} is not a git repository")
        return 2
    probe = git.run("rev-parse", "--verify", f"{args.upstream_ref}^{{commit}}")
    if probe.returncode != 0:
        print(
            f"FAIL baseline ref {args.upstream_ref!r} not found; "
            "run `git fetch upstream --tags` first"
        )
        return 2

    ref = args.upstream_ref
    status = name_status(git, ref)
    hunks = hunks_per_file(git, ref)
    stats = file_numstat(git, ref)
    added, deleted = numstat(git, ref)
    mod_added, mod_deleted = numstat(git, ref, "--diff-filter=M")
    modified = sorted(path for path, state in status.items() if state == "M")
    added_files = sorted(path for path, state in status.items() if state == "A")
    deleted_files = sorted(path for path, state in status.items() if state == "D")
    locales = {p: s for p, s in status.items() if p.startswith("src/locales/")}
    locale_modified = sorted(p for p, s in locales.items() if s == "M")
    locale_added = sorted(p for p, s in locales.items() if s == "A")
    modified_hunks = sum(hunks.get(path, 0) for path in modified)
    release_hunks = hunks.get(RELEASE_WORKFLOW, 0)

    print(f"upstream: {ref}")
    print(f"changed files: {len(status)} (+{added}/-{deleted}), hunks {sum(hunks.values())}")
    print(
        f"upstream-modified: {len(modified)} files (+{mod_added}/-{mod_deleted}), "
        f"hunks {modified_hunks}"
    )
    print(f"fork-only files: {len(added_files)}; deleted upstream files: {len(deleted_files)}")
    print(
        f"locales: upstream-modified={len(locale_modified)}, "
        f"fork-only namespace files={len(locale_added)}"
    )
    print(f"{RELEASE_WORKFLOW} hunks: {release_hunks}")
    print("\nconflict hotspots (hunks, +/- lines, status, path):")
    ranked = sorted(
        status,
        key=lambda path: (hunks.get(path, 0), sum(stats.get(path, (0, 0)))),
        reverse=True,
    )
    for path in ranked[: args.top]:
        plus, minus = stats.get(path, (0, 0))
        print(
            f"  {hunks.get(path, 0):3d}  +{plus:<5d} -{minus:<5d} "
            f"{status[path]}  {path}"
        )

    print()
    failures = check_watchlist(root, status)

    readings = {
        "modified_files": len(modified),
        "modified_added_lines": mod_added,
        "modified_deleted_lines": mod_deleted,
        "modified_hunks": modified_hunks,
        "fork_only_files": len(added_files),
        "modified_locales": len(locale_modified),
        "release_hunks": release_hunks,
    }
    print("\nbudgets:")
    for name, actual in readings.items():
        override = getattr(args, f"max_{name}")
        limit = BASELINE[name] if override is None else override
        state = "PASS" if actual <= limit else "FAIL"
        if actual <= limit:
            print(f"  PASS {name}: {actual} <= {limit}")
        elif args.report_only:
            print(f"  INFO {name}: {actual} > {limit} (report-only)")
        else:
            print(f"  FAIL {name}: {actual} > {limit}")
            failures.append(f"budget regression: {name} {actual} > {limit}")

    if failures:
        print(f"\nFAIL: {len(failures)} divergence issue(s)")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("\nOK: divergence within budgets and manual-sync watchlist intact")
    if args.report_only:
        print("(report-only: budget overages would not fail)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
