#!/usr/bin/env bash
# 本地回归测试：在临时 BOX_HOME 中验证 box.tool sub 命令
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

setup() {
  mkdir -p "${BOX_HOME}/mihomo" "${BOX_HOME}/bin" "${BOX_HOME}/scripts" "${WORK}/bin" "${WORK}/fixtures"
  cp "${ROOT}/box/settings.ini" "${BOX_HOME}/settings.ini"
  cp "${ROOT}/box/mihomo/config.yaml" "${BOX_HOME}/mihomo/config.yaml"
  cp "${ROOT}/box/scripts/box.tool" "${BOX_HOME}/scripts/box.tool"

  cat > "${WORK}/bin/busybox" <<'EOF'
#!/usr/bin/env bash
applet="$1"; shift
exec "${applet}" "$@"
EOF
  chmod +x "${WORK}/bin/busybox"

  cat > "${WORK}/bin/curl" <<'EOF'
#!/usr/bin/env bash
out=""
url=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -w|--user-agent) shift 2 ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
if [ -z "${out}" ]; then
  echo "stub curl: missing -o" >&2
  exit 2
fi
case "${url}" in
  *fail*) printf '500'; exit 0 ;;
  *empty*) : > "${out}"; printf '200'; exit 0 ;;
esac
cp "${FIXTURE}" "${out}"
printf '200'
EOF
  chmod +x "${WORK}/bin/curl"
  export PATH="${WORK}/bin:${PATH}"

  cat > "${WORK}/fixtures/sub.yaml" <<'EOF'
proxies:
  - name: HK1
    type: ss
    server: 1.1.1.1
    port: 443
    cipher: aes-256-gcm
    password: pass
  - name: HK2
    type: ss
    server: 2.2.2.2
    port: 443
    cipher: aes-256-gcm
    password: pass
EOF
  export FIXTURE="${WORK}/fixtures/sub.yaml"
}

run_tool() {
  bash -O extglob "${BOX_HOME}/scripts/box.tool" "$@"
}

setup

# AC-006 模板无非法占位符
if grep -q '填写订阅链接' "${BOX_HOME}/mihomo/config.yaml"; then bad "AC-006 模板仍含非法占位符"; else ok "AC-006 模板无非法占位符"; fi

# AC-001 添加与列出
run_tool sub add "https://example.com/sub" "香港" >/dev/null 2>&1
assert_eq "香港|https://example.com/sub" "$(cat "${BOX_HOME}/mihomo/subscriptions.list")" "AC-001 清单写入"
run_tool sub list 2>&1 | grep -q "example.com/\*\*\*" && ok "AC-001 列表掩码 URL" || bad "AC-001 列表掩码 URL"

run_tool sub add "https://example.com/sub2" "香港" >/dev/null 2>&1
assert_eq "1" "$(wc -l < "${BOX_HOME}/mihomo/subscriptions.list" | tr -d ' ')" "AC-001 拒绝重名"

# AC-002 更新并生成 provider
run_tool sub update >/dev/null 2>&1
assert_eq "0" "$?" "AC-002 更新返回 0"
grep -q '^  - name: HK1$' "${BOX_HOME}/mihomo/proxy_provider/香港.yaml" && ok "AC-002 provider 含节点" || bad "AC-002 provider 含节点"
grep -q '^  "香港":' "${BOX_HOME}/mihomo/config.yaml" && ok "AC-002 config 含 provider" || bad "AC-002 config 含 provider"
grep -q 'path: ./proxy_provider/香港.yaml' "${BOX_HOME}/mihomo/config.yaml" && ok "AC-002 provider 路径正确" || bad "AC-002 provider 路径正确"

# AC-003 失败保留上一个可用 provider
cp "${BOX_HOME}/mihomo/proxy_provider/香港.yaml" "${WORK}/before.yaml"
printf '香港|https://example.com/fail\n' > "${BOX_HOME}/mihomo/subscriptions.list"
run_tool sub update >/dev/null 2>&1
assert_eq "1" "$?" "AC-003 全失败返回 1"
cmp -s "${WORK}/before.yaml" "${BOX_HOME}/mihomo/proxy_provider/香港.yaml" && ok "AC-003 provider 未被覆盖" || bad "AC-003 provider 未被覆盖"

# 部分成功返回 2
printf '香港|https://example.com/fail\n澳门|https://example.com/sub\n' > "${BOX_HOME}/mihomo/subscriptions.list"
run_tool sub update >/dev/null 2>&1
assert_eq "2" "$?" "AC-003 部分成功返回 2"

# AC-005 规则集前缀幂等
sed -i 's#^ruleset_prefix=.*#ruleset_prefix="https://ghfast.top"#' "${BOX_HOME}/settings.ini"
run_tool sub update >/dev/null 2>&1
prefix_count="$(grep -c 'url: "https://ghfast.top/https://raw.githubusercontent.com' "${BOX_HOME}/mihomo/config.yaml")"
assert_eq "6" "${prefix_count}" "AC-005 前缀应用 6 条"
run_tool sub update >/dev/null 2>&1
double_count="$(grep -c 'ghfast.top/https://ghfast.top' "${BOX_HOME}/mihomo/config.yaml")"
assert_eq "0" "${double_count}" "AC-005 重复执行不叠加"

sed -i 's#^ruleset_prefix=.*#ruleset_prefix=""#' "${BOX_HOME}/settings.ini"
run_tool sub update >/dev/null 2>&1
official_count="$(grep -c 'url: "https://raw.githubusercontent.com' "${BOX_HOME}/mihomo/config.yaml")"
assert_eq "6" "${official_count}" "AC-005 清空前缀恢复官方"

# AC-004 删除订阅
run_tool sub rm 香港 >/dev/null 2>&1
assert_eq "0" "$?" "AC-004 删除返回 0"
grep -q '^香港|' "${BOX_HOME}/mihomo/subscriptions.list" && bad "AC-004 清单仍含条目" || ok "AC-004 清单已移除"
[ -f "${BOX_HOME}/mihomo/proxy_provider/香港.yaml" ] && bad "AC-004 provider 仍存在" || ok "AC-004 provider 已删除"

printf '\n通过 %d 项，失败 %d 项\n' "${PASS}" "${FAIL}"
[ "${FAIL}" -eq 0 ]
