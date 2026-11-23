#!/usr/bin/env bash
# 本地回归测试：验证管理页面依赖的 box.tool JSON 契约、校验与任务生命周期
set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(mktemp -d)"
export WORK
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
  mkdir -p "${BOX_HOME}/mihomo" "${BOX_HOME}/sing-box" "${BOX_HOME}/bin" "${BOX_HOME}/scripts" "${WORK}/bin"
  cp "${ROOT}/box/settings.json" "${BOX_HOME}/settings.json"
  cp "${ROOT}/box/settings.default.json" "${BOX_HOME}/settings.default.json"
  cp "${ROOT}/box/settings.schema.json" "${BOX_HOME}/settings.schema.json"
  cp "${ROOT}/box/settings.base.sh" "${BOX_HOME}/settings.base.sh"
  cp "${ROOT}/box/scripts/settings-gen.awk" "${BOX_HOME}/scripts/settings-gen.awk"
  cp "${ROOT}/box/mihomo/config.yaml" "${BOX_HOME}/mihomo/config.yaml"
  cp "${ROOT}/box/sing-box/config.json" "${BOX_HOME}/sing-box/config.json"
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

disable_json="$(run_tool sub disable "香港" --json 2>/dev/null)"
jassert '.ok == true and .data.enabled == false and .data.reloaded == false' "${disable_json}" "sub disable 返回禁用状态"
list_disabled="$(run_tool sub list --json 2>/dev/null)"
jassert '.data.items[0].enabled == false' "${list_disabled}" "sub list 显示已禁用"
yq -e '."proxy-providers" | has("香港") | not' "${BOX_HOME}/mihomo/config.yaml" >/dev/null 2>&1 && ok "禁用后移除 proxy-provider" || bad "禁用后移除 proxy-provider"
enable_json="$(run_tool sub enable "香港" --json 2>/dev/null)"
jassert '.ok == true and .data.enabled == true' "${enable_json}" "sub enable 返回启用状态"
list_enabled="$(run_tool sub list --json 2>/dev/null)"
jassert '.data.items[0].enabled == true' "${list_enabled}" "sub list 显示已启用"
yq -e '."proxy-providers" | has("香港")' "${BOX_HOME}/mihomo/config.yaml" >/dev/null 2>&1 && ok "启用后恢复 proxy-provider" || bad "启用后恢复 proxy-provider"
cp "${BOX_HOME}/mihomo/config.yaml" "${WORK}/config-before-use.yaml"
yq -i '."proxy-groups"[0].use = ["香港"]' "${BOX_HOME}/mihomo/config.yaml"
disable_ref="$(run_tool sub disable "香港" --json 2>/dev/null)"
jassert '.ok == false and .error.code == "E_VALIDATION" and (.error.message | contains("香港"))' "${disable_ref}" "策略组引用 provider 时禁止禁用"
cp "${WORK}/config-before-use.yaml" "${BOX_HOME}/mihomo/config.yaml"

set_json="$(run_tool sub set "香港" "香港2" "https://example.com/sub2" --json 2>/dev/null)"
jassert '.ok == true and .data.name == "香港2" and .data.renamed == false' "${set_json}" "sub set 改名并替换 URL"

list_json="$(run_tool sub list --json 2>/dev/null)"
jassert '.data.items[0].name == "香港2" and (.data.items[0].url_masked | test("/\\*\\*\\*$"))' "${list_json}" "sub set 后列表更新"

run_tool sub rm "香港2" --json >/dev/null 2>&1
list_json="$(run_tool sub list --json 2>/dev/null)"
jassert '.data.items | length == 0' "${list_json}" "sub rm 删除订阅"

cat > "${WORK}/normal-provider.yaml" <<'EOF'
proxies:
  - {name: normal-1, type: ss, server: 127.0.0.1, port: 443, cipher: aes-128-gcm, password: p}
EOF

# 普通 provider：auto 模式保持原有节点导入
cat > "${WORK}/bin/curl" <<'EOF'
#!/usr/bin/env bash
out=""; url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -w) shift 2 ;;
    *) url="$1"; shift ;;
  esac
done
case "${url}" in
  https://normal.example/sub) cp "${WORK}/normal-provider.yaml" "${out}" ;;
  *) exit 22 ;;
esac
printf '200'
EOF
chmod +x "${WORK}/bin/curl"
run_tool sub add "https://normal.example/sub" "普通" --json >/dev/null 2>&1
run_tool sub update --json >/dev/null 2>&1
[ -s "${BOX_HOME}/mihomo/proxy_provider/普通.yaml" ] && ok "普通 provider 自动导入" || bad "普通 provider 自动导入"
run_tool sub rm "普通" --json >/dev/null 2>&1
rm -f "${WORK}/bin/curl"

# 聚合订阅：完整配置中的 proxy-providers 自动拆出节点 provider
groups_before="$(yq -o=json '.proxy-groups' "${BOX_HOME}/mihomo/config.yaml" | md5sum | cut -d' ' -f1)"
rules_before="$(yq -o=json '.rules' "${BOX_HOME}/mihomo/config.yaml" | md5sum | cut -d' ' -f1)"
cat > "${WORK}/aggregate.yaml" <<'EOF'
proxies: []
proxy-providers:
  xusx:
    type: http
    url: https://aggregate-provider.example/xusx
  imagic:
    type: http
    url: https://aggregate-provider.example/imagic
  bkm:
    type: http
    url: https://aggregate-provider.example/bkm
proxy-groups:
  - name: 保留策略
    type: select
    include-all: true
rules:
  - MATCH,DIRECT
EOF
cat > "${WORK}/provider.yaml" <<'EOF'
proxies:
  - {name: node-1, type: ss, server: 127.0.0.1, port: 443, cipher: aes-128-gcm, password: p}
EOF
cat > "${WORK}/partial.yaml" <<'EOF'
proxies: []
proxy-providers:
  p1:
    type: http
    url: https://partial-provider.example/p1
  p2:
    type: http
    url: https://partial-provider.example/p2
  p3:
    type: http
    url: https://partial-provider.example/p3
EOF
cat > "${WORK}/bin/curl" <<'EOF'
#!/usr/bin/env bash
out=""
url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -w) shift 2 ;;
    *) url="$1"; shift ;;
  esac
done
case "${url}" in
  https://aggregate.example/sub) cp "${WORK}/aggregate.yaml" "${out}" ;;
  https://aggregate-provider.example/*) cp "${WORK}/provider.yaml" "${out}" ;;
  https://normal.example/sub) cp "${WORK}/normal-provider.yaml" "${out}" ;;
  https://partial.example/sub) cp "${WORK}/partial.yaml" "${out}" ;;
  https://partial-provider.example/p2) exit 22 ;;
  https://partial-provider.example/*) cp "${WORK}/provider.yaml" "${out}" ;;
  *) exit 22 ;;
esac
printf '200'
EOF
chmod +x "${WORK}/bin/curl"
run_tool sub add "https://aggregate.example/sub" "大杂烩" --json >/dev/null 2>&1
run_tool sub update --json >/dev/null 2>&1
jassert '.data.items[0].provider_count == 3 and .data.items[0].provider == "aggregated"' "$(run_tool sub list --json 2>/dev/null)" "聚合订阅显示 3 个 provider"
jassert '.data.provider_count == 3 and .data.provider == "aggregated"' "$(run_tool sub get "大杂烩" --json 2>/dev/null)" "聚合订阅详情显示 3 个 provider"
aggregate_disable="$(run_tool sub disable "大杂烩" --json 2>/dev/null)"
jassert '.ok == true and .data.enabled == false' "${aggregate_disable}" "禁用聚合订阅"
yq -e '."proxy-providers" | (has("xusx") or has("imagic") or has("bkm")) | not' "${BOX_HOME}/mihomo/config.yaml" >/dev/null 2>&1 && ok "禁用聚合订阅移除派生 provider" || bad "禁用聚合订阅移除派生 provider"
aggregate_enable="$(run_tool sub enable "大杂烩" --json 2>/dev/null)"
jassert '.ok == true and .data.enabled == true' "${aggregate_enable}" "启用聚合订阅"
yq -e '."proxy-providers" | (has("xusx") and has("imagic") and has("bkm"))' "${BOX_HOME}/mihomo/config.yaml" >/dev/null 2>&1 && ok "启用聚合订阅恢复派生 provider" || bad "启用聚合订阅恢复派生 provider"
assert_eq "3" "$(wc -l < "${BOX_HOME}/run/subscriptions.providers" | tr -d '[:space:]')" "派生 provider 清单有 3 条"
for provider_name in xusx imagic bkm; do
  [ -s "${BOX_HOME}/mihomo/proxy_provider/${provider_name}.yaml" ] && ok "生成 provider ${provider_name}" || bad "生成 provider ${provider_name}"
done
grep -q '^  "xusx":' "${BOX_HOME}/mihomo/config.yaml" && ok "主配置包含派生 provider xusx" || bad "主配置包含派生 provider xusx"
grep -q '^proxy-groups:' "${BOX_HOME}/mihomo/config.yaml" && ok "主配置保留 proxy-groups" || bad "主配置保留 proxy-groups"
grep -q '^rules:' "${BOX_HOME}/mihomo/config.yaml" && ok "主配置保留 rules" || bad "主配置保留 rules"
groups_after="$(yq -o=json '.proxy-groups' "${BOX_HOME}/mihomo/config.yaml" | md5sum | cut -d' ' -f1)"
rules_after="$(yq -o=json '.rules' "${BOX_HOME}/mihomo/config.yaml" | md5sum | cut -d' ' -f1)"
assert_eq "${groups_before}" "${groups_after}" "聚合更新不改 proxy-groups"
assert_eq "${rules_before}" "${rules_after}" "聚合更新不改 rules"
run_tool sub add "https://partial.example/sub" "部分" --json >/dev/null 2>&1
partial_result="$(run_tool sub update --json 2>/dev/null)"
jassert '.ok == true and .data.partial == true' "${partial_result}" "单个 provider 失败返回部分成功"
[ -s "${BOX_HOME}/mihomo/proxy_provider/p1.yaml" ] && [ -s "${BOX_HOME}/mihomo/proxy_provider/p3.yaml" ] && ok "部分成功保留成功 provider" || bad "部分成功保留成功 provider"
grep -q '^  "p2":' "${BOX_HOME}/mihomo/config.yaml" && bad "失败 provider 不应写入配置" || ok "失败 provider 不写入配置"
run_tool sub rm "部分" --json >/dev/null 2>&1
cat > "${WORK}/provider-mode.json" <<'EOF'
{"subscription_mode":"provider"}
EOF
run_tool settings apply "${WORK}/provider-mode.json" --json >/dev/null 2>&1
provider_mode_result="$(run_tool sub update --json 2>/dev/null)"
jassert '.ok == false and .error.code == "E_COMMAND"' "${provider_mode_result}" "provider 模式拒绝完整配置聚合"
cat > "${WORK}/auto-mode.json" <<'EOF'
{"subscription_mode":"auto"}
EOF
run_tool settings apply "${WORK}/auto-mode.json" --json >/dev/null 2>&1
run_tool sub rm "大杂烩" --json >/dev/null 2>&1
rm -f "${WORK}/bin/curl"

# ---------------------------------------------------------------------------
# 3. 设置 JSON 接口与后端校验
# ---------------------------------------------------------------------------
settings_json="$(run_tool settings get --json 2>/dev/null)"
jassert '.ok == true and (.data | has("network_mode")) and (.data | has("githubtoken") | not)' "${settings_json}" "settings get 不返回 token 明文"
jassert '(.data | has("wifi_ssids_list")) and .data.sensitive.githubtoken.set == false' "${settings_json}" "settings get 返回数组和敏感状态"
jassert '(.data | has("cn_ip_url")) and (.data | has("cn_ipv6_url")) and (.data | has("subscription_url_singbox"))' "${settings_json}" "settings get 返回新增配置键"
jassert '.data.subscription_mode == "auto"' "${settings_json}" "settings get 返回订阅模式默认 auto"
assert_eq "$(jq -r 'keys[]' "${BOX_HOME}/settings.json" | sort)" "$(jq -r '.properties | keys[]' "${BOX_HOME}/settings.schema.json" | sort)" "默认 settings 与 schema 键一致"

cat > "${WORK}/settings.json" <<'EOF'
{"network_mode":"tproxy","wifi_ssids_list":["My Home WiFi","Cafe 'Test'"],"githubtoken":"abcdef1234567890"}
EOF
settings_result="$(run_tool settings apply "${WORK}/settings.json" --json 2>/dev/null)"
jassert '.ok == true and (.data.changed | index("network_mode")) != null' "${settings_result}" "settings apply 返回 changed"

settings_after="$(run_tool settings get --json 2>/dev/null)"
jassert '.data.network_mode == "tproxy" and .data.sensitive.githubtoken.set == true' "${settings_after}" "settings 写入后读取一致"

cat > "${WORK}/check-array.sh" <<'EOF'
. "${BOX_HOME}/settings.base.sh"
settings_load
[ "${#wifi_ssids_list[@]}" -eq 2 ] || exit 1
[ "${wifi_ssids_list[0]}" = "My Home WiFi" ] || exit 1
[ "${wifi_ssids_list[1]}" = "Cafe 'Test'" ] || exit 1
EOF
if BOX_HOME="${BOX_HOME}" bash "${WORK}/check-array.sh"; then
  ok "settings 数组正确转义空格与单引号"
else
  bad "settings 数组正确转义空格与单引号"
fi
assert_eq "githubtoken='abcdef1234567890'" "$(grep '^githubtoken=' "${BOX_HOME}/run/settings.runtime.sh")" "settings 写入 githubtoken"
if bash -n "${BOX_HOME}/run/settings.runtime.sh" 2>/dev/null; then ok "settings.runtime.sh 语法可解析"; else bad "settings.runtime.sh 语法可解析"; fi

cat > "${WORK}/bad-settings.json" <<'EOF'
{"network_mode":"bad-mode"}
EOF
before_hash="$(md5sum "${BOX_HOME}/settings.json" | cut -d' ' -f1)"
before_runtime_hash="$(md5sum "${BOX_HOME}/run/settings.runtime.sh" | cut -d' ' -f1)"
bad_result="$(run_tool settings apply "${WORK}/bad-settings.json" --json 2>/dev/null)"
after_hash="$(md5sum "${BOX_HOME}/settings.json" | cut -d' ' -f1)"
after_runtime_hash="$(md5sum "${BOX_HOME}/run/settings.runtime.sh" | cut -d' ' -f1)"
jassert '.ok == false and .error.code == "E_VALIDATION"' "${bad_result}" "非法枚举返回 E_VALIDATION"
assert_eq "${before_hash}" "${after_hash}" "非法设置不修改 settings.json"
assert_eq "${before_runtime_hash}" "${after_runtime_hash}" "非法设置不修改 settings.runtime.sh"

cat > "${WORK}/missing-settings.json" <<'EOF'
{"tproxy_port":"7777"}
EOF
run_tool settings apply "${WORK}/missing-settings.json" --json >/dev/null 2>&1
settings_missing="$(run_tool settings get --json 2>/dev/null)"
jassert '.data.tproxy_port == "7777" and (.data | has("network_mode")) and (.data | has("wifi_ssids_list"))' "${settings_missing}" "缺失键补默认值"

run_tool settings set use_ghproxy true --json >/dev/null 2>&1
jassert '.use_ghproxy == "true"' "$(cat "${BOX_HOME}/settings.json")" "settings set 更新 JSON"
assert_eq "use_ghproxy='true'" "$(grep '^use_ghproxy=' "${BOX_HOME}/run/settings.runtime.sh")" "settings set 同步 runtime"

cat > "${WORK}/unknown-settings.json" <<'EOF'
{"not_a_real_key":"x"}
EOF
unknown_result="$(run_tool settings apply "${WORK}/unknown-settings.json" --json 2>/dev/null)"
jassert '.ok == false and .error.code == "E_VALIDATION"' "${unknown_result}" "未知设置键被拒绝"

cat > "${WORK}/bad-core.json" <<'EOF'
{"bin_name":"xray"}
EOF
bad_core_result="$(run_tool settings apply "${WORK}/bad-core.json" --json 2>/dev/null)"
jassert '.ok == false and .error.code == "E_VALIDATION"' "${bad_core_result}" "旧核心 bin_name 被拒绝"

cat > "${WORK}/bad-url.json" <<'EOF'
{"cn_ip_url":"not-a-url","subscription_url_singbox":"ftp://example.com/sub"}
EOF
bad_url_result="$(run_tool settings apply "${WORK}/bad-url.json" --json 2>/dev/null)"
jassert '.ok == false and .error.code == "E_VALIDATION"' "${bad_url_result}" "新增 URL 字段校验失败"

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
# 5. mihomo 核心配置 JSON 接口
# ---------------------------------------------------------------------------
core_json="$(run_tool core get --json 2>/dev/null)"
jassert '.ok == true and .data.core == "mihomo" and .data.structured.mode == "rule" and (.data.raw | contains("mixed-port"))' "${core_json}" "core get 返回结构化与 raw"

cat > "${WORK}/core-structured.json" <<'EOF'
{"structured":{"mode":"global","log-level":"info","allow-lan":false,"profile.store-selected":false}}
EOF
core_apply_result="$(run_tool core apply "${WORK}/core-structured.json" --json 2>/dev/null)"
jassert '.ok == true and .data.mode == "structured"' "${core_apply_result}" "core apply structured 成功"
core_after="$(run_tool core get --json 2>/dev/null)"
jassert '.data.structured.mode == "global" and .data.structured["log-level"] == "info" and .data.structured["allow-lan"] == false' "${core_after}" "core apply 结构化字段生效"

jq -Rs '{raw: .}' "${BOX_HOME}/mihomo/config.yaml" > "${WORK}/core-raw.json"
core_raw_result="$(run_tool core apply "${WORK}/core-raw.json" --json 2>/dev/null)"
jassert '.ok == true and .data.mode == "raw"' "${core_raw_result}" "core apply raw 成功"

before_core_hash="$(md5sum "${BOX_HOME}/mihomo/config.yaml" | cut -d' ' -f1)"
printf '%s
' '{"raw":"bad: [unclosed"}' > "${WORK}/core-bad-raw.json"
core_bad_raw="$(run_tool core apply "${WORK}/core-bad-raw.json" --json 2>/dev/null)"
after_core_hash="$(md5sum "${BOX_HOME}/mihomo/config.yaml" | cut -d' ' -f1)"
jassert '.ok == false and .error.code == "E_VALIDATION"' "${core_bad_raw}" "core apply 非法 YAML 被拒绝"
assert_eq "${before_core_hash}" "${after_core_hash}" "core apply 非法 YAML 不修改文件"

# ---------------------------------------------------------------------------
# 6. 应用与网络清单 JSON 接口
# ---------------------------------------------------------------------------
lists_empty="$(run_tool lists get --json 2>/dev/null)"
jassert '.ok == true and (.data.package | length) == 0 and (.data.gid | length) == 0' "${lists_empty}" "lists get 初始返回空清单"

cat > "${WORK}/lists.json" <<'EOF'
{"package":[{"uid":"0","package":"com.example.app"},{"package":"com.example.other"},{"uid":"10","package":"com.example.work"}],"gid":["1000","10123"],"interfaces":{"allow":["ap+","wlan+"],"ignore":["wlan0"]}}
EOF
lists_apply_result="$(run_tool lists apply "${WORK}/lists.json" --json 2>/dev/null)"
jassert '.ok == true and (.data.changed | index("package")) != null and (.data.changed | index("interfaces")) != null' "${lists_apply_result}" "lists apply 返回 changed"
lists_after="$(run_tool lists get --json 2>/dev/null)"
jassert '(.data.package | length) == 3 and .data.package[2].uid == "10" and (.data.gid | length) == 2 and (.data.interfaces.allow | length) == 2' "${lists_after}" "lists 写入后读取一致"
assert_contains '0:com.example.app' "$(cat "${BOX_HOME}/package.list.cfg")" "package.list.cfg 写入 UID:包名"
assert_contains 'allow wlan+' "$(cat "${BOX_HOME}/ap.list.cfg")" "ap.list.cfg 写入 allow"

before_lists_hash="$(md5sum "${BOX_HOME}/package.list.cfg" | cut -d' ' -f1)"
printf '%s
' '{"package":[{"uid":"0","package":"bad package"}],"gid":["not-gid"],"interfaces":{"allow":["bad interface"]}}' > "${WORK}/lists-bad.json"
lists_bad="$(run_tool lists apply "${WORK}/lists-bad.json" --json 2>/dev/null)"
after_lists_hash="$(md5sum "${BOX_HOME}/package.list.cfg" | cut -d' ' -f1)"
jassert '.ok == false and .error.code == "E_VALIDATION"' "${lists_bad}" "lists apply 非法清单被拒绝"
assert_eq "${before_lists_hash}" "${after_lists_hash}" "lists apply 非法清单不修改文件"

# ---------------------------------------------------------------------------
# 6b. 全局规则与策略订阅
# ---------------------------------------------------------------------------
yq -i '."proxy-providers".test = {"type": "http", "url": "https://nodes.example/test.yaml"}' "${BOX_HOME}/mihomo/config.yaml"
before_rp_proxy="$(yq -o=json '."proxy-providers"' "${BOX_HOME}/mihomo/config.yaml" | jq -S .)"
before_rp_port="$(yq -r '."mixed-port"' "${BOX_HOME}/mihomo/config.yaml")"
before_rp_hash="$(md5sum "${BOX_HOME}/mihomo/config.yaml" | cut -d' ' -f1)"

cat > "${WORK}/rule-profile.yaml" <<'EOF'
proxy-groups:
  - name: 节点选择
    type: select
    include-all: true
  - name: 直连
    type: select
    proxies: [DIRECT]
rule-providers:
  ads:
    type: http
    behavior: domain
    format: yaml
    interval: 86400
    url: https://rules.example/ads.yaml
    path: ./rule_provider/ads.yaml
rules:
  - RULE-SET,ads,REJECT
  - MATCH,节点选择
EOF

cat > "${WORK}/rule-profile-b.yaml" <<'EOF'
proxy-groups:
  - name: 节点选择B
    type: select
    include-all: true
rule-providers:
  adsB:
    type: http
    behavior: domain
    format: yaml
    interval: 86400
    url: https://rules.example/adsB.yaml
    path: ./rule_provider/adsB.yaml
rules:
  - DOMAIN,example.org,节点选择B
  - MATCH,节点选择B
EOF

cat > "${WORK}/rule-profile-bad.yaml" <<'EOF'
proxy-groups:
  - name: 节点选择
    type: select
    include-all: true
rule-providers: {}
EOF

cat > "${WORK}/bin/curl" <<'EOF'
#!/usr/bin/env bash
out=""; url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -w) shift 2 ;;
    *) url="$1"; shift ;;
  esac
done
case "${url}" in
  https://rule-profile.example/config.yaml) cp "${WORK}/rule-profile.yaml" "${out}" ;;
  https://rule-profile.example/b.yaml) cp "${WORK}/rule-profile-b.yaml" "${out}" ;;
  https://rule-profile.example/bad.yaml) cp "${WORK}/rule-profile-bad.yaml" "${out}" ;;
  https://rule-profile.example/fallback.yaml) exit 22 ;;
  https://rule-profile.example/fail.yaml) exit 22 ;;
  *) exit 22 ;;
esac
printf '200'
EOF
chmod +x "${WORK}/bin/curl"

rp_set="$(run_tool rule-profile set "https://rule-profile.example/config.yaml" --json 2>/dev/null)"
jassert '.ok == true and .data.supported == true and .data.configured == true and .data.url_masked == "https://rule-profile.example/***"' "${rp_set}" "rule-profile set 返回掩码 URL"
rp_get="$(run_tool rule-profile get --json 2>/dev/null)"
jassert '.data.configured == true and .data.counts.groups >= 1 and .data.counts.rule_providers >= 1 and .data.counts.rules >= 1' "${rp_get}" "rule-profile get 返回当前配置计数"
rp_url="$(run_tool rule-profile url --json 2>/dev/null)"
jassert '.ok == true and .data.url == "https://rule-profile.example/config.yaml"' "${rp_url}" "rule-profile url 只在编辑时返回完整地址"
rp_fetch="$(run_tool rule-profile fetch --json 2>/dev/null)"
jassert '.ok == true and .data.counts.groups == 2 and .data.counts.rule_providers == 3 and .data.counts.rules == 2' "${rp_fetch}" "rule-profile fetch 下载并校验模板"
assert_eq "${before_rp_hash}" "$(md5sum "${BOX_HOME}/mihomo/config.yaml" | cut -d' ' -f1)" "fetch 不修改 config.yaml"
rp_apply="$(run_tool rule-profile apply --json 2>/dev/null)"
jassert '.ok == true and .data.reloaded == false and (.data.changed | length) == 3' "${rp_apply}" "rule-profile apply 应用三段"
rp_after="$(run_tool rule-profile get --json 2>/dev/null)"
jassert '.data.counts.groups == 2 and .data.counts.rule_providers == 3 and .data.counts.rules == 2 and .data.updated_at != ""' "${rp_after}" "rule-profile apply 更新计数和时间"
assert_eq "${before_rp_proxy}" "$(yq -o=json '."proxy-providers"' "${BOX_HOME}/mihomo/config.yaml" | jq -S .)" "apply 保留 proxy-providers"
assert_eq "${before_rp_port}" "$(yq -r '."mixed-port"' "${BOX_HOME}/mihomo/config.yaml")" "apply 保留 mixed-port"
yq -e '."rule-providers" | (has("cn_domain") and has("Fake-IP-Filter"))' "${BOX_HOME}/mihomo/config.yaml" >/dev/null 2>&1 && ok "apply 保留 DNS 引用的规则集" || bad "apply 保留 DNS 引用的规则集"
[ -f "${BOX_HOME}/mihomo/config.yaml.bak" ] && ok "apply 生成 config.yaml.bak" || bad "apply 生成 config.yaml.bak"
grep -q 'DOMAIN,example.com,国外代理' "${BOX_HOME}/mihomo/config.yaml" && bad "apply 应替换旧规则" || ok "apply 替换旧规则"

rp_fetch_reload="$(run_tool rule-profile fetch --json 2>/dev/null)"
jassert '.ok == true' "${rp_fetch_reload}" "rule-profile 重新 fetch 成功"
rp_apply_reload="$(run_tool rule-profile apply --reload --json 2>/dev/null)"
jassert '.ok == true and .data.reloaded == false' "${rp_apply_reload}" "服务未运行时 apply --reload 返回未重载"

cat > "${WORK}/bin/wget" <<'EOF'
#!/usr/bin/env bash
out=""; url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -O) out="$2"; shift 2 ;;
    *) url="$1"; shift ;;
  esac
done
case "${url}" in
  https://rule-profile.example/fallback.yaml) cp "${WORK}/rule-profile.yaml" "${out}"; exit 0 ;;
  *) exit 1 ;;
esac
EOF
chmod +x "${WORK}/bin/wget"
run_tool rule-profile set "https://rule-profile.example/fallback.yaml" --json >/dev/null 2>&1
rp_fallback_fetch="$(run_tool rule-profile fetch --json 2>/dev/null)"
jassert '.ok == true and .data.counts.groups == 2 and .data.counts.rules == 2' "${rp_fallback_fetch}" "curl 失败时回退 wget 下载模板"
run_tool rule-profile set "https://rule-profile.example/fail.yaml" --json >/dev/null 2>&1
rp_download_fail="$(run_tool rule-profile fetch --json 2>/dev/null)"
jassert '.ok == false and .error.code == "E_DOWNLOAD" and (.error.detail | contains("curl=22"))' "${rp_download_fail}" "下载失败返回 E_DOWNLOAD 而非空 JSON"
rm -f "${WORK}/bin/wget"

rp_bad_before="$(md5sum "${BOX_HOME}/mihomo/config.yaml" | cut -d' ' -f1)"
run_tool rule-profile set "https://rule-profile.example/bad.yaml" --json >/dev/null 2>&1
rp_bad_fetch="$(run_tool rule-profile fetch --json 2>/dev/null)"
jassert '.ok == false and .error.code == "E_VALIDATION"' "${rp_bad_fetch}" "rule-profile 拒绝缺少 rules 的模板"
assert_eq "${rp_bad_before}" "$(md5sum "${BOX_HOME}/mihomo/config.yaml" | cut -d' ' -f1)" "非法模板不修改 config.yaml"

rp_clear="$(run_tool rule-profile clear --json 2>/dev/null)"
jassert '.ok == true and .data.cleared == true' "${rp_clear}" "rule-profile clear 清除 URL"
rp_cleared="$(run_tool rule-profile get --json 2>/dev/null)"
jassert '.data.configured == false and .data.url_masked == ""' "${rp_cleared}" "清除后规则订阅为未配置"
# 6c. 多规则订阅：添加、暂存、使用、删除保护
rp_add_a="$(run_tool rule-profile add "规则A" "https://rule-profile.example/config.yaml" --json 2>/dev/null)"
jassert '.ok == true and .data.name == "规则A" and .data.active == false and .data.pending == false' "${rp_add_a}" "多规则订阅添加规则A"
rp_add_b="$(run_tool rule-profile add "规则B" "https://rule-profile.example/config.yaml" --json 2>/dev/null)"
jassert '.ok == true and .data.name == "规则B"' "${rp_add_b}" "多规则订阅添加规则B"
rp_id_b="$(printf '%s' "${rp_add_b}" | jq -r '.data.id')"
rp_list="$(run_tool rule-profile list --json 2>/dev/null)"
jassert '.data.supported == true and (.data.items | length) == 2 and (([.data.items[].name] | sort) == ["规则A","规则B"])' "${rp_list}" "多规则订阅列表"
jassert '.data.items | map(select(.name == "规则A"))[0].url_masked == "https://rule-profile.example/***"' "${rp_list}" "多规则列表掩码 URL"

rp_multi_before="$(md5sum "${BOX_HOME}/mihomo/config.yaml" | cut -d' ' -f1)"
rp_upd_b="$(run_tool rule-profile update "${rp_id_b}" --json 2>/dev/null)"
jassert '.ok == true and .data.pending == true and .data.counts.groups == 2 and .data.counts.rules == 2' "${rp_upd_b}" "更新非当前规则只暂存"
assert_eq "${rp_multi_before}" "$(md5sum "${BOX_HOME}/mihomo/config.yaml" | cut -d' ' -f1)" "更新非当前规则不修改 config.yaml"

rp_use_b="$(run_tool rule-profile use "${rp_id_b}" --json 2>/dev/null)"
jassert '.ok == true and .data.reloaded == false and .data.counts.groups == 2' "${rp_use_b}" "使用指定规则订阅并应用"
rp_list_used="$(run_tool rule-profile list --json 2>/dev/null)"
jassert '.data.items | ((map(select(.name == "规则B"))[0].active == true) and (map(select(.name == "规则A"))[0].active == false))' "${rp_list_used}" "多规则当前使用标记"
rp_remove_active="$(run_tool rule-profile remove "${rp_id_b}" --json 2>/dev/null)"
jassert '.ok == false and .error.code == "E_VALIDATION"' "${rp_remove_active}" "当前使用规则禁止删除"
rp_deact_hash="$(md5sum "${BOX_HOME}/mihomo/config.yaml" | cut -d' ' -f1)"
rp_deactivate="$(run_tool rule-profile deactivate --json 2>/dev/null)"
jassert '.ok == true and .data.active == false' "${rp_deactivate}" "停用当前规则订阅"
assert_eq "${rp_deact_hash}" "$(md5sum "${BOX_HOME}/mihomo/config.yaml" | cut -d' ' -f1)" "停用不修改 config.yaml"
rp_remove_b="$(run_tool rule-profile remove "${rp_id_b}" --json 2>/dev/null)"
jassert '.ok == true and .data.removed == true' "${rp_remove_b}" "停用后删除规则B"
rp_id_a="$(printf '%s' "${rp_add_a}" | jq -r '.data.id')"
run_tool rule-profile remove "${rp_id_a}" --json >/dev/null 2>&1

rp_edit_add="$(run_tool rule-profile add "规则C" "https://rule-profile.example/config.yaml" --json 2>/dev/null)"
rp_id_c="$(printf '%s' "${rp_edit_add}" | jq -r '.data.id')"
rp_edit="$(run_tool rule-profile edit "${rp_id_c}" "规则C改" "https://rule-profile.example/config.yaml" --json 2>/dev/null)"
jassert '.ok == true and .data.name == "规则C改"' "${rp_edit}" "编辑规则订阅名称"
rp_url_c="$(run_tool rule-profile url "${rp_id_c}" --json 2>/dev/null)"
jassert '.ok == true and .data.url == "https://rule-profile.example/config.yaml"' "${rp_url_c}" "按 ID 读取规则订阅完整地址"
run_tool rule-profile remove "${rp_id_c}" --json >/dev/null 2>&1

rp_add_d="$(run_tool rule-profile add "规则D" "https://rule-profile.example/config.yaml" --json 2>/dev/null)"
rp_id_d="$(printf '%s' "${rp_add_d}" | jq -r '.data.id')"
rp_use_d="$(run_tool rule-profile use "${rp_id_d}" --json 2>/dev/null)"
jassert '.ok == true and .data.counts.groups == 2 and .data.counts.rules == 2' "${rp_use_d}" "无快照时使用自动下载并应用"
run_tool rule-profile deactivate --json >/dev/null 2>&1
run_tool rule-profile remove "${rp_id_d}" --json >/dev/null 2>&1

# 6d. 两条规则订阅先后使用，只保留当前一条
run_tool rule-profile clear --json >/dev/null 2>&1
rp_no_merge_a="$(run_tool rule-profile add "规则E" "https://rule-profile.example/config.yaml" --json 2>/dev/null)"
rp_id_e="$(printf '%s' "${rp_no_merge_a}" | jq -r '.data.id')"
run_tool rule-profile use "${rp_id_e}" --json >/dev/null 2>&1
rp_no_merge_b="$(run_tool rule-profile add "规则F" "https://rule-profile.example/b.yaml" --json 2>/dev/null)"
rp_id_f="$(printf '%s' "${rp_no_merge_b}" | jq -r '.data.id')"
run_tool rule-profile use "${rp_id_f}" --json >/dev/null 2>&1
grep -q 'DOMAIN,example.org,节点选择B' "${BOX_HOME}/mihomo/config.yaml" && ok "使用规则F写入F的三段" || bad "使用规则F写入F的三段"
grep -q 'DOMAIN,example.com,节点选择' "${BOX_HOME}/mihomo/config.yaml" && bad "使用规则F不保留规则E" || ok "使用规则F不保留规则E"
run_tool rule-profile deactivate --json >/dev/null 2>&1
run_tool rule-profile remove "${rp_id_f}" --json >/dev/null 2>&1
run_tool rule-profile remove "${rp_id_e}" --json >/dev/null 2>&1

run_tool rule-profile clear --json >/dev/null 2>&1
mkdir -p "${BOX_HOME}/mihomo" "${BOX_HOME}/run/state"
printf 'https://legacy.example/config.yaml
' > "${BOX_HOME}/mihomo/rule-profile.url"
printf '2026-11-24T03:00:00+0800
' > "${BOX_HOME}/run/state/rule-profile.updated_at"
rp_migrated="$(run_tool rule-profile list --json 2>/dev/null)"
jassert '.data.items | ((length == 1) and (.[0].id == "default") and (.[0].name == "默认规则") and (.[0].active == true))' "${rp_migrated}" "旧单条规则订阅迁移"
rp_migrated_get="$(run_tool rule-profile get --json 2>/dev/null)"
jassert '.data.configured == true and .data.id == "default" and .data.updated_at != ""' "${rp_migrated_get}" "迁移后旧 get 仍可用"
run_tool rule-profile clear --json >/dev/null 2>&1

run_tool rule-profile set "https://secret.example/config.yaml?token=abc123" --json >/dev/null 2>&1
if grep -Fq 'token=abc123' "${BOX_HOME}/run/tool.log" 2>/dev/null; then
  bad "tool.log 不泄露规则订阅 token"
else
  ok "tool.log 不泄露规则订阅 token"
fi
run_tool rule-profile clear --json >/dev/null 2>&1
rm -f "${WORK}/bin/curl"

# ---------------------------------------------------------------------------
# 7. 日志 JSON 接口
# ---------------------------------------------------------------------------
mkdir -p "${BOX_HOME}/run"
printf 'line1\nline2\nline3\n' > "${BOX_HOME}/run/runs.log"
logs_json="$(run_tool logs 2 --json 2>/dev/null)"
jassert '.ok == true and (.data.lines | length) == 2 and .data.lines[1] == "line3"' "${logs_json}" "logs 返回行数组"

# ---------------------------------------------------------------------------
# 8. 后台任务生命周期
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
# 9. 服务包装、日志清空与重置
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

# 切换到 sing-box，status 应读取 sing-box 配置
cat > "${WORK}/switch-singbox.json" <<'EOF'
{"bin_name":"sing-box","subscription_url_singbox":"https://example.com/singbox"}
EOF
run_tool settings apply "${WORK}/switch-singbox.json" --json >/dev/null 2>&1
sing_status="$(run_tool status --json 2>/dev/null)"
jassert '.ok == true and .data.core == "sing-box" and .data.config == "config.json" and .data.config_exists == true' "${sing_status}" "status 正确识别 sing-box 配置"
settings_sing="$(run_tool settings get --json 2>/dev/null)"
jassert '.data.subscription_url_singbox == "https://example.com/singbox"' "${settings_sing}" "sing-box 订阅地址写入 settings"
core_sing="$(run_tool core get --json 2>/dev/null)"
jassert '.ok == false and .error.code == "E_VALIDATION"' "${core_sing}" "sing-box 不支持 mihomo 核心配置接口"
config_sing="$(run_tool config get --json 2>/dev/null)"
jassert '.ok == false and .error.code == "E_VALIDATION"' "${config_sing}" "sing-box 不支持 mihomo 规则配置接口"
rp_sing="$(run_tool rule-profile get --json 2>/dev/null)"
jassert '.ok == true and .data.supported == false' "${rp_sing}" "sing-box 规则订阅返回不支持状态"
rp_set_sing="$(run_tool rule-profile set "https://rules.example/config.yaml" --json 2>/dev/null)"
jassert '.ok == false and .error.code == "E_VALIDATION"' "${rp_set_sing}" "sing-box 拒绝配置规则订阅"

# ---------------------------------------------------------------------------
# 10. runtime 恢复与损坏设置
# ---------------------------------------------------------------------------
rm -f "${BOX_HOME}/run/settings.runtime.sh"
status_regen="$(run_tool status --json 2>/dev/null)"
jassert '.ok == true and .data.running == false' "${status_regen}" "runtime 缺失时按 settings.json 恢复"

cp "${BOX_HOME}/settings.json" "${WORK}/settings-valid-backup.json"
printf '{broken-json' > "${BOX_HOME}/settings.json"
corrupt_result="$(run_tool settings get --json 2>/dev/null)"
jassert '.ok == false and .error.code == "E_VALIDATION"' "${corrupt_result}" "损坏 settings.json 返回 E_VALIDATION"
status_corrupt="$(run_tool status --json 2>/dev/null)"
jassert '.ok == true and .data.running == false' "${status_corrupt}" "损坏 settings.json 时保留旧 runtime"
mv "${WORK}/settings-valid-backup.json" "${BOX_HOME}/settings.json"

# ---------------------------------------------------------------------------
# 10. CLI 文本兼容（不带 --json）
# ---------------------------------------------------------------------------
run_tool sub add "https://example.com/cli" "CLI订阅" >/dev/null 2>&1
cli_list="$(run_tool sub list 2>/dev/null)"
assert_contains "CLI订阅" "${cli_list}" "sub list 文本输出保留"
assert_contains "https://example.com/***" "${cli_list}" "sub list 文本仍掩码 URL"

printf '\n通过 %d 项，失败 %d 项\n' "${PASS}" "${FAIL}"
[ "${FAIL}" -eq 0 ]
