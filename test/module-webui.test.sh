#!/usr/bin/env bash
# 本地回归测试：验证管理页面依赖的 box.tool JSON 接口
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

json_get() {
  node -e "const v=JSON.parse(require('fs').readFileSync(0,'utf8')); console.log(eval('v.'+process.argv[1]))" "$1"
}

setup() {
  mkdir -p "${BOX_HOME}/mihomo" "${BOX_HOME}/bin" "${BOX_HOME}/scripts" "${WORK}/bin"
  cp "${ROOT}/box/settings.ini" "${BOX_HOME}/settings.ini"
  cp "${ROOT}/box/mihomo/config.yaml" "${BOX_HOME}/mihomo/config.yaml"
  cp "${ROOT}/box/scripts/box.tool" "${BOX_HOME}/scripts/box.tool"
  cp "${ROOT}/box/scripts/box.service" "${BOX_HOME}/scripts/box.service"

  cat > "${WORK}/bin/busybox" <<'EOF'
#!/usr/bin/env bash
applet="$1"; shift
exec "${applet}" "$@"
EOF
  chmod +x "${WORK}/bin/busybox"
  export PATH="${WORK}/bin:${PATH}"
}

run_tool() { bash -O extglob "${BOX_HOME}/scripts/box.tool" "$@"; }

setup

# 1 status --json
status_json="$(run_tool status --json 2>/dev/null)"
assert_eq "false" "$(printf '%s' "${status_json}" | json_get running)" "status JSON running=false"
assert_eq "mihomo" "$(printf '%s' "${status_json}" | json_get core)" "status JSON core=mihomo"

# 2 sub list --json
run_tool sub add "https://example.com/sub" "香港" >/dev/null 2>&1
subs_json="$(run_tool sub list --json 2>/dev/null)"
assert_eq "香港" "$(printf '%s' "${subs_json}" | node -e "const v=JSON.parse(require('fs').readFileSync(0,'utf8'));console.log(v.items[0].name)")" "sub list JSON name"
assert_eq "未下载" "$(printf '%s' "${subs_json}" | node -e "const v=JSON.parse(require('fs').readFileSync(0,'utf8'));console.log(v.items[0].provider)")" "sub list JSON provider"

# 3 settings get --json
settings_json="$(run_tool settings get --json 2>/dev/null)"
assert_eq "tun" "$(printf '%s' "${settings_json}" | node -e "const v=JSON.parse(require('fs').readFileSync(0,'utf8'));console.log(v.settings.network_mode)")" "settings JSON network_mode"

# 4 settings apply
cat > "${WORK}/settings.json" <<'EOF'
{"network_mode":"tproxy","proxy_mode":"blacklist"}
EOF
run_tool settings apply "${WORK}/settings.json" >/dev/null 2>&1
assert_eq "0" "$?" "settings apply 返回 0"
assert_eq 'network_mode="tproxy"' "$(grep '^network_mode=' "${BOX_HOME}/settings.ini")" "settings 写入 network_mode"
assert_eq 'proxy_mode="blacklist"' "$(grep '^proxy_mode=' "${BOX_HOME}/settings.ini")" "settings 写入 proxy_mode"
assert_eq 'tproxy_port="9898"' "$(grep '^tproxy_port=' "${BOX_HOME}/settings.ini")" "settings 未改动其他键"

# 5 config get --json
config_json="$(run_tool config get --json 2>/dev/null)"
assert_eq "3" "$(printf '%s' "${config_json}" | node -e "const v=JSON.parse(require('fs').readFileSync(0,'utf8'));console.log(v['proxy-groups'].length)")" "config get 策略组数量"
assert_eq "4" "$(printf '%s' "${config_json}" | node -e "const v=JSON.parse(require('fs').readFileSync(0,'utf8'));console.log(v.rules.length)")" "config get 规则数量"

# 6 config apply 保留其他段
node -e "
const fs=require('fs');
const c=JSON.parse(process.argv[1]);
c.rules.push('DOMAIN,example.com,国外代理');
fs.writeFileSync(process.argv[2], JSON.stringify(c));
" "${config_json}" "${WORK}/config.json"
run_tool config apply "${WORK}/config.json" >/dev/null 2>&1
assert_eq "0" "$?" "config apply 返回 0"
grep -q 'DOMAIN,example.com,国外代理' "${BOX_HOME}/mihomo/config.yaml" && ok "config apply 写入新规则" || bad "config apply 写入新规则"
grep -q '^mixed-port: 7890' "${BOX_HOME}/mihomo/config.yaml" && ok "config apply 保留 mixed-port" || bad "config apply 保留 mixed-port"

# 7 非法 JSON 拒绝
before_hash="$(md5sum "${BOX_HOME}/mihomo/config.yaml" | cut -d' ' -f1)"
printf '{"proxy-groups":[]}' > "${WORK}/bad.json"
run_tool config apply "${WORK}/bad.json" >/dev/null 2>&1
assert_eq "1" "$?" "非法 config 返回 1"
after_hash="$(md5sum "${BOX_HOME}/mihomo/config.yaml" | cut -d' ' -f1)"
assert_eq "${before_hash}" "${after_hash}" "非法 config 不修改文件"

# 8 logs --json
mkdir -p "${BOX_HOME}/run"
printf 'line1\nline2\nline3\n' > "${BOX_HOME}/run/runs.log"
logs_json="$(run_tool logs 2 --json 2>/dev/null)"
assert_eq "2" "$(printf '%s' "${logs_json}" | node -e "const v=JSON.parse(require('fs').readFileSync(0,'utf8'));console.log(v.lines.length)")" "logs 返回行数"
assert_eq "line3" "$(printf '%s' "${logs_json}" | node -e "const v=JSON.parse(require('fs').readFileSync(0,'utf8'));console.log(v.lines[1])")" "logs 末行"

# 9 危险操作：删除配置
run_tool config delete >/dev/null 2>&1
assert_eq "0" "$?" "config delete 返回 0"
[ -f "${BOX_HOME}/mihomo/config.yaml" ] && bad "config delete 后文件仍存在" || ok "config delete 已移走配置"

# 10 reset subscriptions
run_tool reset subscriptions >/dev/null 2>&1
assert_eq "0" "$?" "reset subscriptions 返回 0"
[ -s "${BOX_HOME}/mihomo/subscriptions.list" ] && bad "reset 后清单非空" || ok "reset 清空订阅清单"

printf '\n通过 %d 项，失败 %d 项\n' "${PASS}" "${FAIL}"
[ "${FAIL}" -eq 0 ]
