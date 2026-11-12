#!/usr/bin/env python3
"""End-to-end acceptance: timer auto-update coalesces bundle members.

Proves, against the installed app and privileged service, that two remote
Profile Bundle members whose 1-minute timer fires together are fetched
individually but regenerated/applied exactly once:

  * two  ``[Timer] Running timer task for profile: <uid>`` lines (same round)
  * one  ``[Cmd] coalesced bundle materialization finished`` line
  * one  ``Operation 'PUT_OPTIMIZED /clash/stage-runtime'`` line
  * one bundle ``generated_at`` advance and both marker proxies present in the
    generated profile and in the running core's ``/proxies``

Safety: the app is stopped before any file changes. ``profiles.yaml``, every
bundle member file, ``bundles/bundle-main.yaml``, ``profiles/bundle-main.yaml``
and ``verge.yaml`` are copied into a 0700 run directory and restored before the
app is started again; the restore is verified by hash, by the live core's
``/configs`` snapshot, by the absence of the markers and by the timer map.

The member urls are temporarily pointed at an in-process loopback HTTP server,
so both fetches finish inside the materializer debounce window and the run does
not depend on upstream subscription latency. Served bytes are the real member
files plus one unused marker proxy each, so the applied runtime only gains two
unused proxies.

Usage:
    python3 scripts-workflow/run-timer-bundle-e2e.py --dry-run
    python3 scripts-workflow/run-timer-bundle-e2e.py --yes
    python3 scripts-workflow/run-timer-bundle-e2e.py --restore-only <run-dir>

Exit codes: 0 pass, 1 preconditions/assertions failed, 2 restore verification
failed (backup kept), 3 interrupted after changes (best-effort restore done).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from datetime import datetime, timedelta
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

try:
    import yaml
except ImportError:  # pragma: no cover - environment problem
    sys.exit("PyYAML is required: python3 -m pip install pyyaml")

APP_NAME = "Clash Verge"
APP_PATH = Path("/Applications/Clash Verge.app")
EXPECTED_VERSION_PREFIX = "2.5.10-fa.5"
DEFAULT_DATA_DIR = Path.home() / "Library/Application Support/io.github.clash-verge-rev.clash-verge-rev"
PROFILES = "profiles.yaml"
META = "bundles/bundle-main.yaml"
GENERATED = "profiles/bundle-main.yaml"
VERGE = "verge.yaml"

TS_RE = re.compile(r"^\[(\d{4}-\d\d-\d\d \d\d:\d\d:\d\d)\.")


def run(cmd: list[str], *, timeout: float = 30, check: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    if check and result.returncode != 0:
        raise RuntimeError(f"command failed ({result.returncode}): {' '.join(cmd)}\n{result.stderr.strip()}")
    return result


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def app_pids() -> list[int]:
    result = run(["pgrep", "-f", str(APP_PATH / "Contents/MacOS/clash-verge")], check=False)
    return [int(pid) for pid in result.stdout.split()]


def wait_for(predicate, timeout: float, interval: float = 0.5) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return True
        time.sleep(interval)
    return predicate()


def quit_app(timeout: float = 45) -> None:
    if not app_pids():
        return
    for _ in range(3):
        subprocess.run(
            ["osascript", "-e", f'tell application "{APP_NAME}" to quit'],
            capture_output=True,
            text=True,
            timeout=20,
        )
        if wait_for(lambda: not app_pids(), timeout / 3):
            return
    raise RuntimeError("app did not quit gracefully; no files were changed")


def start_app() -> None:
    run(["open", "-a", APP_NAME], timeout=30)
    if not wait_for(lambda: bool(app_pids()), 30):
        raise RuntimeError("app process did not start")


def app_version(app_path: Path) -> str:
    return run(
        ["/usr/libexec/PlistBuddy", "-c", "Print :CFBundleShortVersionString", str(app_path / "Contents/Info.plist")]
    ).stdout.strip()


class Logs:
    def __init__(self, log_dir: Path, since: datetime):
        self.log_dir = log_dir
        self.since = since - timedelta(seconds=1)

    def lines(self) -> list[tuple[datetime, str]]:
        out: list[tuple[datetime, str]] = []
        for path in sorted(self.log_dir.glob("*.log")):
            try:
                text = path.read_text(errors="replace")
            except OSError:
                continue
            for line in text.splitlines():
                match = TS_RE.match(line)
                if not match:
                    continue
                stamp = datetime.strptime(match.group(1), "%Y-%m-%d %H:%M:%S")
                if stamp >= self.since:
                    out.append((stamp, line))
        return out

    def matching(self, needle: str) -> list[tuple[datetime, str]]:
        return [(stamp, line) for stamp, line in self.lines() if needle in line]


class ServeHandler(BaseHTTPRequestHandler):
    files: dict[str, bytes] = {}

    def do_GET(self) -> None:  # noqa: N802 - http.server API
        body = self.files.get(self.path.lstrip("/"))
        if body is None:
            self.send_response(404)
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("Content-Type", "application/yaml")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args: Any) -> None:  # keep the run output clean
        return


def serve(files: dict[str, bytes]) -> tuple[ThreadingHTTPServer, int]:
    handler = type("Handler", (ServeHandler,), {"files": files})
    server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, name="bundle-e2e-http", daemon=True)
    thread.start()
    return server, server.server_address[1]


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def mihomo_get(sock: Path, path: str) -> Any:
    result = run(["curl", "-s", "-m", "5", "--unix-socket", str(sock), f"http://localhost{path}"], check=False)
    if result.returncode != 0 or not result.stdout:
        raise RuntimeError(f"core API {path} unavailable: {result.stderr.strip()}")
    return json.loads(result.stdout)


def marker_entry(indent: str, marker: str) -> str:
    return (
        f"{indent}- name: {marker}\n"
        f"{indent}  type: ss\n"
        f"{indent}  server: 127.0.0.1\n"
        f"{indent}  port: 1\n"
        f"{indent}  cipher: aes-128-gcm\n"
        f"{indent}  password: e2e-marker\n"
    )


def list_indent(rest: str) -> str:
    """Match the indentation the member already uses for its proxy list items."""
    for line in rest.splitlines():
        if line.strip():
            match = re.match(r"^([ \t]*)-", line)
            return match.group(1) if match else ""
    return ""


def add_marker(text: str, marker: str) -> str:
    match = re.search(r"(?m)^proxies:[ \t]*(.*)$", text)
    if match is None:
        return text.rstrip("\n") + "\nproxies:\n" + marker_entry("", marker)
    rest = match.group(1).strip()
    if rest == "":
        entry = marker_entry(list_indent(text[match.end() :]), marker)
        return text[: match.end()] + "\n" + entry + text[match.end() :]
    if rest == "[]":
        return text[: match.start()] + "proxies:\n" + marker_entry("", marker) + text[match.end() :]
    raise RuntimeError("member profile has an unsupported inline proxies value")


def load_yaml(path: Path) -> dict[str, Any]:
    data = yaml.safe_load(path.read_text())
    if not isinstance(data, dict):
        raise RuntimeError(f"{path} is not a YAML mapping")
    return data


def save_yaml(path: Path, data: dict[str, Any]) -> None:
    path.write_text(yaml.safe_dump(data, allow_unicode=True, sort_keys=False, default_flow_style=False))


def schedule_map(profiles: dict[str, Any]) -> dict[str, int]:
    scheduled: dict[str, int] = {}
    for item in profiles.get("items") or []:
        option = item.get("option") or {}
        interval = option.get("update_interval")
        if option.get("allow_auto_update", True) and isinstance(interval, int) and interval > 0 and item.get("uid"):
            scheduled[item["uid"]] = interval
    return scheduled


def remote_members(profiles: dict[str, Any], meta: dict[str, Any]) -> list[dict[str, Any]]:
    by_uid = {item.get("uid"): item for item in profiles.get("items") or []}
    members = []
    for uid in meta.get("members") or []:
        item = by_uid.get(uid)
        if item is not None and item.get("type") == "remote":
            members.append(item)
    return members


def read_meta(data_dir: Path) -> dict[str, Any]:
    meta = yaml.safe_load((data_dir / META).read_text())
    if not isinstance(meta, dict):
        raise RuntimeError(f"{META} is not a YAML mapping")
    return meta


def preconditions(args: argparse.Namespace) -> dict[str, Any]:
    if sys.platform != "darwin":
        raise RuntimeError("this acceptance targets the macOS app")
    if not APP_PATH.exists():
        raise RuntimeError(f"app not installed at {APP_PATH}")
    version = app_version(APP_PATH)
    if not version.startswith(EXPECTED_VERSION_PREFIX):
        raise RuntimeError(f"installed app is {version}, expected {EXPECTED_VERSION_PREFIX}*")
    data_dir: Path = args.app_data
    for name in (PROFILES, META, GENERATED, VERGE):
        if not (data_dir / name).is_file():
            raise RuntimeError(f"missing {data_dir / name}")
    sock = socket_path(args)
    if not app_pids():
        raise RuntimeError("app is not running; start it once so pre-test runtime state can be captured")
    if not sock.exists():
        raise RuntimeError(f"core control socket missing: {sock}")
    profiles = load_yaml(data_dir / PROFILES)
    meta = read_meta(data_dir)
    members = remote_members(profiles, meta)
    if len(members) != 2:
        raise RuntimeError(f"expected exactly 2 remote bundle members, found {[m.get('uid') for m in members]}")
    member_uids = list(meta.get("members") or [])
    for uid in member_uids:
        if not (data_dir / "profiles" / f"{uid}.yaml").is_file():
            raise RuntimeError(f"bundle member file missing: profiles/{uid}.yaml")
    configs = mihomo_get(sock, "/configs")
    subset = {
        "mixed-port": configs.get("mixed-port"),
        "mode": configs.get("mode"),
        "tun.enable": (configs.get("tun") or {}).get("enable"),
        "tun.device": (configs.get("tun") or {}).get("device"),
    }
    return {
        "version": version,
        "sock": str(sock),
        "profiles": profiles,
        "meta": meta,
        "member_uids": member_uids,
        "remote_uids": [m["uid"] for m in members],
        "scheduled": schedule_map(profiles),
        "pre_version": mihomo_get(sock, "/version").get("version"),
        "pre_configs": subset,
        "pre_generated_at": meta.get("generated_at"),
        "pre_hashes": {name: sha256(data_dir / name) for name in (PROFILES, META, GENERATED, VERGE)},
    }


def socket_path(args: argparse.Namespace) -> Path:
    return args.service_sock or Path(f"/var/run/clash-verge-service/users/{os.getuid()}/verge-mihomo.sock")


def backup_files(data_dir: Path, run_dir: Path, member_uids: list[str]) -> dict[str, dict[str, Any]]:
    names = [PROFILES, META, GENERATED, VERGE] + [f"profiles/{uid}.yaml" for uid in member_uids]
    backup: dict[str, dict[str, Any]] = {}
    for name in names:
        source = data_dir / name
        target = run_dir / "backup" / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
        backup[name] = {"sha256": sha256(source), "mode": source.stat().st_mode & 0o777}
    (run_dir / "backup").chmod(0o700)
    return backup


def restore_backup(data_dir: Path, run_dir: Path, state: dict[str, Any]) -> None:
    for name, digest in state["backup"].items():
        source = run_dir / "backup" / name
        target = data_dir / name
        shutil.copy2(source, target)
        if sha256(target) != digest["sha256"]:
            raise RuntimeError(f"restored {name} does not match its backup")


def wait_ready(logs: Logs, sock: Path, timeout: float = 120) -> None:
    def ready() -> bool:
        return bool(logs.matching("Timer initialization completed")) or bool(
            logs.matching("Registered timer task count:")
        )

    if not wait_for(ready, timeout, 1.0):
        raise RuntimeError("app did not log timer initialization")
    if not wait_for(lambda: sock.exists(), 30):
        raise RuntimeError("core control socket did not appear")


def collect_events(logs: Logs) -> dict[str, list[tuple[datetime, str]]]:
    return {
        "timer": logs.matching("Running timer task for profile:"),
        "materialized": logs.matching("coalesced bundle materialization finished"),
        "materialize_failed": logs.matching("coalesced bundle materialization failed"),
        "stage_ok": logs.matching("Operation 'PUT_OPTIMIZED /clash/stage-runtime' completed successfully"),
        "stage_retry": logs.matching("operation: PUT_OPTIMIZED /clash/stage-runtime"),
        "stage_failed": logs.matching("Operation 'PUT_OPTIMIZED /clash/stage-runtime' failed"),
        "fetch_ok": logs.matching("[订阅更新] 更新订阅配置成功"),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--dry-run", action="store_true", help="check preconditions and print the plan only")
    parser.add_argument("--yes", action="store_true", help="required to modify and restart the live app")
    parser.add_argument("--restore-only", metavar="RUN_DIR", help="restore a previous run and verify it")
    parser.add_argument("--keep-artifacts", action="store_true", help="keep secret-bearing backups after verification")
    parser.add_argument("--app-data", type=Path, default=DEFAULT_DATA_DIR)
    parser.add_argument("--service-sock", type=Path, help="override the core control socket path")
    parser.add_argument("--timer-timeout", type=float, default=180.0)
    args = parser.parse_args()

    if args.restore_only:
        return restore_only(args, Path(args.restore_only))
    facts = preconditions(args)
    print(f"app {facts['version']} | core {facts['pre_version']} | bundle members {facts['member_uids']}")
    print(f"remote members under test: {facts['remote_uids']} | scheduled timer map: {facts['scheduled']}")
    print(f"pre-test runtime: {facts['pre_configs']} | generated_at={facts['pre_generated_at']}")
    if args.dry_run:
        print("dry-run: no files changed, app not restarted")
        return 0
    if not args.yes:
        print("refusing to touch the live app without --yes (use --dry-run to inspect)")
        return 1

    run_dir = Path(tempfile.mkdtemp(prefix="cvr-bundle-e2e-"))
    run_dir.chmod(0o700)
    state: dict[str, Any] = {"created": int(time.time()), "status": "prepared", "data_dir": str(args.app_data)}
    state.update(
        {
            "backup": backup_files(args.app_data, run_dir, facts["member_uids"]),
            "member_uids": facts["member_uids"],
            "remote_uids": facts["remote_uids"],
            "pre_hashes": facts["pre_hashes"],
            "pre_configs": facts["pre_configs"],
            "pre_version": facts["pre_version"],
            "pre_generated_at": facts["pre_generated_at"],
            "scheduled": facts["scheduled"],
            "sock": facts["sock"],
        }
    )
    state_path = run_dir / "state.json"
    state_path.write_text(json.dumps(state, indent=2, sort_keys=True))
    print(f"run dir (0700): {run_dir}")
    server: ThreadingHTTPServer | None = None
    try:
        server, port = prepare_live_run(args, facts, run_dir, state, state_path)
        events = wait_for_round(args, facts, state, run_dir, state_path)
        evaluate(facts, state, run_dir, state_path, events)
        quit_app()
        restore_backup(args.app_data, run_dir, state)
        verify_restored_files(args.app_data, state)
        state["status"] = "restored"
        state_path.write_text(json.dumps(state, indent=2, sort_keys=True))
        restore_start = datetime.now()
        start_app()
        verify_restore(state, run_dir, restore_start)
        state["status"] = "verified"
        state_path.write_text(json.dumps(state, indent=2, sort_keys=True))
        print(f"RESTORE VERIFIED: {run_dir / 'summary.json'}")
        if not args.keep_artifacts:
            shutil.rmtree(run_dir / "backup")
            shutil.rmtree(run_dir / "serve", ignore_errors=True)
        return 0
    except Exception as error:  # noqa: BLE001 - report and restore
        print(f"FAILED: {error}", file=sys.stderr)
        state["status"] = "failed"
        state["error"] = str(error)
        state_path.write_text(json.dumps(state, indent=2, sort_keys=True))
        try:
            if server is not None:
                server.shutdown()
            quit_app()
            restore_backup(args.app_data, run_dir, state)
            start_app()
            print(f"best-effort restore done; backups kept at {run_dir}/backup", file=sys.stderr)
        except Exception as restore_error:  # noqa: BLE001
            print(f"RESTORE FAILED: {restore_error}; backups at {run_dir}/backup", file=sys.stderr)
            return 3
        return 1
    finally:
        if server is not None:
            server.shutdown()


def prepare_live_run(
    args: argparse.Namespace,
    facts: dict[str, Any],
    run_dir: Path,
    state: dict[str, Any],
    state_path: Path,
) -> tuple[ThreadingHTTPServer, int]:
    epoch = int(time.time())
    marker = f"bundle-e2e-{epoch}"
    serve_dir = run_dir / "serve"
    serve_dir.mkdir()
    served: dict[str, bytes] = {}
    markers: dict[str, str] = {}
    for uid in facts["remote_uids"]:
        source = args.app_data / "profiles" / f"{uid}.yaml"
        marker_name = f"{marker}-{uid}"
        markers[uid] = marker_name
        body = add_marker(source.read_text(), marker_name)
        yaml.safe_load(body)  # fail before touching the live app, not after an invalid fetch
        served[f"{uid}.yaml"] = body.encode()
    quit_app()
    server, port = serve(served)

    profiles = facts["profiles"]
    now = int(time.time())
    for item in profiles.get("items") or []:
        if item.get("uid") in facts["remote_uids"]:
            item["url"] = f"http://127.0.0.1:{port}/{item['uid']}.yaml"
            option = item.setdefault("option", {})
            option["update_interval"] = 1
            option["allow_auto_update"] = True
            item["updated"] = now
    save_yaml(args.app_data / PROFILES, profiles)
    state.update({"markers": markers, "port": port, "modified": now, "status": "modified"})
    state_path.write_text(json.dumps(state, indent=2, sort_keys=True))

    start_app()
    logs = Logs(args.app_data / "logs", datetime.now())
    wait_ready(logs, Path(facts["sock"]))
    state["started"] = datetime.now().isoformat(timespec="seconds")
    state_path.write_text(json.dumps(state, indent=2, sort_keys=True))
    state["_logs_since"] = state["started"]
    return server, port


def wait_for_round(
    args: argparse.Namespace,
    facts: dict[str, Any],
    state: dict[str, Any],
    run_dir: Path,
    state_path: Path,
) -> dict[str, Any]:
    since = datetime.fromisoformat(state["_logs_since"])
    logs = Logs(facts_path(args) / "logs", since)
    expected = set(state["remote_uids"])

    def round_seen() -> bool:
        fired = {uid for _stamp, line in logs.matching("Running timer task for profile:") for uid in expected if uid in line}
        return fired == expected and bool(logs.matching("coalesced bundle materialization finished"))

    if not wait_for(round_seen, args.timer_timeout, 1.0):
        events = collect_events(logs)
        raise RuntimeError(f"timer round/materialization not observed: {summarize(events)}")
    for _ in range(60):
        events = collect_events(logs)
        if events["stage_ok"] or events["materialize_failed"]:
            break
        time.sleep(0.5)
    events = collect_events(logs)
    print(
        "events: "
        + ", ".join(f"{name}={len(rows)}" for name, rows in events.items())
        + f" | first materialize={fmt(events['materialized'])}"
    )
    state["events"] = {name: [(stamp.isoformat(), line) for stamp, line in rows] for name, rows in events.items()}
    state_path.write_text(json.dumps(state, indent=2, sort_keys=True))
    return events


def facts_path(args: argparse.Namespace) -> Path:
    return args.app_data


def evaluate(facts: dict[str, Any], state: dict[str, Any], run_dir: Path, state_path: Path, events: dict[str, Any]) -> None:
    data_dir = Path(state["data_dir"])
    timer_lines = events["timer"]
    expected = set(state["remote_uids"])
    fired = {uid for _stamp, line in timer_lines for uid in expected if uid in line}
    if fired != expected:
        raise AssertionError(f"only {sorted(fired)} timer tasks fired, expected {sorted(expected)}")
    fired_span = max(stamp for stamp, _line in timer_lines) - min(stamp for stamp, _line in timer_lines)
    if fired_span > timedelta(seconds=5):
        raise AssertionError(f"timer tasks fired {fired_span} apart; not one round")

    materialized = events["materialized"]
    if len(materialized) != 1:
        raise AssertionError(f"expected exactly 1 coalesced materialization, got {len(materialized)}")
    first = materialized[0][0]
    window_start = min(stamp for stamp, _line in timer_lines) - timedelta(seconds=2)
    stage_in_window = [row for row in events["stage_ok"] if window_start <= row[0] <= first + timedelta(seconds=20)]
    if len(stage_in_window) != 1:
        raise AssertionError(f"expected exactly 1 /clash/stage-runtime PUT in the timer window, got {len(stage_in_window)}")
    if events["materialize_failed"]:
        raise AssertionError("materializer retried or failed during the run")

    meta = read_meta(data_dir)
    generated = (data_dir / GENERATED).read_text()
    sock = Path(state["sock"])
    markers = state["markers"]
    missing_generated = [uid for uid, name in markers.items() if name not in generated]

    def markers_applied() -> list[str]:
        proxies = mihomo_get(sock, "/proxies").get("proxies") or {}
        return [uid for uid, name in markers.items() if name not in proxies]

    if not wait_for(lambda: not markers_applied(), 15, 0.5):
        missing_running = markers_applied()
    else:
        missing_running = []
    if missing_generated or missing_running:
        raise AssertionError(
            f"marker not in generated={missing_generated} running={missing_running}; "
            "the coalesced apply did not reach the generated profile/core"
        )
    if not isinstance(meta.get("generated_at"), int) or meta["generated_at"] <= state["pre_generated_at"]:
        raise AssertionError(f"bundle generated_at did not advance: {state['pre_generated_at']} -> {meta.get('generated_at')}")

    summary = {
        "result": "pass",
        "app_version": facts["version"],
        "core_version": facts["pre_version"],
        "remote_members": state["remote_uids"],
        "timer_fired": sorted(fired),
        "timer_span_seconds": fired_span.total_seconds(),
        "materializations": len(materialized),
        "stage_put_optimized": len(stage_in_window),
        "member_fetches_ok": len(events["fetch_ok"]),
        "generated_at": [state["pre_generated_at"], meta["generated_at"]],
        "member_mtimes": {uid: (data_dir / "profiles" / f"{uid}.yaml").stat().st_mtime for uid in state["remote_uids"]},
        "materialize_first": first.isoformat(),
        "stage_first": stage_in_window[0][0].isoformat(),
        "evidence_log_lines": {
            "timer": [line for _stamp, line in timer_lines],
            "materialized": [line for _stamp, line in materialized],
            "stage": [line for _stamp, line in stage_in_window],
        },
    }
    (run_dir / "summary.json").write_text(json.dumps(summary, indent=2, ensure_ascii=False))
    print(json.dumps(summary, indent=2, ensure_ascii=False))


def verify_restored_files(data_dir: Path, state: dict[str, Any]) -> None:
    for name, digest in state["backup"].items():
        if sha256(data_dir / name) != digest["sha256"]:
            raise RuntimeError(f"restore drift: {name} does not match its backup")


def verify_restore(state: dict[str, Any], run_dir: Path, restore_start: datetime) -> None:
    data_dir = Path(state["data_dir"])
    logs = Logs(data_dir / "logs", restore_start)
    sock = Path(state["sock"])
    wait_ready(logs, sock)
    if not wait_for(lambda: sock.exists(), 30):
        raise RuntimeError("restore: core control socket missing")
    meta = read_meta(data_dir)
    if meta.get("generated_at") != state["pre_generated_at"]:
        raise RuntimeError(f"restore drift: generated_at {state['pre_generated_at']} -> {meta.get('generated_at')}")
    proxies = mihomo_get(sock, "/proxies").get("proxies") or {}
    leftovers = [name for name in state["markers"].values() if name in proxies]
    if leftovers:
        raise RuntimeError(f"restore drift: markers still applied: {leftovers}")
    configs = mihomo_get(sock, "/configs")
    subset = {
        "mixed-port": configs.get("mixed-port"),
        "mode": configs.get("mode"),
        "tun.enable": (configs.get("tun") or {}).get("enable"),
        "tun.device": (configs.get("tun") or {}).get("device"),
    }
    if subset != state["pre_configs"]:
        raise RuntimeError(f"restore drift: runtime config {state['pre_configs']} -> {subset}")
    registered = {
        m.group(1): int(m.group(2))
        for _stamp, line in logs.matching("Registered timer task: uid=")
        if (m := re.search(r"uid=([^,]+), interval=(\d+)min", line))
    }
    if registered != state["scheduled"]:
        raise RuntimeError(f"restore drift: timer map {state['scheduled']} -> {registered}")
    print(f"restore verified: hashes match, generated_at={meta.get('generated_at')}, runtime={subset}, timer={registered}")


def restore_only(_args: argparse.Namespace, run_dir: Path) -> int:
    state = json.loads((run_dir / "state.json").read_text())
    data_dir = Path(state["data_dir"])
    if app_pids():
        quit_app()
    restore_backup(data_dir, run_dir, state)
    verify_restored_files(data_dir, state)
    restore_start = datetime.now()
    start_app()
    verify_restore(state, run_dir, restore_start)
    print("restore-only verified")
    return 0


def fmt(rows: list[tuple[datetime, str]]) -> str:
    return rows[0][0].isoformat() if rows else "none"


def summarize(events: dict[str, Any]) -> str:
    return ", ".join(f"{name}={len(rows)}" for name, rows in events.items())


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(3)
