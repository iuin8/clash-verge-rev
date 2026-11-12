#!/usr/bin/env python3
"""Assert the fork release CI is isolated from the upstream release workflow.

Upstream owns `.github/workflows/release.yml`. Fork releases live in
`.github/workflows/release-fa.yml`, and the upstream workflow gets exactly one
job-level guard (`!contains(github.ref_name, '-fa.')`) on its first job so a
`-fa.` tag skips every upstream job through `needs:` propagation.

This static check fails when that separation drifts:
  * upstream still triggers on the upstream tag glob and its first job still
    has the `-fa.` exclusion guard plus the original push/publish conditions;
  * every other upstream job reaches that first job through `needs:` and no
    job uses `always()`, so skipping the guard skips the whole workflow;
  * release-fa.yml triggers only on `v*-fa.*`, matches package.json's fork tag,
    and does not match the stable tag;
  * release-fa.yml keeps the fork Debian-only bundle arg, updater/latest
    behavior, macOS signing verification and attestation steps.

Usage: python3 scripts-workflow/check-release-isolation.py
Exit codes: 0 isolated, 1 drift, 2 usage/environment error.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover - environment problem, not a check failure
    sys.exit("PyYAML is required: python3 -m pip install pyyaml")

UPSTREAM = Path(".github/workflows/release.yml")
FORK = Path(".github/workflows/release-fa.yml")


def load(root: Path, path: Path) -> dict:
    data = yaml.safe_load((root / path).read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError(f"{path}: top level is not a mapping")
    return data


def on_config(data: dict) -> dict:
    on = data.get("on")
    if on is None and True in data:  # YAML 1.1 parses `on:` as boolean True
        on = data[True]
    return on if isinstance(on, dict) else {}


def tag_patterns(data: dict) -> list[str]:
    push = on_config(data).get("push", {})
    tags = push.get("tags", []) if isinstance(push, dict) else []
    if isinstance(tags, str):
        tags = [tags]
    return [str(tag) for tag in tags]


def tag_matches(pattern: str, tag: str) -> bool:
    regex = "^" + "".join(
        "[^/]*" if char == "*" else "[^/]" if char == "?" else re.escape(char)
        for char in pattern
    ) + "$"
    return re.match(regex, tag) is not None


def needs_of(job: dict) -> list[str]:
    needs = job.get("needs", [])
    if isinstance(needs, str):
        return [needs]
    if isinstance(needs, list):
        return [str(need) for need in needs]
    return []


class Isolation:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.failures: list[str] = []
        self.passes = 0

    def ok(self, message: str) -> None:
        self.passes += 1
        print(f"PASS {message}")

    def fail(self, message: str) -> None:
        self.failures.append(message)
        print(f"FAIL {message}")

    def run(self) -> int:
        try:
            upstream = load(self.root, UPSTREAM)
            fork = load(self.root, FORK)
        except (OSError, ValueError, yaml.YAMLError) as exc:
            print(f"FAIL {exc}")
            return 1

        version = json.loads(
            (self.root / "package.json").read_text(encoding="utf-8")
        )["version"]
        fork_tag = f"v{version}"
        stable_tag = f"v{version.split('-fa.')[0]}"

        self.check_upstream(upstream)
        self.check_fork(fork, fork_tag, stable_tag)
        self.check_trigger_split(upstream, fork, fork_tag, stable_tag)
        print(f"\n{self.passes} checks passed, {len(self.failures)} failed")
        return 0 if not self.failures else 1

    def check_upstream(self, upstream: dict) -> None:
        tags = tag_patterns(upstream)
        if "v*.*.*" in tags:
            self.ok(f"{UPSTREAM}: upstream tag glob v*.*.* present")
        else:
            self.fail(f"{UPSTREAM}: upstream tag glob v*.*.* missing ({tags})")

        jobs = upstream.get("jobs", {})
        first_id = next(iter(jobs), "")
        guard = str(jobs.get(first_id, {}).get("if", ""))
        if "!contains" in guard and "-fa." in guard:
            self.ok(f"{UPSTREAM}: first job {first_id} excludes -fa. tags")
        else:
            self.fail(f"{UPSTREAM}: first job {first_id} lacks -fa. guard")
        for fragment in ("github.event_name == 'push'", "publish/v"):
            if fragment in guard:
                self.ok(f"{UPSTREAM}: guard keeps upstream condition {fragment!r}")
            else:
                self.fail(f"{UPSTREAM}: guard dropped upstream condition {fragment!r}")

        dependents: dict[str, list[str]] = {job_id: [] for job_id in jobs}
        for job_id, job in jobs.items():
            for need in needs_of(job if isinstance(job, dict) else {}):
                if need in dependents:
                    dependents[need].append(job_id)
        reachable = {first_id}
        frontier = [first_id]
        while frontier:
            for child in dependents.get(frontier.pop(), []):
                if child not in reachable:
                    reachable.add(child)
                    frontier.append(child)
        if reachable == set(jobs):
            self.ok(f"{UPSTREAM}: all jobs reach {first_id} via needs")
        else:
            self.fail(
                f"{UPSTREAM}: jobs bypass {first_id}: {sorted(set(jobs) - reachable)}"
            )
        for job_id, job in jobs.items():
            if isinstance(job, dict) and "always()" in str(job.get("if", "")):
                self.fail(f"{UPSTREAM}: job {job_id} uses always(); guard can be bypassed")
        self.ok(f"{UPSTREAM}: no job bypasses the first-job guard with always()")

    def check_fork(self, fork: dict, fork_tag: str, stable_tag: str) -> None:
        name = str(fork.get("name", ""))
        if name and name != "Release Build":
            self.ok(f"{FORK}: distinct workflow name {name!r}")
        else:
            self.fail(f"{FORK}: workflow name collides with upstream concurrency group")

        tags = tag_patterns(fork)
        if any(tag_matches(pattern, fork_tag) for pattern in tags):
            self.ok(f"{FORK}: matching pattern covers {fork_tag}")
        else:
            self.fail(f"{FORK}: no pattern matches fork tag {fork_tag} ({tags})")
        if "v*-fa.*" in tags:
            self.ok(f"{FORK}: fork tag pattern v*-fa.* present")
        else:
            self.fail(f"{FORK}: fork tag pattern v*-fa.* missing ({tags})")
        if any(tag_matches(pattern, stable_tag) for pattern in tags):
            self.fail(f"{FORK}: pattern also matches stable tag {stable_tag}")

        jobs = fork.get("jobs", {})
        required = {
            "check_tag_version",
            "prepare_release",
            "release",
            "update_tag",
            "release-update",
        }
        if required <= set(jobs):
            self.ok(f"{FORK}: fork release jobs present")
        else:
            self.fail(f"{FORK}: missing fork jobs {sorted(required - set(jobs))}")

        text = (self.root / FORK).read_text(encoding="utf-8")
        features = {
            "Debian-only bundles": "--bundles deb",
            "updater JSON": "includeUpdaterJson: true",
            "updater publish": "pnpm updater",
            "macOS signing verification": "scripts/verify-macos-signing.sh",
            "build provenance": "attest-build-provenance",
        }
        for label, fragment in features.items():
            if fragment in text:
                self.ok(f"{FORK}: keeps {label}")
            else:
                self.fail(f"{FORK}: lost {label} ({fragment!r})")

    def check_trigger_split(
        self, upstream: dict, fork: dict, fork_tag: str, stable_tag: str
    ) -> None:
        upstream_matches_fork = any(
            tag_matches(pattern, fork_tag) for pattern in tag_patterns(upstream)
        )
        if upstream_matches_fork:
            self.ok(
                f"trigger split: upstream glob also fires on {fork_tag}, "
                "but first-job guard skips it"
            )
        else:
            self.fail(f"trigger split: upstream glob does not fire on {fork_tag}")
        if any(
            tag_matches(pattern, stable_tag) for pattern in tag_patterns(fork)
        ):
            self.fail(f"trigger split: fork workflow fires on stable tag {stable_tag}")
        else:
            self.ok(f"trigger split: fork workflow ignores stable tag {stable_tag}")


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Check fork/upstream release workflow isolation."
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="repository root (default: parent of scripts-workflow/)",
    )
    return Isolation(parser.parse_args().root.resolve()).run()


if __name__ == "__main__":
    sys.exit(main())
