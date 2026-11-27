#!/bin/sh

# 构建脚本：自动打包 Box 模块为 zip 文件
set -eu

ROOT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
cd "${ROOT_DIR}"

VERSION=$(grep '^version=' module.prop | awk -F '=' '{print $2}' | tr -d '\r')
[ -n "${VERSION}" ] || {
  echo "无法从 module.prop 读取版本号" >&2
  exit 1
}

ZASHBOARD_URL="${ZASHBOARD_URL:-https://github.com/Zephyruso/zashboard/releases/latest/download/dist-no-fonts.zip}"
WORK_DIR=$(mktemp -d "${TMPDIR:-/tmp}/box-build.XXXXXX")
cleanup() {
  rm -rf "${WORK_DIR}"
}
trap cleanup EXIT HUP INT TERM

ARCHIVE="${ZASHBOARD_ARCHIVE:-${WORK_DIR}/dist-no-fonts.zip}"
if [ -z "${ZASHBOARD_ARCHIVE:-}" ]; then
  echo "下载 zashboard: ${ZASHBOARD_URL}"
  if command -v curl >/dev/null 2>&1; then
    curl --fail --location --silent --show-error --output "${ARCHIVE}" "${ZASHBOARD_URL}"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "${ARCHIVE}" "${ZASHBOARD_URL}"
  else
    echo "构建失败：需要 curl 或 wget 下载 zashboard" >&2
    exit 1
  fi
else
  echo "使用本地 zashboard 包: ${ZASHBOARD_ARCHIVE}"
fi

[ -s "${ARCHIVE}" ] || {
  echo "构建失败：zashboard 下载文件为空" >&2
  exit 1
}

EXTRACT_DIR="${WORK_DIR}/extract"
mkdir -p "${EXTRACT_DIR}"
if ! unzip -q "${ARCHIVE}" -d "${EXTRACT_DIR}"; then
  echo "构建失败：zashboard 压缩包解压失败" >&2
  exit 1
fi

INDEX_HTML=$(find "${EXTRACT_DIR}" -maxdepth 3 -type f -name index.html | head -n 1)
[ -n "${INDEX_HTML}" ] || {
  echo "构建失败：zashboard 压缩包中未找到 index.html" >&2
  exit 1
}
if ! grep -qi 'zashboard' "${INDEX_HTML}"; then
  echo "构建失败：zashboard index.html 内容校验失败" >&2
  exit 1
fi
DASHBOARD_SOURCE=${INDEX_HTML%/*}

DASHBOARD_STAGE="${WORK_DIR}/box/mihomo/dashboard"
mkdir -p "${DASHBOARD_STAGE}"
cp -a "${DASHBOARD_SOURCE}/." "${DASHBOARD_STAGE}/"

ZIP_NAME="box-${VERSION}.zip"
if [ ! -x "${ROOT_DIR}/box/bin/boxctl" ]; then
  if [ -x "${ROOT_DIR}/rust/package.sh" ]; then
    "${ROOT_DIR}/rust/package.sh"
  else
    echo "构建失败：缺少可执行文件 box/bin/boxctl" >&2
    exit 1
  fi
fi
rm -f "${ZIP_NAME}"
zip -r -o -X -ll "${ZIP_NAME}" ./ \
  -x '.git' \
  -x '.git/*' \
  -x '.gitignore' \
  -x 'CHANGELOG.md' \
  -x 'update.json' \
  -x 'build.sh' \
  -x '.github/*' \
  -x 'docs/*' \
  -x 'test/*' \
  -x 'tools/*' \
  -x 'webui/*' \
  -x 'rust/*' \
  -x 'legacy/*' \
  -x 'artifacts/*' \
  -x '*.zip' \
  -x 'LICENSE' \
  -x 'box/mihomo/dashboard/*'
(
  cd "${WORK_DIR}"
  zip -r -X -ll "${ROOT_DIR}/${ZIP_NAME}" box/mihomo/dashboard
)
unzip -l "${ZIP_NAME}" | grep -q 'box/mihomo/dashboard/index.html' || {
  echo "构建失败：模块包中缺少 zashboard/index.html" >&2
  exit 1
}
unzip -l "${ZIP_NAME}" | grep -q 'box/bin/boxctl' || {
  echo "构建失败：模块包中缺少 box/bin/boxctl" >&2
  exit 1
}

echo "构建完成: ${ZIP_NAME}"
