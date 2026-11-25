#!/bin/sh
# Build the Android boxctl binary and stage it for the module zip.
set -eu

ROOT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
TARGET_BIN="${ROOT_DIR}/target/aarch64-linux-android/release/boxctl"
MODULE_BIN="${ROOT_DIR}/../box/bin/boxctl"

if [ ! -x "${TARGET_BIN}" ]; then
  echo "boxctl binary missing; running build-android.sh"
  "${ROOT_DIR}/build-android.sh"
fi
[ -x "${TARGET_BIN}" ] || {
  echo "package failed: ${TARGET_BIN} is not executable" >&2
  exit 1
}

mkdir -p "${ROOT_DIR}/../box/bin"
cp "${TARGET_BIN}" "${MODULE_BIN}"
chmod 0755 "${MODULE_BIN}"
echo "staged ${MODULE_BIN}"
