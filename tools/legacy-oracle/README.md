# Legacy Subscription Oracle

This directory is a **behavior baseline**, not product code. It runs the sealed
legacy `box.tool` and Rust `boxctl` against the same subscription fixtures and
records the observable differences.

## Prerequisites

- `yq` and `python3` on the host
- Rust binary at `rust/target/release/boxctl`
- A host HTTP server is started by the harness; no device is required

## Run

```bash
cargo build --release --manifest-path rust/Cargo.toml
python3 tools/legacy-oracle/oracle.py --out /tmp/legacy-oracle-result.json
```

The harness:

1. Creates isolated temporary `BOX_HOME` roots;
2. runs legacy `sub add/update` with a busybox shim;
3. starts a temporary Rust daemon and runs the same operations;
4. normalizes provider YAML and config to JSON;
5. writes a diff-friendly result file.

`legacy-baseline.json` contains the observed legacy results and the current Rust
return codes. It is a snapshot, not a claim that Rust passes.

## Current baseline findings

- Legacy and Rust both accept `http://` subscription/provider URLs after the P0
  download-policy change.
- Normal subscription parity is now observed: both write a provider file,
  `proxy-providers`, and the same health-check block, with two proxies.
- Aggregate parity is now observed: both write `a.yaml`/`b.yaml` and their
  provider definitions. Rust intentionally adds the same health-check block
  that legacy writes for normal subscriptions; legacy aggregate definitions
  omit it.
- Mixed-source and empty-source cases are rejected by both. Rust reports a
  typed `E_CONTENT`/`E_DOWNLOAD`; legacy returns exit status 1.
- The base64 fixture exposes a legacy bug, not a Rust regression: `yq 'has("proxies")'`
  exits 0 for a scalar/base64 document, so legacy enters the proxies branch and
  rejects the file before trying `base64 -d`. Rust correctly accepts it. Do not
  use that row as a parity result.

## Limitations

- This harness covers subscription file/JSON behavior only.
- It does not validate network rules or real traffic.
- `nft` is not exercised.
- It must not be used as proof that the Rust rewrite is behavior-equivalent.
