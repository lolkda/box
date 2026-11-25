#!/usr/bin/env bash
# 本地回归测试：验证 mihomo /ui 固定 zashboard 的资源替换与配置清理
set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(mktemp -d)"
export BOX_HOME="${WORK}/box"
PASS=0
FAIL=0

cleanup() { rm -rf "${WORK}"; }
trap cleanup EXIT

ok() { PASS=$((PASS + 1)); printf 'ok   - %s\n' "$1"; }
bad() { FAIL=$((FAIL + 1)); printf 'FAIL - %s\n' "$1"; }

assert_eq() {
  local expected="$1" actual="$2" label="$3"
  if [ "${expected}" = "${actual}" ]; then ok "${label}"; else bad "${label} (expected=${expected} actual=${actual})"; fi
}

assert_contains() {
  local needle="$1" file="$2" label="$3"
  if grep -Fq "${needle}" "${file}"; then ok "${label}"; else bad "${label} (missing=${needle})"; fi
}

assert_not_contains() {
  local needle="$1" file="$2" label="$3"
  if grep -Fq "${needle}" "${file}"; then bad "${label} (unexpected=${needle})"; else ok "${label}"; fi
}

setup() {
  mkdir -p "${BOX_HOME}/mihomo/dashboard" "${BOX_HOME}/scripts" "${WORK}/bin" "${WORK}/fixture/dist"
  cp "${ROOT}/box/settings.json" "${BOX_HOME}/settings.json"
  cp "${ROOT}/box/settings.default.json" "${BOX_HOME}/settings.default.json"
  cp "${ROOT}/box/settings.schema.json" "${BOX_HOME}/settings.schema.json"
  cp "${ROOT}/box/settings.base.sh" "${BOX_HOME}/settings.base.sh"
  cp "${ROOT}/box/scripts/settings-gen.awk" "${BOX_HOME}/scripts/settings-gen.awk"
  cp "${ROOT}/box/mihomo/config.yaml" "${BOX_HOME}/mihomo/config.yaml"
  cp "${ROOT}/box/scripts/box.tool" "${BOX_HOME}/scripts/box.tool"
  chmod +x "${BOX_HOME}/scripts/box.tool"

  cat > "${WORK}/bin/busybox" <<'BUSYBOX_EOF'
#!/usr/bin/env bash
applet="$1"; shift
exec "${applet}" "$@"
BUSYBOX_EOF
  cat > "${WORK}/bin/curl" <<'CURL_EOF'
#!/usr/bin/env bash
if [ "${FAKE_CURL_FAIL:-0}" = "1" ]; then
  exit 22
fi
out=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    *) shift ;;
  esac
done
cp "${FAKE_ZASHBOARD_ZIP}" "${out}"
printf '200'
CURL_EOF
  cat > "${WORK}/bin/wget" <<'WGET_EOF'
#!/usr/bin/env bash
if [ "${FAKE_CURL_FAIL:-0}" = "1" ]; then
  exit 22
fi
exec /usr/bin/wget "$@"
WGET_EOF
  chmod +x "${WORK}/bin/busybox" "${WORK}/bin/curl" "${WORK}/bin/wget"
  export PATH="${WORK}/bin:${PATH}"

  printf '<!doctype html><html><head><title>zashboard</title></head><body>zashboard</body></html>\n' > "${WORK}/fixture/dist/index.html"
  printf 'asset\n' > "${WORK}/fixture/dist/app.js"
  (cd "${WORK}/fixture" && zip -qr "${WORK}/zashboard.zip" dist)
  export FAKE_ZASHBOARD_ZIP="${WORK}/zashboard.zip"

  printf '<html><title>METACUBE (XD)</title></html>\n' > "${BOX_HOME}/mihomo/dashboard/index.html"
  sed -i 's|^external-ui:.*|external-ui: ./ui|' "${BOX_HOME}/mihomo/config.yaml"
  printf '\nexternal-ui-url: "https://example.com/metacubexd.zip"\n' >> "${BOX_HOME}/mihomo/config.yaml"
}

run_tool() { bash -O extglob "${BOX_HOME}/scripts/box.tool" "$@"; }

setup

run_tool upxui >/tmp/module-dashboard-upxui-success.log 2>&1
assert_eq "0" "$?" "upxui 执行成功"
assert_contains "zashboard" "${BOX_HOME}/mihomo/dashboard/index.html" "面板目录替换为 zashboard"
assert_contains "external-ui: ./dashboard" "${BOX_HOME}/mihomo/config.yaml" "external-ui 强制为标准路径"
assert_not_contains "external-ui-url:" "${BOX_HOME}/mihomo/config.yaml" "旧 external-ui-url 已删除"

printf '<html><title>KEEP</title></html>\n' > "${BOX_HOME}/mihomo/dashboard/index.html"
export FAKE_CURL_FAIL=1
run_tool upxui >/tmp/module-dashboard-upxui-failure.log 2>&1
unset FAKE_CURL_FAIL
assert_contains "下载 Dashboard 失败" /tmp/module-dashboard-upxui-failure.log "upxui 记录下载失败"
assert_contains "KEEP" "${BOX_HOME}/mihomo/dashboard/index.html" "下载失败保留当前面板"

printf '通过 %d 项，失败 %d 项\n' "${PASS}" "${FAIL}"
[ "${FAIL}" -eq 0 ]
