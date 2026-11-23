#!/usr/bin/env bash
# 本地回归测试：验证管理页面依赖的 box.tool JSON 契约、校验与任务生命周期
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
  local needle="$1" haystack="$2" label="$3"
  if printf '%s' "${haystack}" | grep -Fq "${needle}"; then ok "${label}"; else bad "${label} (missing=${needle})"; fi
}

jget() {
  jq -r "$1" <<<"$2"
}

jassert() {
  local expression="$1" payload="$2" label="$3"
  if printf '%s' "${payload}" | jq -e "${expression}" >/dev/null 2>&1; then
    ok "${label}"
  else
    bad "${label}"
  fi
}

run_tool() { bash -O extglob "${BOX_HOME}/scripts/box.tool" "$@"; }

setup() {
  mkdir -p "${BOX_HOME}/mihomo" "${BOX_HOME}/bin" "${BOX_HOME}/scripts" "${WORK}/bin"
  cp "${ROOT}/box/settings.ini" "${BOX_HOME}/settings.ini"
  cp "${ROOT}/box/mihomo/config.yaml" "${BOX_HOME}/mihomo/config.yaml"
  cp "${ROOT}/box/scripts/box.tool" "${BOX_HOME}/scripts/box.tool"
  cp "${ROOT}/box/scripts/box.service" "${BOX_HOME}/scripts/box.service"
  chmod +x "${BOX_HOME}/scripts/box.tool" "${BOX_HOME}/scripts/box.service"

  cat > "${WORK}/bin/busybox" <<'EOF'
#!/usr/bin/env bash
applet="$1"; shift
exec "${applet}" "$@"
EOF
  chmod +x "${WORK}/bin/busybox"
  export PATH="${WORK}/bin:${PATH}"
}

setup

# ---------------------------------------------------------------------------
# 1. 统一 JSON 包络
# ---------------------------------------------------------------------------
status_json="$(run_tool status --json 2>/dev/null)"
jassert '.ok == true and .meta.version == 1 and .data.running == false and .data.core == "mihomo"' "${status_json}" "status 返回统一成功包络"
jassert '.data.config_exists == true and .data.active_tasks == 0' "${status_json}" "status 包含 config_exists/active_tasks"

# ---------------------------------------------------------------------------
# 2. 订阅 JSON 接口
# ---------------------------------------------------------------------------
add_json="$(run_tool sub add "https://example.com/sub" "香港" --json 2>/dev/null)"
jassert '.ok == true and .data.name == "香港" and .data.url_masked == "https://example.com/***"' "${add_json}" "sub add 返回掩码 URL"

list_json="$(run_tool sub list --json 2>/dev/null)"
jassert '.data.items[0].name == "香港" and .data.items[0].provider == "missing"' "${list_json}" "sub list 返回 provider 状态"

get_json="$(run_tool sub get "香港" --json 2>/dev/null)"
jassert '.data.url == "https://example.com/sub"' "${get_json}" "sub get 编辑时才返回完整 URL"

set_json="$(run_tool sub set "香港" "香港2" "https://example.com/sub2" --json 2>/dev/null)"
jassert '.ok == true and .data.name == "香港2" and .data.renamed == false' "${set_json}" "sub set 改名并替换 URL"

list_json="$(run_tool sub list --json 2>/dev/null)"
jassert '.data.items[0].name == "香港2" and (.data.items[0].url_masked | test("/\\*\\*\\*$"))' "${list_json}" "sub set 后列表更新"

run_tool sub rm "香港2" --json >/dev/null 2>&1
list_json="$(run_tool sub list --json 2>/dev/null)"
jassert '.data.items | length == 0' "${list_json}" "sub rm 删除订阅"

# ---------------------------------------------------------------------------
# 3. 设置 JSON 接口与后端校验
# ---------------------------------------------------------------------------
settings_json="$(run_tool settings get --json 2>/dev/null)"
jassert '.ok == true and (.data.settings | has("network_mode")) and (.data.settings | has("githubtoken") | not)' "${settings_json}" "settings get 不返回 token 明文"
jassert '(.data.arrays | has("wifi_ssids_list")) and .data.sensitive.githubtoken.set == false' "${settings_json}" "settings get 返回数组和敏感状态"

cat > "${WORK}/settings.json" <<'EOF'
{"network_mode":"tproxy","wifi_ssids_list":["My Home WiFi","Cafe 'Test'"],"githubtoken":"abcdef1234567890"}
EOF
settings_result="$(run_tool settings apply "${WORK}/settings.json" --json 2>/dev/null)"
jassert '.ok == true and (.data.changed | index("network_mode")) != null' "${settings_result}" "settings apply 返回 changed"

settings_after="$(run_tool settings get --json 2>/dev/null)"
jassert '.data.settings.network_mode == "tproxy" and .data.sensitive.githubtoken.set == true' "${settings_after}" "settings 写入后读取一致"

cat > "${WORK}/check-array.sh" <<'EOF'
source "${BOX_HOME}/settings.ini" >/dev/null 2>&1
[ "${#wifi_ssids_list[@]}" -eq 2 ] || exit 1
[ "${wifi_ssids_list[0]}" = "My Home WiFi" ] || exit 1
[ "${wifi_ssids_list[1]}" = "Cafe 'Test'" ] || exit 1
EOF
if BOX_HOME="${BOX_HOME}" bash "${WORK}/check-array.sh"; then
  ok "settings 数组正确转义空格与单引号"
else
  bad "settings 数组正确转义空格与单引号"
fi
assert_eq "githubtoken='abcdef1234567890'" "$(grep '^githubtoken=' "${BOX_HOME}/settings.ini")" "settings 写入 githubtoken"
if bash -n "${BOX_HOME}/settings.ini" 2>/dev/null; then ok "settings.ini 语法可解析"; else bad "settings.ini 语法可解析"; fi

cat > "${WORK}/bad-settings.json" <<'EOF'
{"network_mode":"bad-mode"}
EOF
before_hash="$(md5sum "${BOX_HOME}/settings.ini" | cut -d' ' -f1)"
bad_result="$(run_tool settings apply "${WORK}/bad-settings.json" --json 2>/dev/null)"
after_hash="$(md5sum "${BOX_HOME}/settings.ini" | cut -d' ' -f1)"
jassert '.ok == false and .error.code == "E_VALIDATION"' "${bad_result}" "非法枚举返回 E_VALIDATION"
assert_eq "${before_hash}" "${after_hash}" "非法设置不修改文件"

cat > "${WORK}/unknown-settings.json" <<'EOF'
{"not_a_real_key":"x"}
EOF
unknown_result="$(run_tool settings apply "${WORK}/unknown-settings.json" --json 2>/dev/null)"
jassert '.ok == false and .error.code == "E_VALIDATION"' "${unknown_result}" "未知设置键被拒绝"

# ---------------------------------------------------------------------------
# 4. 配置 JSON 接口与引用校验
# ---------------------------------------------------------------------------
config_json="$(run_tool config get --json 2>/dev/null)"
jassert '.ok == true and (.data["proxy-groups"] | length) == 3 and (.data.rules | length) == 4' "${config_json}" "config get 返回策略组与规则"

config_data="${WORK}/config-data.json"
printf '%s' "${config_json}" | jq '.data' > "${config_data}"
jq '.rules += ["DOMAIN,example.com,国外代理"]' "${config_data}" > "${WORK}/config-apply.json"
apply_result="$(run_tool config apply "${WORK}/config-apply.json" --json 2>/dev/null)"
jassert '.ok == true' "${apply_result}" "config apply 合法规则成功"
grep -q 'DOMAIN,example.com,国外代理' "${BOX_HOME}/mihomo/config.yaml" && ok "config apply 写入新规则" || bad "config apply 写入新规则"

before_hash="$(md5sum "${BOX_HOME}/mihomo/config.yaml" | cut -d' ' -f1)"
jq '.rules += ["RULE-SET,no_such_provider,国外代理"]' "${config_data}" > "${WORK}/config-bad-ref.json"
bad_ref_result="$(run_tool config apply "${WORK}/config-bad-ref.json" --json 2>/dev/null)"
after_hash="$(md5sum "${BOX_HOME}/mihomo/config.yaml" | cut -d' ' -f1)"
jassert '.ok == false and .error.code == "E_VALIDATION"' "${bad_ref_result}" "缺失规则集引用被拒绝"
assert_eq "${before_hash}" "${after_hash}" "非法 config 不修改文件"

jq '. ["proxy-groups"] += [. ["proxy-groups"][0]]' "${config_data}" > "${WORK}/config-dup.json"
dup_result="$(run_tool config apply "${WORK}/config-dup.json" --json 2>/dev/null)"
jassert '.ok == false and .error.code == "E_VALIDATION"' "${dup_result}" "重复策略组名称被拒绝"

jq '. ["proxy-groups"] += [{"type":"select"}]' "${config_data}" > "${WORK}/config-empty-name.json"
empty_name_result="$(run_tool config apply "${WORK}/config-empty-name.json" --json 2>/dev/null)"
jassert '.ok == false and .error.code == "E_VALIDATION"' "${empty_name_result}" "空策略组名称被拒绝"

# ---------------------------------------------------------------------------
# 5. 日志 JSON 接口
# ---------------------------------------------------------------------------
mkdir -p "${BOX_HOME}/run"
printf 'line1\nline2\nline3\n' > "${BOX_HOME}/run/runs.log"
logs_json="$(run_tool logs 2 --json 2>/dev/null)"
jassert '.ok == true and (.data.lines | length) == 2 and .data.lines[1] == "line3"' "${logs_json}" "logs 返回行数组"

# ---------------------------------------------------------------------------
# 6. 后台任务生命周期
# ---------------------------------------------------------------------------
task_json="$(run_tool task start sub-update --json 2>/dev/null)"
jassert '.ok == true and .data.state == "running" and .data.cancelable == true' "${task_json}" "task start 返回运行中任务"
task_id="$(jget '.data.id' "${task_json}")"
for _ in 1 2 3 4 5 6 7 8 9 10; do
  task_status="$(run_tool task status "${task_id}" --json 2>/dev/null)"
  state="$(jget '.data.state' "${task_status}")"
  [ "${state}" = "success" ] && break
  sleep 0.5
done
assert_eq "success" "${state}" "task status 最终成功"
task_logs="$(run_tool task logs "${task_id}" 50 --json 2>/dev/null)"
jassert '.ok == true and (.data.lines | length) >= 1' "${task_logs}" "task logs 返回输出"
task_list="$(run_tool task list --json 2>/dev/null)"
jassert '(.data.items | length) >= 1 and .data.items[0].id == "'"${task_id}"'"' "${task_list}" "task list 返回历史任务"

# 任务互斥与取消：用会阻塞的 curl 桩保证第一个任务仍在运行
cat > "${WORK}/bin/curl" <<'EOF'
#!/usr/bin/env bash
sleep 3
exit 22
EOF
chmod +x "${WORK}/bin/curl"
run_tool sub add "https://slow.example.com/sub" "慢订阅" --json >/dev/null 2>&1
slow_task="$(run_tool task start sub-update --json 2>/dev/null)"
slow_id="$(jget '.data.id' "${slow_task}")"
sleep 0.3
busy_result="$(run_tool task start sub-update --json 2>/dev/null)"
jassert '.ok == false and .error.code == "E_BUSY"' "${busy_result}" "并行任务返回 E_BUSY"
cancel_result="$(run_tool task cancel "${slow_id}" --json 2>/dev/null)"
jassert '.ok == true and (.data.state == "canceled" or .data.state == "failed")' "${cancel_result}" "可取消任务能被取消"
rm -f "${WORK}/bin/curl"

cat > "${WORK}/bad-array.json" <<'EOF'
{"force_proxy_cidrs":["not-a-cidr"]}
EOF
bad_array_result="$(run_tool settings apply "${WORK}/bad-array.json" --json 2>/dev/null)"
jassert '.ok == false and .error.code == "E_VALIDATION"' "${bad_array_result}" "非法数组项被拒绝"

# ---------------------------------------------------------------------------
# 7. 服务包装、日志清空与重置
# ---------------------------------------------------------------------------
cat > "${BOX_HOME}/scripts/box.service" <<'EOF'
#!/usr/bin/env bash
echo "stub:$1"
EOF
chmod +x "${BOX_HOME}/scripts/box.service"
service_json="$(run_tool service start --json 2>/dev/null)"
jassert '.ok == true and .data.action == "start" and (.data.output | contains("stub:start"))' "${service_json}" "service 包装返回统一包络"

printf 'one\ntwo\n' > "${BOX_HOME}/run/runs.log"
clear_json="$(run_tool logs clear --json 2>/dev/null)"
jassert '.ok == true and (.data.cleared | length) >= 1' "${clear_json}" "logs clear 返回清理结果"
assert_eq "0" "$(wc -c < "${BOX_HOME}/run/runs.log" | tr -d ' ')" "logs clear 清空文件"

reset_json="$(run_tool reset subscriptions --json 2>/dev/null)"
jassert '.ok == true and .data.cleared == true' "${reset_json}" "reset subscriptions 返回包络"

delete_json="$(run_tool config delete --json 2>/dev/null)"
jassert '.ok == true and (.data.path | contains("config.yaml.deleted"))' "${delete_json}" "config delete 返回备份路径"

# ---------------------------------------------------------------------------
# 8. CLI 文本兼容（不带 --json）
# ---------------------------------------------------------------------------
run_tool sub add "https://example.com/cli" "CLI订阅" >/dev/null 2>&1
cli_list="$(run_tool sub list 2>/dev/null)"
assert_contains "CLI订阅" "${cli_list}" "sub list 文本输出保留"
assert_contains "https://example.com/***" "${cli_list}" "sub list 文本仍掩码 URL"

printf '\n通过 %d 项，失败 %d 项\n' "${PASS}" "${FAIL}"
[ "${FAIL}" -eq 0 ]
