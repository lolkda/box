#!/usr/bin/env python3
"""Run legacy box.tool and Rust boxctl against the same subscription fixtures.

This is a baseline harness, not a product test: it records observable file/JSON
differences so the Rust implementation can be compared with the sealed legacy
shell implementation.
"""
from __future__ import annotations

import argparse
import http.server
import json
import os
import re
import shutil
import subprocess
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = Path(__file__).resolve().parent / "fixtures"
RUST_BIN = ROOT / "rust" / "target" / "release" / "boxctl"


def run(cmd: list[str], *, env: dict[str, str] | None = None, cwd: Path | None = None, timeout=30):
    return subprocess.run(cmd, env=env, cwd=cwd, text=True, capture_output=True, timeout=timeout)


def yaml_json(path: Path):
    if not path.exists() or path.stat().st_size == 0:
        return None
    p = run(["yq", "-o=json", "eval", ".", str(path)])
    if p.returncode != 0:
        return {"_parse_error": p.stderr.strip()}
    try:
        return json.loads(p.stdout)
    except json.JSONDecodeError:
        return {"_raw": p.stdout}


def files(directory: Path):
    if not directory.exists():
        return []
    result = []
    for path in sorted(directory.rglob("*")):
        if path.is_file():
            rel = str(path.relative_to(directory))
            result.append({"path": rel, "sha256": __import__("hashlib").sha256(path.read_bytes()).hexdigest(), "yaml": yaml_json(path) if path.suffix in {".yaml", ".yml"} else None})
    return result


def make_busybox(path: Path):
    path.write_text("#!/usr/bin/env bash\napplet=\"$1\"; shift\nexec \"$applet\" \"$@\"\n")
    path.chmod(0o755)


def prepare_root(root: Path, *, legacy: bool, fixture_base: str):
    box = root / "box"
    (box / "mihomo").mkdir(parents=True)
    (box / "scripts").mkdir()
    (box / "bin").mkdir()
    (box / "run").mkdir()
    (root / "bin").mkdir()
    for name in ("settings.json", "settings.default.json", "settings.schema.json"):
        shutil.copy2(ROOT / "box" / name, box / name)
    shutil.copy2(ROOT / "box" / "mihomo" / "config.yaml", box / "mihomo" / "config.yaml")
    if legacy:
        shutil.copy2(ROOT / "legacy" / "settings.base.sh", box / "settings.base.sh")
        shutil.copy2(ROOT / "legacy" / "box-scripts" / "box.tool", box / "scripts" / "box.tool")
        shutil.copy2(ROOT / "legacy" / "box-scripts" / "settings-gen.awk", box / "scripts" / "settings-gen.awk")
        make_busybox(root / "bin" / "busybox")
    return box


def legacy_run(box: Path, args: list[str]):
    env = os.environ.copy()
    env["BOX_HOME"] = str(box)
    env["PATH"] = f"{box.parent / 'bin'}:{env.get('PATH','')}"
    return run(["bash", "-O", "extglob", str(box / "scripts" / "box.tool"), *args], env=env)


def rust_start(box: Path):
    if not RUST_BIN.exists():
        raise SystemExit(f"missing Rust binary: {RUST_BIN}; run cargo build --release")
    socket = box / "run" / "boxctl" / "control.sock"
    last = None
    for _ in range(3):
        err_file = open("/tmp/rust-oracle-daemon.err", "w")
        daemon = subprocess.Popen(
            [str(RUST_BIN), "--root", str(box), "daemon"],
            stdout=subprocess.DEVNULL,
            stderr=err_file,
        )
        for _ in range(100):
            if socket.exists():
                return daemon
            if daemon.poll() is not None:
                last = daemon.returncode
                break
            time.sleep(0.05)
        if daemon.poll() is None:
            daemon.terminate()
            try:
                daemon.wait(timeout=3)
            except subprocess.TimeoutExpired:
                daemon.kill()
        time.sleep(0.2)
    raise RuntimeError(f"Rust daemon socket did not appear (last exit {last})")


def rust_run(root: Path, args: list[str]):
    return run([str(RUST_BIN), "--root", str(root), *args])


def snapshot(box: Path):
    return {
        "subscriptions.list": (box / "mihomo" / "subscriptions.list").read_text() if (box / "mihomo" / "subscriptions.list").exists() else "",
        "provider_files": files(box / "mihomo" / "proxy_provider"),
        "config_providers": (yaml_json(box / "mihomo" / "config.yaml") or {}).get("proxy-providers"),
        "manifest": (box / "run" / "subscriptions.providers").read_text() if (box / "run" / "subscriptions.providers").exists() else "",
    }


def compare_case(case: str, base: str, legacy_root: Path, rust_root: Path):
    legacy_box = prepare_root(legacy_root, legacy=True, fixture_base=base)
    rust_box = prepare_root(rust_root, legacy=False, fixture_base=base)
    fixture_file = {
        "normal": "normal.yaml",
        "normal.b64": "normal.b64",
        "aggregate": "aggregate.yaml",
        "mixed": "mixed.yaml",
        "empty": "empty.yaml",
        "fail": "does-not-exist.yaml",
    }[case]
    url = f"{base}/{fixture_file}"
    name = case
    lg = legacy_run(legacy_box, ["sub", "add", url, name])
    daemon = rust_start(rust_box)
    try:
        rg = rust_run(rust_box, ["sub", "add", url, name, "--json"])
        lg2 = legacy_run(legacy_box, ["sub", "update", name])
        rg2 = rust_run(rust_box, ["sub", "update", name, "--json"])
    finally:
        daemon.terminate()
        try:
            daemon.wait(timeout=5)
        except subprocess.TimeoutExpired:
            daemon.kill()
    result = {
        "case": case,
        "legacy": {"add_rc": lg.returncode, "add_tail": (lg.stdout + lg.stderr)[-400:], "update_rc": lg2.returncode, "update_tail": (lg2.stdout + lg2.stderr)[-800:], "snapshot": snapshot(legacy_box)},
        "rust": {"add_rc": rg.returncode, "add": rg.stdout[-400:], "update_rc": rg2.returncode, "update": rg2.stdout[-800:], "snapshot": snapshot(rust_box)},
    }
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, default=Path("/tmp/legacy-oracle-result.json"))
    args = parser.parse_args()
    fixture_root = Path(tempfile.mkdtemp(prefix="legacy-oracle-fixtures-"))
    for fixture in FIXTURES.iterdir():
        if fixture.is_file():
            shutil.copy2(fixture, fixture_root / fixture.name)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), lambda *a, **kw: http.server.SimpleHTTPRequestHandler(*a, directory=str(fixture_root), **kw))
    port = server.server_address[1]
    base = f"http://127.0.0.1:{port}"
    for fixture in fixture_root.glob("*.yaml"):
        text = fixture.read_text()
        text = re.sub(r"http://(?:fixture|127\.0\.0\.1:\d+)/", base + "/", text)
        fixture.write_text(text)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    results = []
    try:
        for case in ["normal", "normal.b64", "aggregate", "mixed", "empty", "fail"]:
            with tempfile.TemporaryDirectory(prefix="legacy-oracle-") as tmp:
                tmp = Path(tmp)
                legacy_root = tmp / "legacy"
                rust_root = tmp / "rust"
                legacy_root.mkdir()
                rust_root.mkdir()
                # The fixture path is normalized to .yaml; normal.b64 must be served as a file, not a YAML fixture.
                fixture_name = case
                results.append(compare_case(fixture_name, base, legacy_root, rust_root))
    finally:
        server.shutdown()
        shutil.rmtree(fixture_root, ignore_errors=True)
        args.out.write_text(json.dumps(results, ensure_ascii=False, indent=2))
    print(f"wrote {args.out}")
    for item in results:
        print(item["case"], "legacy add/update", item["legacy"]["add_rc"], item["legacy"]["update_rc"], "rust", item["rust"]["add_rc"], item["rust"]["update_rc"])


if __name__ == "__main__":
    main()
