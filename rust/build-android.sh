#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
NDK=${ANDROID_NDK_HOME:-/opt/android-sdk/ndk/29.0.14206865}
HOST_TAG=linux-x86_64
TOOLCHAIN_BIN="${NDK}/toolchains/llvm/prebuilt/${HOST_TAG}/bin"
CC="${TOOLCHAIN_BIN}/aarch64-linux-android24-clang"
AR="${TOOLCHAIN_BIN}/llvm-ar"
TARGET=aarch64-linux-android

[ -x "${CC}" ] || {
  echo "Android NDK linker not found: ${CC}" >&2
  exit 1
}

export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="${CC}"
export CC_aarch64_linux_android="${CC}"
export AR_aarch64_linux_android="${AR}"
export RANLIB_aarch64_linux_android="${TOOLCHAIN_BIN}/llvm-ranlib"
cd "${ROOT_DIR}"
cargo build --locked --target "${TARGET}" --release "$@"
