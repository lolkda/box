# boxctl

This directory contains the Rust `boxctl` daemon and CLI control plane packaged
into `box/bin/boxctl`.

## Scope

Implemented:

- `boxctl daemon` with a root-only Unix stream socket and NDJSON framing.
- v2 `{ok,data,error,meta}` envelope, `meta.version=2`, `meta.protocol=1`.
- `settings.get`, `settings.apply`, `settings.set`.
- `config.get/apply/delete` for proxy-groups/rule-providers/rules.
- `core.get/apply` for structured mihomo fields and raw YAML.
- `lists.get/apply` for package/gid/interface lists with multi-file rollback.
- `sub.list/get/add/set/rm/enable/disable`; UA overrides and provider state.
- `rule_profile.list/get/add/edit/remove/deactivate/url/set/clear/update/apply/use` local management, plus `rule_profile.fetch` via HTTPS.
- WebUI API client targets `/data/adb/box/bin/boxctl`; settings apply payload and rule-profile reload paths adapted.
- `box_service.sh` prefers the `boxctl` daemon with `BOXCTL_BOOT_AUTOSTART=1` and fails closed when the binary is absent; the legacy shell fallback has been sealed.
- `rust/package.sh` stages the aarch64 `boxctl` into `box/bin/boxctl`; `build.sh` excludes Rust source from the module zip. A fixture-dashboard ZIP build was verified to contain the binary, updated WebUI artifacts and omit `rust/`.
- `sub.update` for YAML-proxy, URI-list, base64 URI and aggregate proxy-provider subscriptions.
- `download` module: HTTPS-only, GitHub mirror rewrite, token host allowlist/cross-host stripping, size cap, SHA-256, atomic write, platform verifier.
- `task.start` for `sub-update`, plus `task.list/status/logs/cancel`; other task kinds are not yet implemented.
- `service.start/stop/restart` core process lifecycle, persistent transaction journal, candidate config staging, startup recovery, port/TUN health checks, finite supervisor backoff and external-controller `service.reload`; device network-rule validation and crash-point injection are pending.
- `network` capabilities, rule-plan generation for tun/tproxy/redirect/mixed/enhance, and a rollback-capable executor wired into `service.start/stop`; actual device rule execution and cleanup validation are pending.
- `upgrade.status/confirm`: root-only confirmation state, idempotent confirm, core start gate.
- `internal upgrade-prepare`: root-only backup of settings/token/legacy cron/logs, then settings reset with `boot_auto_start=false`, token/list reset, and `upgrade.required=true`; backup must succeed first.
- `uninstall.preflight`, token-gated `purge.plan/apply` with path whitelist and symlink avoidance; platform uninstall/force hooks still pending.
- `schedule` cron parser, allowlisted sub-update/geox planning, bounded missed-run state, status, and a daemon 30s timer that starts the allowlisted `sub-update` task; Wi-Fi/network event parsing, 2.5s debounce, 3s stop/auto-start control loop with bounded backoff and stop-priority task interruption exist; retry/geox execution and manual/auto control-mode UX are pending.
- `logs.get/clear`, `reset.subscriptions`.
- `status.get`, `version.get`; `boxctl status` has a degraded local fallback.
- `settings.json` defaults from `settings.default.json`; schema/default key consistency.
- Manual validation matching the 2.0.0 shell rules and the documented schema gaps.
- Atomic `settings.json`/`githubtoken` writes with rollback of settings when token commit fails.
- `githubtoken` is set-only in `settings.get`; no preview is returned.
- Persistent revision at `state/revision`; data-plane mutations bump it.
- Host unit and socket/CLI integration tests: 106 unit + 27 integration = 133.

Not implemented yet:

- non-sub-update task kinds;
- installer/upgrade application, platform uninstall/force hooks;
- actual iptables/ipset/TUN execution;
- Wi-Fi auto-start/retry, geox execution and scheduler stop-priority;
- Device installation/deployment and a full installer flow;
- device deployment.

## Build and test

```sh
cd rust
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Android arm64 build

```sh
cd rust
./build-android.sh
file target/aarch64-linux-android/release/boxctl
```

The binary is built for `aarch64-linux-android24` with the NDK referenced by
`ANDROID_NDK_HOME` or the default NDK path used by this environment. It is not deployed to a
device by this script.

## Manual smoke test

```sh
root=$(mktemp -d)
cp ../box/settings.default.json "$root/settings.default.json"
cp ../box/settings.schema.json "$root/settings.schema.json"
cp ../box/settings.default.json "$root/settings.json"
chmod 0600 "$root/settings.json"
./target/debug/boxctl daemon --root "$root" --socket "$root/run/boxctl/control.sock"
# in another shell:
./target/debug/boxctl settings get --json --socket "$root/run/boxctl/control.sock"
```

The daemon is root-only; the host smoke test must run as root. No HTTP/TCP listener is created.
