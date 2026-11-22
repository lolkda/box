#!/system/bin/sh

scripts_dir="${0%/*}"

user_agent="box_for_root"
source "${BOX_HOME:-/data/adb/box}/settings.ini"

for _arg in "$@"; do
  [ "${_arg}" = "--json" ] && export BOX_JSON_MODE=1
done

# 使用 settings.ini 中提供的 log()
TOOL_LOG="${box_run}/tool.log"
busybox mkdir -p "$(dirname "$TOOL_LOG")"
box_log="$TOOL_LOG"

# 设置 GitHub API 访问配置
setup_github_api() {
  rev1="busybox wget --no-check-certificate -qO-"
  if which curl >/dev/null; then
    rev1="curl --insecure -sL"
  fi
  if [ -n "$githubtoken" ]; then
    if which curl >/dev/null; then
      rev1="curl --insecure -sL -H \"Authorization: token ${githubtoken}\""
    else
      rev1="busybox wget --no-check-certificate -qO- --header=\"Authorization: token ${githubtoken}\""
    fi
    log Debug "GitHub Token 已配置，将使用认证访问 GitHub API"
  else
    log Debug "未配置 GitHub Token，将使用匿名访问 GitHub API"
  fi
}

mask_url() {
  local u="$1"
  echo "$u" | sed -E 's#^([a-zA-Z][a-zA-Z0-9+.-]*://)?([^/]+).*$#\1\2/***#'
}

# 启动提示
divider() {
  [ -n "$box_log" ] && log Info "----------------------------------------" >/dev/null
}
trap divider EXIT
log Info "执行命令: $0 $@"

# 更新文件
upfile() {
  local file="$1"
  local update_url="$2"
  local custom_ua="$3" # 接收自定义 User-Agent
  local current_ua

  # 如果提供了自定义 UA, 则使用它; 否则使用全局默认值
  if [ -n "${custom_ua}" ]; then
    current_ua="${custom_ua}"
  else
    current_ua="${user_agent}"
  fi

  local file_bak="${file}.bak"
  [ -f "${file}" ] && mv "${file}" "${file_bak}"

  # 使用 ghproxy
  if [ "${use_ghproxy}" = "true" ] && [[ "${update_url}" == @(https://github.com/*|https://raw.githubusercontent.com/*|https://gist.github.com/*|https://gist.githubusercontent.com/*) ]]; then
    update_url="${url_ghproxy}/${update_url}"
  fi
  
  log_url="${update_url}"
  if [ "${LOG_MASK_URL}" = "mask" ]; then
    log_url="$(mask_url "${update_url}")"
  fi
  log Info "开始下载: ${log_url}"
  log Debug "保存到: ${file}"
  log Debug "使用 User-Agent: ${current_ua}"

  if which curl >/dev/null; then
    http_code=$(curl -L -s --insecure --http1.1 --compressed --user-agent "${current_ua}" -o "${file}" -w "%{http_code}" "${update_url}")
    curl_exit_code=$?

    if [ ${curl_exit_code} -ne 0 ]; then
      log Error "使用 curl 下载失败 (退出码: ${curl_exit_code})"
      [ -f "${file_bak}" ] && mv "${file_bak}" "${file}"
      return 1
    fi

    if [ "${http_code}" -ne 200 ]; then
      log Error "下载失败: 服务器返回 HTTP 状态码 ${http_code}"
      [ -f "${file_bak}" ] && mv "${file_bak}" "${file}"
      return 1
    fi
  else
    if ! busybox wget --no-check-certificate -q -U "${current_ua}" -O "${file}" "${update_url}"; then
      log Error "使用 wget 下载失败"
      [ -f "${file_bak}" ] && mv "${file_bak}" "${file}"
      return 1
    fi
  fi

  if [ ! -s "${file}" ]; then
    log Error "下载失败: 文件为空"
    [ -f "${file_bak}" ] && mv "${file_bak}" "${file}"
    return 1
  fi
  
  log Info "下载成功"
  rm -f "${file_bak}" 2>/dev/null
  return 0
}

# CN IPv4/IPv6 列表更新
is_box_running() {
  [ -f "${box_pid}" ] && kill -0 "$(<"${box_pid}" 2>/dev/null)" 2>/dev/null
}

reload_cnip_set() {
  local set_name="$1"
  local family="$2"
  local source_file="$3"
  local entries=0

  if [ ! -s "${source_file}" ]; then
    log Error "${set_name} 重写入失败: 文件不存在或为空"
    return 1
  fi

  if ! command -v ipset >/dev/null 2>&1; then
    log Warning "ipset 不可用，跳过 ${set_name} 重写入"
    return 1
  fi

  entries="$(busybox awk '!/^[[:space:]]*#/ && NF > 0 {count++} END {print count + 0}' "${source_file}" 2>/dev/null)"
  log Info "正在重写入 ${set_name}，条目=${entries}"

  if {
    echo "create ${set_name} hash:net family ${family} hashsize 8192 maxelem 65536 -exist"
    echo "flush ${set_name}"
    busybox awk -v set="${set_name}" '!/^[[:space:]]*#/ && NF > 0 {printf "add %s %s\n", set, $0}' "${source_file}"
  } | ipset restore -exist 2>/dev/null; then
    log Info "${set_name} 重写入完成"
    return 0
  fi

  log Error "${set_name} 重写入失败"
  return 1
}

reload_cnip_sets_after_update() {
  local updated_v4="$1"
  local updated_v6="$2"
  local ok="true"

  [ "${bypass_cn_ip}" = "true" ] || return 0
  is_box_running || return 0

  if [ "${updated_v4}" = "true" ] || [ "${updated_v6}" = "true" ]; then
    log Info "服务运行中且已启用 CNIP，正在重写入 ipset"
  fi

  if [ "${updated_v4}" = "true" ]; then
    reload_cnip_set cnip inet "${cn_ip_file}" || ok="false"
  fi

  if [ "${updated_v6}" = "true" ]; then
    reload_cnip_set cnip6 inet6 "${cn_ipv6_file}" || ok="false"
  fi

  [ "${ok}" = "true" ]
}

upcnip() {
  local attempted=0
  local success=0
  local updated_v4="false"
  local updated_v6="false"

  # IPv4
  if [ "${bypass_cn_ip}" = "true" ] && [ "${bypass_cn_ip_v4}" = "true" ]; then
    if [ -z "${cn_ip_url}" ] || [ -z "${cn_ip_file}" ]; then
      log Warning "cn_ip_url 或 cn_ip_file 未配置，跳过 IPv4"
    else
      attempted=$((attempted + 1))
      busybox mkdir -p "$(dirname "${cn_ip_file}")" >/dev/null 2>&1 || true
      log Info "下载 CN IPv4 列表 → ${cn_ip_file}"
      if upfile "${cn_ip_file}" "${cn_ip_url}"; then
        log Info "CN IPv4 列表更新完成"
        success=$((success + 1))
        updated_v4="true"
      else
        log Error "CN IPv4 列表更新失败"
      fi
    fi
  else
    log Debug "未启用 IPv4 CN 分流（bypass_cn_ip/bypass_cn_ip_v4=false），跳过下载"
  fi

  # IPv6
  if [ "${bypass_cn_ip}" = "true" ] && [ "${ipv6}" = "true" ] && [ "${bypass_cn_ip_v6}" = "true" ]; then
    if [ -z "${cn_ipv6_url}" ] || [ -z "${cn_ipv6_file}" ]; then
      log Warning "cn_ipv6_url 或 cn_ipv6_file 未配置，跳过 IPv6"
    else
      attempted=$((attempted + 1))
      busybox mkdir -p "$(dirname "${cn_ipv6_file}")" >/dev/null 2>&1 || true
      log Info "下载 CN IPv6 列表 → ${cn_ipv6_file}"
      if upfile "${cn_ipv6_file}" "${cn_ipv6_url}"; then
        log Info "CN IPv6 列表更新完成"
        success=$((success + 1))
        updated_v6="true"
      else
        log Error "CN IPv6 列表更新失败"
      fi
    fi
  else
    log Debug "未启用 IPv6 CN 分流或未开启 IPv6，跳过下载"
  fi

  if [ "${attempted}" -eq 0 ]; then
    log Info "未启用 CNIP 更新项，已跳过"
    return 0
  fi

  if [ "${success}" -ne "${attempted}" ]; then
    log Error "CNIP 列表更新异常（成功 ${success}/${attempted}）"
    return 1
  fi

  if ! reload_cnip_sets_after_update "${updated_v4}" "${updated_v6}"; then
    log Error "CNIP ipset 重写入失败"
    return 1
  fi

  log Info "CNIP 列表更新完成（成功 ${success}/${attempted}）"
  return 0
}

# 重启核心进程
restart_box() {
  local core_to_restart=${1:-$bin_name}
  if [ -z "$core_to_restart" ]; then
    log Error "restart_box: 未指定需要重启的核心"
    return 1
  fi
  
  "${scripts_dir}/box.service" restart "$core_to_restart"
  
  local pid
  pid=$(busybox pidof "$core_to_restart")

  if [ -n "$pid" ]; then
    log Info "$core_to_restart 重启完成 [$(date +"%F %R")]"
  else
    log Error "重启 $core_to_restart 失败."
    "${scripts_dir}/box.iptables" disable >/dev/null 2>&1
  fi
}

# 检查配置
check() {
  case "${bin_name}" in
    sing-box)
      if ${bin_path} check -c "${sing_config}" > "${box_run}/${bin_name}_report.log" 2>&1; then
        log Info "${sing_config} 检查通过"
      else
        log Debug "${sing_config}"
        log Error "$(<"${box_run}/${bin_name}_report.log")" >&2
      fi
      ;;
    mihomo)
      if ${bin_path} -t -d "${box_dir}/mihomo" -f "${mihomo_config}" > "${box_run}/${bin_name}_report.log" 2>&1; then
        log Info "${mihomo_config} 检查通过"
      else
        log Debug "${mihomo_config}"
        log Error "$(<"${box_run}/${bin_name}_report.log")" >&2
      fi
      ;;
    xray)
      export XRAY_LOCATION_ASSET="${box_dir}/xray"
      if ${bin_path} -test -confdir "${box_dir}/${bin_name}" > "${box_run}/${bin_name}_report.log" 2>&1; then
        log Info "配置检查通过"
      else
        log Debug "$(ls ${box_dir}/${bin_name})"
        log Error "$(<"${box_run}/${bin_name}_report.log")" >&2
      fi
      ;;
    v2fly)
      export V2RAY_LOCATION_ASSET="${box_dir}/v2fly"
      if ${bin_path} test -d "${box_dir}/${bin_name}" > "${box_run}/${bin_name}_report.log" 2>&1; then
        log Info "配置检查通过"
      else
        log Debug "$(ls ${box_dir}/${bin_name})"
        log Error "$(<"${box_run}/${bin_name}_report.log")" >&2
      fi
      ;;
    hysteria)
      true
      ;;
    *)
      log Error "<${bin_name}> 未知的二进制文件."
      exit 1
      ;;
  esac
}

# 重载基础配置
reload() {
  ip_port=$(if [ "${bin_name}" = "mihomo" ]; then busybox awk '/external-controller:/ {print $2}' "${mihomo_config}" | sed "s/'//g"; else busybox awk -F'[:,]' '/"external_controller"/ {print $2":"$3}' "${sing_config}" | sed 's/^[ \t]*//;s/"//g'; fi;)
  secret=$(if [ "${bin_name}" = "mihomo" ]; then busybox awk '/^secret:/ {print $2}' "${mihomo_config}" | sed 's/"//g'; else busybox awk -F'"' '/"secret"/ {print $4}' "${sing_config}" | head -n 1; fi;)

  curl_command="curl"
  if ! command -v curl >/dev/null; then
    if [ ! -e "${bin_dir}/curl" ]; then
      log Debug "$bin_dir/curl 文件未找到, 无法重载配置"
      log Debug "开始从 GitHub 下载"
      upcurl || exit 1
    fi
    curl_command="${bin_dir}/curl"
  fi

  check

  case "${bin_name}" in
    "mihomo")
      endpoint="http://${ip_port}/configs?force=true"

      if ${curl_command} -X PUT -H "Authorization: Bearer ${secret}" "${endpoint}" -d '{"path": "", "payload": ""}' 2>&1; then
        log Info "${bin_name} 配置重载成功"
        return 0
      else
        log Error "${bin_name} 配置重载失败 !"
        return 1
      fi
      ;;
    "sing-box")
      endpoint="http://${ip_port}/configs?force=true"
      if ${curl_command} -X PUT -H "Authorization: Bearer ${secret}" "${endpoint}" -d '{"path": "", "payload": ""}' 2>&1; then
        log Info "${bin_name} 配置重载成功."
        return 0
      else
        log Error "${bin_name} 配置重载失败 !"
        return 1
      fi
      ;;
    "xray"|"v2fly"|"hysteria")
      if [ -f "${box_pid}" ]; then
        if kill -0 "$(<"${box_pid}" 2>/dev/null)"; then
          restart_box
        fi
      fi
      ;;
    *)
      log Warning "${bin_name} 不支持使用 API 重载配置."
      return 1
      ;;
  esac
}

# 获取最新的 curl
upcurl() {
  setup_github_api
  
  local arch
  case $(uname -m) in
    "aarch64") arch="aarch64" ;;
    "armv7l"|"armv8l") arch="armv7" ;;
    "i686") arch="i686" ;;
    "x86_64") arch="amd64" ;;
    *) log Warning "不支持的架构: $(uname -m)" >&2; return 1 ;;
  esac

  mkdir -p "${bin_dir}/backup"
  [ -f "${bin_dir}/curl" ] && cp "${bin_dir}/curl" "${bin_dir}/backup/curl.bak" >/dev/null 2>&1

  local latest_version=$($rev1 "https://api.github.com/repos/stunnel/static-curl/releases" | grep "tag_name" | busybox grep -oE "[0-9.]*" | head -1)
  local download_link="https://github.com/stunnel/static-curl/releases/download/${latest_version}/curl-linux-${arch}-glibc-${latest_version}.tar.xz"
  local temp_archive="${box_dir}/curl.tar.xz"
  local temp_extract_dir="${box_dir}/curl_temp"

  log Debug "下载 ${download_link}"
  if ! upfile "${temp_archive}" "${download_link}"; then
    log Error "下载 curl 失败"
    return 1
  fi
  
  rm -rf "${temp_extract_dir}"
  mkdir -p "${temp_extract_dir}"

  if ! busybox tar -xJf "${temp_archive}" -C "${temp_extract_dir}" >&2; then
    log Error "解压 ${temp_archive} 失败" >&2
    cp "${bin_dir}/backup/curl.bak" "${bin_dir}/curl" >/dev/null 2>&1 && log Info "已恢复 curl"
    rm -f "${temp_archive}"
    rm -rf "${temp_extract_dir}"
    return 1
  fi

  local curl_binary=$(find "${temp_extract_dir}" -type f -name "curl")
  if [ -n "${curl_binary}" ]; then
    mv "${curl_binary}" "${bin_dir}/curl"
    log Info "curl 已成功更新到 ${bin_dir}/curl"
  else
    log Error "在解压的存档中未找到 curl 二进制文件"
    rm -f "${temp_archive}"
    rm -rf "${temp_extract_dir}"
    return 1
  fi
  
  chown "${box_user_group}" "${box_dir}/bin/curl"
  chmod 0755 "${bin_dir}/curl"

  rm -f "${temp_archive}"
  rm -rf "${temp_extract_dir}"
}

# 获取最新的 yq
upyq() {
  local arch platform
  case $(uname -m) in
    "aarch64") arch="arm64"; platform="android" ;;
    "armv7l"|"armv8l") arch="arm"; platform="android" ;;
    "i686") arch="386"; platform="android" ;;
    "x86_64") arch="amd64"; platform="android" ;;
    *) log Warning "不支持的架构: $(uname -m)" >&2; return 1 ;;
  esac

  local download_link="https://github.com/taamarin/yq/releases/download/prerelease/yq_${platform}_${arch}"

  log Debug "下载 ${download_link}"
  upfile "${box_dir}/bin/yq" "${download_link}"

  chown "${box_user_group}" "${box_dir}/bin/yq"
  chmod 0755 "${box_dir}/bin/yq"
}

# 检查并更新 geoip 和 geosite
upgeox() {
  geodata_mode=$(busybox awk '!/^ *#/ && /geodata-mode:*./{print $2}' "${mihomo_config}")
  [ -z "${geodata_mode}" ] && geodata_mode=false
  case "${bin_name}" in
    mihomo)
      geoip_file="${box_dir}/mihomo/Country.mmdb"
      geoip_url="https://github.com/MetaCubeX/meta-rules-dat/raw/release/country-lite.mmdb"
      geosite_file="${box_dir}/mihomo/GeoSite.dat"
      geosite_url="https://github.com/MetaCubeX/meta-rules-dat/raw/release/geosite.dat"
      ;;
    sing-box)
      geoip_file="${box_dir}/sing-box/geoip.db"
      geoip_url="https://github.com/MetaCubeX/meta-rules-dat/raw/release/geoip-lite.db"
      geosite_file="${box_dir}/sing-box/geosite.db"
      geosite_url="https://github.com/MetaCubeX/meta-rules-dat/raw/release/geosite.db"
      ;;
    *)
      geoip_file="${box_dir}/${bin_name}/geoip.dat"
      geoip_url="https://github.com/MetaCubeX/meta-rules-dat/raw/release/geoip-lite.dat"
      geosite_file="${box_dir}/${bin_name}/geosite.dat"
      geosite_url="https://github.com/MetaCubeX/meta-rules-dat/raw/release/geosite.dat"
      ;;
  esac
  if [ "${update_geo}" = "true" ] && { log Info "每日更新 GeoX" && log Debug "正在下载 ${geoip_url}"; } && upfile "${geoip_file}" "${geoip_url}" && { log Debug "正在下载 ${geosite_url}" && upfile "${geosite_file}" "${geosite_url}"; }; then

    find "${box_dir}/${bin_name}" -maxdepth 1 -type f -name "*.db.bak" -delete
    find "${box_dir}/${bin_name}" -maxdepth 1 -type f -name "*.dat.bak" -delete
    find "${box_dir}/${bin_name}" -maxdepth 1 -type f -name "*.mmdb.bak" -delete

    log Debug "更新 GeoX 于 $(date "+%F %R")"
    return 0
  else
   return 1
  fi
}

upgeox_all() {
  local original_bin_name=$bin_name
  for core in mihomo sing-box xray v2fly; do
      bin_name=$core
      upgeox
  done
  bin_name=$original_bin_name
}

# 订阅清单：每行 名字|URL
subscriptions_file="${subscriptions_file:-${box_dir}/mihomo/subscriptions.list}"

# 把订阅名字转换成安全的 provider 文件名
sub_provider_file_name() {
  local safe
  safe="$(printf '%s' "$1" | busybox sed 's#[/\\|[:space:]]#_#g; s/^\.*//')"
  [ -n "${safe}" ] || safe="sub"
  printf '%s' "${safe}"
}

# 订阅名字是否已存在
subscription_exists() {
  local wanted="$1"
  [ -f "${subscriptions_file}" ] || return 1
  busybox awk -F'|' -v wanted="${wanted}" '$1 == wanted { found = 1 } END { exit(found ? 0 : 1) }' "${subscriptions_file}"
}

# 更新 mihomo 配置的 proxy-providers
update_mihomo_providers() {
  if [ ! -f "${mihomo_config}" ]; then
    log Error "配置文件不存在: ${mihomo_config}"
    return 1
  fi

  log Debug "开始更新 proxy-providers 配置项..."
  local temp_providers="${mihomo_config}.providers.tmp"
  local provider_count=0

  echo "proxy-providers:" > "${temp_providers}"

  if [ -s "${subscriptions_file}" ]; then
    while IFS='|' read -r provider_name provider_url; do
      case "${provider_name}" in
        ''|\#*) continue ;;
      esac
      [ -n "${provider_url}" ] || continue

      local safe_name
      safe_name="$(sub_provider_file_name "${provider_name}")"
      local relative_path="./$(basename "${mihomo_provide_path}")/${safe_name}.yaml"
      local escaped_url

      escaped_url="$(printf '%s' "${provider_url}" | busybox sed 's/\\/\\\\/g; s/"/\\"/g')"

      log Debug "添加 provider: ${provider_name} -> ${relative_path} (http)"

      cat >> "${temp_providers}" <<EOF
  "${provider_name}":
    type: http
    url: "${escaped_url}"
    path: ${relative_path}
    interval: 86400
    health-check:
      enable: true
      url: https://cp.cloudflare.com
      interval: 300
      timeout: 1000
      tolerance: 100
EOF
      provider_count=$((provider_count + 1))
    done < "${subscriptions_file}"
  fi

  if [ "${provider_count}" -eq 0 ]; then
    echo "proxy-providers: {}" > "${temp_providers}"
  fi

  local temp_output="${mihomo_config}.output.tmp"

  busybox awk -v new_providers="${temp_providers}" '
    BEGIN {
      in_providers = 0
      providers_done = 0
    }
    /^proxy-providers:/ {
      in_providers = 1
      if (providers_done == 0) {
        while ((getline line < new_providers) > 0) {
          print line
        }
        close(new_providers)
        providers_done = 1
      }
      next
    }
    in_providers == 1 && /^[a-zA-Z-]+:/ {
      in_providers = 0
      print
      next
    }
    in_providers == 1 {
      next
    }
    {
      print
    }
    END {
      if (providers_done == 0) {
        while ((getline line < new_providers) > 0) {
          print line
        }
        close(new_providers)
      }
    }
  ' "${mihomo_config}" > "${temp_output}"

  mv "${temp_output}" "${mihomo_config}"
  rm -f "${temp_providers}"

  log Debug "proxy-providers 配置构建完成（${provider_count} 个）"
  return 0
}

# 按 ruleset_prefix 重写 rule-providers 的 url，重复执行不会叠加前缀
update_mihomo_rulesets() {
  [ -f "${mihomo_config}" ] || return 1

  local state_file="${box_run_state}/ruleset_prefix"
  local old_prefix=""
  [ -f "${state_file}" ] && old_prefix="$(cat "${state_file}" 2>/dev/null)"

  if [ -z "${ruleset_prefix}" ] && [ -z "${old_prefix}" ]; then
    return 0
  fi

  local temp_output="${mihomo_config}.rulesets.tmp"

  busybox awk -v old_prefix="${old_prefix}" -v new_prefix="${ruleset_prefix}" '
    /^rule-providers:/ { in_rulesets = 1; print; next }
    in_rulesets == 1 && /^[a-zA-Z-]+:/ { in_rulesets = 0 }
    in_rulesets == 1 && /^[[:space:]]+url:[[:space:]]*/ {
      value = $0
      sub(/^[[:space:]]+url:[[:space:]]*/, "", value)
      gsub(/^"|"$/, "", value)
      if (old_prefix != "" && index(value, old_prefix "/") == 1) {
        value = substr(value, length(old_prefix) + 2)
      }
      if (new_prefix != "") {
        value = new_prefix "/" value
      }
      match($0, /^[[:space:]]+/)
      indent = substr($0, 1, RLENGTH)
      print indent "url: \"" value "\""
      next
    }
    { print }
  ' "${mihomo_config}" > "${temp_output}" && mv "${temp_output}" "${mihomo_config}"

  printf '%s' "${ruleset_prefix}" > "${state_file}"
  log Debug "rule-providers 前缀已更新: ${ruleset_prefix:-官方}"
  return 0
}

# 检查并更新订阅
# 参数: [force] true 时忽略 update_subscription 开关
upsubs() {
  local force="${1:-false}"

  if [ "${force}" != "true" ] && [ "${update_subscription}" != "true" ]; then
    log Warning "更新订阅已禁用: update_subscription=\"${update_subscription}\""
    return 1
  fi

  yq="yq"
  if ! command -v yq &>/dev/null; then
    if [ ! -e "${box_dir}/bin/yq" ]; then
      log Debug "yq 文件未找到, 开始从 GitHub 下载"
      ${scripts_dir}/box.tool upyq
    fi
    yq="${box_dir}/bin/yq"
  fi

  case "${bin_name}" in
    "mihomo")
      if [ ! -s "${subscriptions_file}" ]; then
        log Warning "订阅清单为空: ${subscriptions_file}"
        return 0
      fi

      log Info "${bin_name} 开始更新订阅 → $(date)"

      if [ -z "${mihomo_provide_path}" ] || ! mkdir -p "${mihomo_provide_path}"; then
        log Error "mihomo_provide_path 未定义或无法创建目录!"
        return 1
      fi

      local success_count=0
      local failure_count=0
      local total_count=0
      local renew_done=false

      while IFS='|' read -r provider_name provider_url; do
        case "${provider_name}" in
          ''|\#*) continue ;;
        esac

        if [ -z "${provider_url}" ]; then
          log Warning "订阅地址为空，跳过: ${provider_name}"
          failure_count=$((failure_count + 1))
          continue
        fi

        total_count=$((total_count + 1))
        local safe_name
        safe_name="$(sub_provider_file_name "${provider_name}")"
        local provider_file="${mihomo_provide_path}/${safe_name}.yaml"
        local temp_file="${provider_file}.download"

        log Info "--> 正在处理订阅: ${provider_name}"

        if [ "${renew}" = "true" ] && [ "${renew_done}" = "false" ]; then
          log Info "检测到 renew=true, 仅使用第一个订阅链接覆盖主配置"
          if LOG_MASK_URL=mask upfile "${mihomo_config}" "${provider_url}" "ClashMeta"; then
            log Info "${mihomo_config} 更新成功"
            reload_if_running
            log Info "${bin_name} 订阅更新完成 → $(date)"
            return 0
          else
            log Error "${mihomo_config} 更新失败"
            return 1
          fi
        fi

        rm -f "${temp_file}"

        if ! LOG_MASK_URL=mask upfile "${temp_file}" "${provider_url}" "ClashMeta"; then
          log Error "订阅下载失败: ${provider_name}"
          rm -f "${temp_file}"
          [ "${subscription_keep_last_good}" = "true" ] || rm -f "${provider_file}"
          failure_count=$((failure_count + 1))
          continue
        fi

        local proxy_count=0
        local valid_subscription=false

        if ${yq} 'has("proxies")' "${temp_file}" >/dev/null 2>&1; then
          local temp_proxies="${temp_file}.proxies"
          if ${yq} '.proxies' "${temp_file}" > "${temp_proxies}" 2>/dev/null && [ -s "${temp_proxies}" ]; then
            proxy_count="$(${yq} 'length' "${temp_proxies}" 2>/dev/null || echo 0)"
            if [ "${proxy_count}" -gt 0 ] 2>/dev/null; then
              ${yq} -i '{"proxies": .}' "${temp_proxies}"
              mv "${temp_proxies}" "${temp_file}"
              valid_subscription=true
            else
              log Error "订阅 proxies 为空: ${provider_name}"
              rm -f "${temp_proxies}" "${temp_file}"
            fi
          else
            log Error "订阅 proxies 提取失败: ${provider_name}"
            rm -f "${temp_proxies}" "${temp_file}"
          fi
        elif [ -n "$(base64 -d "${temp_file}" 2>/dev/null | grep -E "vless://|vmess://|ss://|hysteria://|hysteria2://|anytls://|trojan://" | head -n 1)" ]; then
          local decoded_file="${temp_file}.decoded"
          base64 -d "${temp_file}" > "${decoded_file}" 2>/dev/null
          proxy_count="$(grep -cE "vless://|vmess://|ss://|hysteria://|hysteria2://|anytls://|trojan://" "${decoded_file}" 2>/dev/null || true)"
          [ -n "${proxy_count}" ] || proxy_count=0
          mv "${decoded_file}" "${temp_file}"
          valid_subscription=true
        fi

        if [ "${valid_subscription}" != "true" ]; then
          log Error "订阅格式无法识别或内容为空: ${provider_name}"
          rm -f "${temp_file}"
          [ "${subscription_keep_last_good}" = "true" ] || rm -f "${provider_file}"
          failure_count=$((failure_count + 1))
          continue
        fi

        mv "${temp_file}" "${provider_file}"
        log Info "订阅 ${provider_name} 已保存（${proxy_count} 个节点）"
        success_count=$((success_count + 1))
      done < "${subscriptions_file}"

      log Info "成功更新 ${success_count} / ${total_count} 个订阅"

      if [ "${success_count}" -gt 0 ] && [ "${auto_modify_config}" = "true" ]; then
        log Info "正在更新 ${name_mihomo_config} 的 proxy-providers 与 rule-providers 配置..."
        update_mihomo_providers || log Warning "proxy-providers 配置更新失败，请手动检查配置文件"
        update_mihomo_rulesets || log Warning "rule-providers 前缀更新失败，请手动检查配置文件"
      elif [ "${auto_modify_config}" != "true" ]; then
        log Info "auto_modify_config 未启用，跳过更新 proxy-providers 配置"
      fi

      if [ "${failure_count}" -gt 0 ]; then
        log Error "${failure_count} 个订阅更新失败"
        if [ "${success_count}" -gt 0 ]; then
          return 2
        fi
        return 1
      fi

      log Info "更新订阅于 $(date +"%F %R")"
      return 0
      ;;
    "sing-box")
      update_file_name="${sing_config}"
      if [ -n "${subscription_url_singbox}" ]; then
        log Info "${bin_name} 每日更新订阅 → $(date)"
        log Debug "正在下载 ${update_file_name}"
        if upfile "${update_file_name}" "${subscription_url_singbox}" "sing-box"; then
          log Info "${update_file_name} 已保存"
          log Info "更新订阅于 $(date +"%F %R")"
          reload_if_running
          return 0
        else
          log Error "更新订阅失败"
          return 1
        fi
      else
        log Warning "${bin_name} 订阅链接为空..."
        return 1
      fi
      ;;
    "xray"|"v2fly"|"hysteria")
      log Warning "${bin_name} 不支持订阅功能.."
      return 1
      ;;
    *)
      log Error "<${bin_name}> 未知的二进制文件."
      return 1
      ;;
  esac
}

reload_if_running() {
  if [ -f "${box_pid}" ] && kill -0 "$(<"${box_pid}" 2>/dev/null)" 2>/dev/null; then
    reload
  fi
}

sub_usage() {
  cat <<'EOF'
用法:
  box.tool sub add <订阅地址> [名字]
  box.tool sub list
  box.tool sub rm <名字>
  box.tool sub update
EOF
}

sub_add() {
  local url="$1"
  local name="$2"

  case "${url}" in
    http://*|https://*) ;;
    *)
      log Error "订阅地址必须以 http:// 或 https:// 开头"
      return 1
      ;;
  esac

  if [ -z "${name}" ]; then
    name="$(printf '%s' "${url}" | busybox sed -E 's#^https?://([^/]+).*#\1#')"
  fi

  case "${name}" in
    *"|"*)
      log Error "名字不能包含 |"
      return 1
      ;;
  esac

  mkdir -p "$(dirname "${subscriptions_file}")" || return 1
  touch "${subscriptions_file}"

  if subscription_exists "${name}"; then
    log Error "订阅已存在: ${name}"
    return 1
  fi

  local new_safe existing_name
  new_safe="$(sub_provider_file_name "${name}")"
  while IFS='|' read -r existing_name _; do
    case "${existing_name}" in
      ''|\#*) continue ;;
    esac
    if [ "$(sub_provider_file_name "${existing_name}")" = "${new_safe}" ]; then
      log Error "订阅名生成的节点文件名冲突: ${existing_name}"
      return 1
    fi
  done < "${subscriptions_file}"

  printf '%s|%s\n' "${name}" "${url}" >> "${subscriptions_file}"
  log Info "已添加订阅: ${name}"
  log Info "执行 box.tool sub update 拉取节点"
  return 0
}

sub_list() {
  if [ ! -s "${subscriptions_file}" ]; then
    log Info "订阅清单为空: ${subscriptions_file}"
    return 0
  fi

  printf '%-20s %-44s %s\n' "名字" "地址" "节点文件"

  while IFS='|' read -r name url; do
    case "${name}" in
      ''|\#*) continue ;;
    esac

    local safe_name provider_file state
    safe_name="$(sub_provider_file_name "${name}")"
    provider_file="${mihomo_provide_path}/${safe_name}.yaml"

    if [ -s "${provider_file}" ]; then
      state="已下载"
    else
      state="未下载"
    fi

    printf '%-20s %-44s %s\n' "${name}" "$(mask_url "${url}")" "${state}"
  done < "${subscriptions_file}"
}

sub_rm() {
  local name="$1"

  if [ -z "${name}" ]; then
    sub_usage
    return 1
  fi

  if ! subscription_exists "${name}"; then
    log Error "订阅不存在: ${name}"
    return 1
  fi

  local temp_file="${subscriptions_file}.tmp"
  if busybox awk -F'|' -v name="${name}" '$1 != name' "${subscriptions_file}" > "${temp_file}"; then
    mv "${temp_file}" "${subscriptions_file}"
  else
    rm -f "${temp_file}"
    log Error "删除订阅失败: ${name}"
    return 1
  fi

  local safe_name provider_file
  safe_name="$(sub_provider_file_name "${name}")"
  provider_file="${mihomo_provide_path}/${safe_name}.yaml"
  rm -f "${provider_file}" "${provider_file}.download"

  log Info "已删除订阅: ${name}"
  return 0
}

sub_update() {
  upsubs true
  local status=$?

  if [ "${status}" -ne 1 ]; then
    reload_if_running
  fi

  return "${status}"
}

# ---------------------------------------------------------------------------
# 管理页面 JSON 接口
# ---------------------------------------------------------------------------

json_quote() {
  local escaped
  escaped="$(printf '%s' "$1" | busybox sed 's/\\/\\\\/g; s/"/\\"/g' | busybox awk 'BEGIN { ORS = "" } { if (NR > 1) printf "\\n"; printf "%s", $0 }')"
  printf '"%s"' "${escaped}"
}

json_bool() {
  if [ "$1" = "true" ]; then printf 'true'; else printf 'false'; fi
}

ensure_yq() {
  yq="yq"
  if ! command -v yq &>/dev/null; then
    if [ ! -e "${box_dir}/bin/yq" ]; then
      ${scripts_dir}/box.tool upyq
    fi
    yq="${box_dir}/bin/yq"
  fi
}

status_json() {
  local running=false
  local pid="null"
  if [ -f "${box_pid}" ]; then
    local current_pid
    current_pid="$(cat "${box_pid}" 2>/dev/null)"
    if [ -n "${current_pid}" ] && kill -0 "${current_pid}" 2>/dev/null; then
      running=true
      pid="${current_pid}"
    fi
  fi

  printf '{"running":%s,"pid":%s,"core":%s,"mode":%s,"config":%s,"subscriptions":%s}\n' \
    "$(json_bool "${running}")" \
    "${pid}" \
    "$(json_quote "${bin_name}")" \
    "$(json_quote "${network_mode}")" \
    "$(json_quote "${name_mihomo_config}")" \
    "$(subscription_count)"
}

subscription_count() {
  if [ ! -s "${subscriptions_file}" ]; then
    printf '0'
    return 0
  fi
  busybox awk -F'|' '!/^[[:space:]]*#/ && NF >= 2 && $1 != "" { count++ } END { print count + 0 }' "${subscriptions_file}"
}

logs_json() {
  local line_count="${1:-200}"
  case "${line_count}" in
    ''|*[!0-9]*) line_count=200 ;;
  esac

  local file="${box_run}/runs.log"
  [ -f "${file}" ] || file="${box_log}"

  if [ ! -f "${file}" ]; then
    printf '{"file":%s,"lines":[]}\n' "$(json_quote "${file}")"
    return 0
  fi

  printf '{"file":%s,"lines":[' "$(json_quote "${file}")"
  tail -n "${line_count}" "${file}" 2>/dev/null | busybox awk '
    BEGIN { first = 1 }
    {
      line = $0
      gsub(/\\/, "\\\\", line)
      gsub(/"/, "\\\"", line)
      gsub(/\t/, "\\t", line)
      gsub(/\r/, "", line)
      if (!first) printf ","
      printf "\"%s\"", line
      first = 0
    }
    END { print "]}" }
  '
}

sub_list_json() {
  printf '{"items":['
  local first=1
  if [ -s "${subscriptions_file}" ]; then
    while IFS='|' read -r name url; do
      case "${name}" in
        ''|\#*) continue ;;
      esac
      local safe_name provider_file state
      safe_name="$(sub_provider_file_name "${name}")"
      provider_file="${mihomo_provide_path}/${safe_name}.yaml"
      if [ -s "${provider_file}" ]; then state="已下载"; else state="未下载"; fi
      [ "${first}" -eq 1 ] || printf ','
      printf '{"name":%s,"url":%s,"provider":%s}' \
        "$(json_quote "${name}")" "$(json_quote "${url}")" "$(json_quote "${state}")"
      first=0
    done < "${subscriptions_file}"
  fi
  printf ']}\n'
}

settings_keys() {
  cat <<'EOF'
boot_auto_start bin_name network_mode tproxy_port redir_port ipv6 performance_mode quic mihomo_dns_forward proxy_tcp proxy_udp dns_hijack_tcp dns_hijack_udp dns_hijack_mode bypass_cn_ip bypass_cn_ip_v4 bypass_cn_ip_v6 cnip_mode proxy_mode update_subscription update_geo use_ghproxy url_ghproxy mihomo_stable singbox_stable run_crontab interva_update cgroup_memcg memcg_limit taskset_cpu allow_cpu cgroup_blkio weight mac_filter mac_mode ruleset_prefix
EOF
}

settings_array_keys() {
  cat <<'EOF'
cnip_force_proxy_apps force_proxy_cidrs force_proxy_cidrs6 macs_list wifi_ssids_list wifi_bssids_list
EOF
}

settings_value() {
  local key="$1"
  eval "printf '%s' \"\${${key}}\""
}

settings_get_json() {
  printf '{"settings":{'
  local first=1
  for key in $(settings_keys); do
    local value
    value="$(settings_value "${key}")"
    [ "${first}" -eq 1 ] || printf ','
    printf '%s:%s' "$(json_quote "${key}")" "$(json_quote "${value}")"
    first=0
  done
  printf '},"arrays":{'
  first=1
  for key in $(settings_array_keys); do
    [ "${first}" -eq 1 ] || printf ','
    printf '%s:[' "$(json_quote "${key}")"
    eval "items=\"\${${key}[*]}\""
    local array_first=1
    for item in ${items}; do
      [ "${array_first}" -eq 1 ] || printf ','
      printf '%s' "$(json_quote "${item}")"
      array_first=0
    done
    printf ']'
    first=0
  done
  printf '}}\n'
}

settings_apply_json() {
  local file="$1"
  if [ ! -s "${file}" ]; then
    log Error "settings apply: 缺少 JSON 文件"
    return 1
  fi

  ensure_yq

  local changed=""
  local key

  for key in $(settings_keys); do
    if ${yq} -e "has(\"${key}\")" "${file}" >/dev/null 2>&1; then
      local value
      value="$(${yq} -r ".${key} // \"\"" "${file}" 2>/dev/null)"
      value="$(printf '%s' "${value}" | busybox sed 's/[&\\#]/\\&/g')"
      busybox sed -i "s#^${key}=.*#${key}=\"${value}\"#" "${settings_file}"
      changed="${changed}${changed:+,}$(json_quote "${key}")"
    fi
  done

  for key in $(settings_array_keys); do
    if ${yq} -e "has(\"${key}\")" "${file}" >/dev/null 2>&1; then
      local joined
      joined="$(${yq} -r ".${key} | join(\" \")" "${file}" 2>/dev/null)"
      busybox sed -i "s#^${key}=.*#${key}=(${joined})#" "${settings_file}"
      changed="${changed}${changed:+,}$(json_quote "${key}")"
    fi
  done

  local sh_bin="/system/bin/sh"
  if [ ! -x "${sh_bin}" ]; then
    if command -v bash >/dev/null 2>&1; then
      sh_bin="bash"
    elif command -v mksh >/dev/null 2>&1; then
      sh_bin="mksh"
    else
      sh_bin=""
    fi
  fi
  if [ -n "${sh_bin}" ] && ! ${sh_bin} -n "${settings_file}" 2>/dev/null; then
    log Error "settings.ini 语法校验失败，已放弃本次修改"
    return 1
  fi

  printf '{"ok":true,"changed":[%s]}\n' "${changed}"
}

config_get_json() {
  ensure_yq
  if [ ! -f "${mihomo_config}" ]; then
    log Error "配置文件不存在: ${mihomo_config}"
    return 1
  fi
  ${yq} -o=json '{ "proxy-groups": ."proxy-groups", "rule-providers": ."rule-providers", "rules": .rules }' "${mihomo_config}"
}

config_apply_json() {
  local file="$1"
  if [ ! -s "${file}" ]; then
    log Error "config apply: 缺少 JSON 文件"
    return 1
  fi
  if [ ! -f "${mihomo_config}" ]; then
    log Error "配置文件不存在: ${mihomo_config}"
    return 1
  fi

  ensure_yq

  if ! ${yq} -e 'has("proxy-groups") and has("rule-providers") and has("rules")' "${file}" >/dev/null 2>&1; then
    log Error "config apply: JSON 必须包含 proxy-groups、rule-providers、rules"
    return 1
  fi
  if ! ${yq} -e '."proxy-groups" | tag == "!!seq"' "${file}" >/dev/null 2>&1; then
    log Error "config apply: proxy-groups 必须是数组"
    return 1
  fi
  if ! ${yq} -e '.rules | tag == "!!seq"' "${file}" >/dev/null 2>&1; then
    log Error "config apply: rules 必须是数组"
    return 1
  fi

  local backup="${mihomo_config}.bak"
  cp -f "${mihomo_config}" "${backup}" 2>/dev/null

  if ! ${yq} -i '."proxy-groups" = (load("'"${file}"'") | ."proxy-groups") | ."rule-providers" = (load("'"${file}"'") | ."rule-providers") | .rules = (load("'"${file}"'") | .rules)' "${mihomo_config}" 2>/dev/null; then
    cp -f "${backup}" "${mihomo_config}" 2>/dev/null
    log Error "config apply: 写入失败，已还原"
    return 1
  fi

  if ! ${yq} '.' "${mihomo_config}" >/dev/null 2>&1; then
    cp -f "${backup}" "${mihomo_config}" 2>/dev/null
    log Error "config apply: YAML 校验失败，已还原"
    return 1
  fi

  printf '{"ok":true}\n'
}

config_delete_json() {
  if [ ! -f "${mihomo_config}" ]; then
    log Error "配置文件不存在: ${mihomo_config}"
    return 1
  fi
  local deleted="${mihomo_config}.deleted.$(date +%Y%m%d%H%M%S)"
  mv "${mihomo_config}" "${deleted}"
  printf '{"ok":true,"path":%s}\n' "$(json_quote "${deleted}")"
}

logs_clear_json() {
  local file
  for file in "${box_run}/runs.log" "${box_run}/tool.log" "${box_run}/${bin_name}.log"; do
    [ -f "${file}" ] && : > "${file}"
  done
  printf '{"ok":true}\n'
}

reset_subscriptions_json() {
  : > "${subscriptions_file}"
  rm -f "${mihomo_provide_path}"/*.yaml 2>/dev/null
  printf '{"ok":true}\n'
}

upkernel() {
  setup_github_api
  
  local core_to_update="$1"
  if [ -z "$core_to_update" ]; then
    log Error "upkernel: 未提供核心名称"
    return 1
  fi

  local target_bin_name="${core_to_update}"
  case "${core_to_update}" in
    "mihomo_smart")
      target_bin_name="mihomo"
      ;;
    "sing-box_ref1nd")
      target_bin_name="sing-box"
      ;;
  esac

  mkdir -p "${bin_dir}/backup"
  if [ -f "${bin_dir}/${target_bin_name}" ]; then
    cp "${bin_dir}/${target_bin_name}" "${bin_dir}/backup/${target_bin_name}.bak" >/dev/null 2>&1
  fi
  case $(uname -m) in
    "aarch64") 
      if [ "$core_to_update" = "mihomo" ]; then 
        arch="arm64-v8"
      else 
        arch="arm64"
      fi
      platform="android"
      ;;
    "armv7l"|"armv8l") arch="armv7"; platform="linux" ;;
    "i686") arch="386"; platform="linux" ;;
    "x86_64") arch="amd64"; platform="linux" ;;
    *) log Warning "不支持的架构: $(uname -m)" >&2; return 1 ;;
  esac
  
  local file_kernel="${core_to_update}-${arch}"
  case "${core_to_update}" in
    "mihomo_smart")
      log Info "正在更新 mihomo-smart 核心 (来自 vernesong/mihomo)"
      local arch_smart
      case $(uname -m) in
        "aarch64") arch_smart="arm64-v8" ;;
        *) log Error "mihomo-smart 当前仅支持 aarch64 架构"; return 1 ;;
      esac

      local release_page_url="https://github.com/vernesong/mihomo/releases/expanded_assets/Prerelease-Alpha"
      [ "${use_ghproxy}" = "true" ] && release_page_url="${url_ghproxy}/${release_page_url}"
      
      local smart_version_tag=$($rev1 "${release_page_url}" | busybox grep -oE "smart-[a-f0-9]+" | head -1)

      if [ -z "$smart_version_tag" ]; then
        log Error "获取 mihomo-smart 最新版本标签失败"
        return 1
      fi

      local download_link="https://github.com/vernesong/mihomo/releases/download/Prerelease-Alpha/mihomo-android-${arch_smart}-alpha-${smart_version_tag}.gz"
      local file_kernel="${core_to_update}-${arch_smart}"
      
      log Debug "下载 ${download_link}"
      upfile "${box_dir}/${file_kernel}.gz" "${download_link}" && xkernel "$core_to_update" "" "" "" "$file_kernel"
      ;;
    "sing-box_ref1nd")
      log Info "正在更新 sing-box reF1nd 核心"
      local api_url="https://api.github.com/repos/reF1nd/sing-box-releases/releases"
      local url_down="https://github.com/reF1nd/sing-box-releases/releases"

      if [ "${singbox_stable}" = "disable" ]; then
        log Debug "正在下载 ${core_to_update} 预发布版本"
        latest_version=$($rev1 "${api_url}" | grep "tag_name" | busybox grep -oE "v[0-9].*" | head -1 | cut -d'"' -f1)
      else
        log Debug "正在下载 ${core_to_update} 最新稳定版本"
        latest_version=$($rev1 "${api_url}/latest" | grep "tag_name" | busybox grep -oE "v[0-9].*" | head -1 | cut -d'"' -f1)
      fi

      if [ -z "$latest_version" ]; then
        log Error "获取 sing-box reF1nd 最新版本失败"
        return 1
      fi

      local file_kernel="${core_to_update}-${arch}"
      local download_link="${url_down}/download/${latest_version}/sing-box-${latest_version#v}-${platform}-${arch}.tar.gz"
      log Debug "下载 ${download_link}"
      upfile "${box_dir}/${file_kernel}.tar.gz" "${download_link}" && xkernel "$core_to_update" "$platform" "$arch" "$latest_version" "$file_kernel"
      ;;
    "sing-box")
      api_url="https://api.github.com/repos/SagerNet/sing-box/releases"
      url_down="https://github.com/SagerNet/sing-box/releases"

      if [ "${singbox_stable}" = "disable" ]; then
        log Debug "下载 ${core_to_update} 预发行版"
        latest_version=$($rev1 "${api_url}" | grep "tag_name" | busybox grep -oE "v[0-9].*" | head -1 | cut -d'"' -f1)
      else
        log Debug "下载 ${core_to_update} 最新稳定版"
        latest_version=$($rev1 "${api_url}/latest" | grep "tag_name" | busybox grep -oE "v[0-9.]*" | head -1)
      fi

      if [ -z "$latest_version" ]; then
        log Error "获取 sing-box 最新 稳定版/测试版/Alpha版 失败"
        return 1
      fi

      download_link="${url_down}/download/${latest_version}/sing-box-${latest_version#v}-${platform}-${arch}.tar.gz"
      log Debug "下载 ${download_link}"
      upfile "${box_dir}/${file_kernel}.tar.gz" "${download_link}" && xkernel "$core_to_update" "$platform" "$arch" "$latest_version" "$file_kernel"
      ;;
    "mihomo")
      download_link="https://github.com/MetaCubeX/mihomo/releases"

      if [ "${mihomo_stable}" = "enable" ]; then
        latest_version=$($rev1 "https://api.github.com/repos/MetaCubeX/mihomo/releases" | grep "tag_name" | busybox grep -oE "v[0-9.]*" | head -1)
        tag="$latest_version"
      else
        if [ "$use_ghproxy" == true ]; then
          download_link="${url_ghproxy}/${download_link}"
        fi
        tag="Prerelease-Alpha"
        latest_version=$($rev1 "${download_link}/expanded_assets/${tag}" | busybox grep -oE "alpha-[0-9a-z]+" | head -1)
      fi

      if [ -z "$latest_version" ]; then
        log Error "获取 mihomo 最新 稳定版/预发行版 失败"
        return 1
      fi

      local extension="gz"
      if [ "${platform}" = "android" ]; then
        extension="zip"
      fi

      filename="mihomo-${platform}-${arch}-${latest_version}"
      log Debug "下载 ${download_link}/download/${tag}/${filename}.gz"
      upfile "${box_dir}/${file_kernel}.gz" "${download_link}/download/${tag}/${filename}.gz" && xkernel "$core_to_update" "" "" "" "$file_kernel"
      ;;
    "xray"|"v2fly")
      [ "${core_to_update}" = "xray" ] && bin='Xray' || bin='v2ray'
      api_url="https://api.github.com/repos/$(if [ "${core_to_update}" = "xray" ]; then echo "XTLS/Xray-core/releases"; else echo "v2fly/v2ray-core/releases"; fi)"
      latest_version=$($rev1 ${api_url} | grep "tag_name" | busybox grep -oE "v[0-9.]*" | head -1)

      if [ -z "$latest_version" ]; then
        log Error "获取 ${core_to_update} 最新版本号失败"
        return 1
      fi

      case $(uname -m) in
        "i386") download_file="$bin-linux-32.zip" ;;
        "x86_64") download_file="$bin-linux-64.zip" ;;
        "armv7l"|"armv8l") download_file="$bin-linux-arm32-v7a.zip" ;;
        "aarch64") download_file="$bin-android-arm64-v8a.zip" ;;
        *) log Error "不支持的架构: $(uname -m)" >&2; return 1 ;;
      esac
      download_link="https://github.com/$(if [ "${core_to_update}" = "xray" ]; then echo "XTLS/Xray-core/releases"; else echo "v2fly/v2ray-core/releases"; fi)"
      log Debug "正在下载 ${download_link}/download/${latest_version}/${download_file}"
      upfile "${box_dir}/${file_kernel}.zip" "${download_link}/download/${latest_version}/${download_file}" && xkernel "$core_to_update" "" "" "" "$file_kernel"
      ;;
    "hysteria")
      local arch
      case $(uname -m) in
        "aarch64") arch="arm64" ;;
        "armv7l" | "armv8l") arch="armv7" ;;
        "i686") arch="386" ;;
        "x86_64") arch="amd64" ;;
        *)
          log Warning "不支持的架构: $(uname -m)"
          return 1
          ;;
      esac
      mkdir -p "${bin_dir}/backup"
      if [ -f "${bin_dir}/hysteria" ]; then
        cp "${bin_dir}/hysteria" "${bin_dir}/backup/hysteria.bak" >/dev/null 2>&1
      fi
      local latest_version=$($rev1 "https://api.github.com/repos/apernet/hysteria/releases" | grep "tag_name" | grep -oE "[0-9.].*" | head -1 | sed 's/,//g' | cut -d '"' -f 1)

      if [ -z "$latest_version" ]; then
        log Error "获取 hysteria 最新版本号失败"
        return 1
      fi

      local download_link="https://github.com/apernet/hysteria/releases/download/app%2Fv${latest_version}/hysteria-android-${arch}"

      log Debug "正在下载 ${download_link}"
      upfile "${bin_dir}/hysteria" "${download_link}" && xkernel "$core_to_update"
      ;;
    *)
      log Error "<${core_to_update}> 未知的二进制文件."
      return 1
      ;;
  esac
}

upkernels() {
  for core in "$@"; do
    upkernel "$core"
  done
}

xkernel() {
  local core_to_process="$1"
  local platform="$2"
  local arch="$3"
  local latest_version="$4"
  local file_kernel="$5"
  
  local original_bin_name=$bin_name
  local target_bin_name="$core_to_process"
  if [ "$core_to_process" = "mihomo_smart" ]; then
    target_bin_name="mihomo"
  elif [ "$core_to_process" = "sing-box_ref1nd" ]; then
    target_bin_name="sing-box"
  fi
  
  bin_name=$core_to_process

  case "${core_to_process}" in
    "mihomo"|"mihomo_smart")
      gunzip_command="gunzip"
      if ! command -v gunzip >/dev/null; then
        gunzip_command="busybox gunzip"
      fi

      if ${gunzip_command} -f "${box_dir}/${file_kernel}.gz" >&2 && mv "${box_dir}/${file_kernel}" "${bin_dir}/${target_bin_name}"; then
        log Info "${target_bin_name} 已成功更新 (来自: ${core_to_process})"
      else
        log Error "解压或移动 ${target_bin_name} 核心失败."
        bin_name=$original_bin_name
        return 1
      fi
      ;;
    "sing-box"|"sing-box_ref1nd")
      tar_command="tar"
      if ! command -v tar >/dev/null; then
        tar_command="busybox tar"
      fi
      log Info "正在解压 Sing-Box 核心..."
      if ${tar_command} -xf "${box_dir}/${file_kernel}.tar.gz" -C "${bin_dir}" >/dev/null; then
        mv "${bin_dir}/sing-box-${latest_version#v}-${platform}-${arch}/sing-box" "${bin_dir}/${target_bin_name}"
        if [ -f "${box_pid}" ]; then
          rm -rf /data/adb/box/sing-box/cache.db
          restart_box "$target_bin_name"
        else
          log Debug "${core_to_process} 无需重启."
        fi
      else
        log Error "解压 ${box_dir}/${file_kernel}.tar.gz 失败."
      fi
      [ -d "${bin_dir}/sing-box-${latest_version#v}-${platform}-${arch}" ] && \
        rm -r "${bin_dir}/sing-box-${latest_version#v}-${platform}-${arch}"
      ;;
    "v2fly"|"xray")
      bin="xray"
      if [ "${core_to_process}" != "xray" ]; then
        bin="v2ray"
      fi
      unzip_command="unzip"
      if ! command -v unzip >/dev/null; then
        unzip_command="busybox unzip"
      fi

      mkdir -p "${bin_dir}/update"
      log Info "正在解压 ${bin} 核心..."
      if ${unzip_command} -oq "${box_dir}/${file_kernel}.zip" "${bin}" -d "${bin_dir}/update"; then
        if mv "${bin_dir}/update/${bin}" "${bin_dir}/${core_to_process}"; then
          true # 成功
        else
          log Error "移动核心失败."
          rm -rf "${bin_dir}/update"
          return 1
        fi
      else
        log Error "解压 ${box_dir}/${file_kernel}.zip 失败."
        rm -rf "${bin_dir}/update"
        return 1
      fi
      rm -rf "${bin_dir}/update"
      ;;
    "hysteria")
      true
      ;;
    *)
      log Error "<${core_to_process}> 未知的二进制文件."
      bin_name=$original_bin_name
      return 1
      ;;
  esac

  find "${box_dir}" -maxdepth 1 -type f -name "${file_kernel}.*" -delete

  chown ${box_user_group} "${bin_dir}/${target_bin_name}"
  chmod 0755 "${bin_dir}/${target_bin_name}"
  
  if [ -f "${box_pid}" ]; then
    if [ "$original_bin_name" = "$target_bin_name" ]; then
      log Info "检测到正在运行的核心已被更新，将自动重启服务..."
      restart_box "$target_bin_name"
    else
      log Info "${target_bin_name} 已更新，但当前运行的是 ${original_bin_name}，无需重启。"
    fi
  else
    log Info "服务未在运行，无需重启。"
  fi
  
  bin_name=$original_bin_name
}
upxui() {
  if [[ "${bin_name}" == @(mihomo|sing-box) ]]; then
    local ui_path
    local ui_url
    if [ "${bin_name}" = "mihomo" ]; then
      ui_path=$(busybox awk '!/^ *#/ && /^external-ui:/ {print $2; exit}' "${mihomo_config}" | sed "s/[\"']//g")
      ui_url=$(busybox awk '!/^ *#/ && /^external-ui-url:/ {print $2; exit}' "${mihomo_config}" 2>/dev/null | sed "s/[\"']//g")
    else
      ui_path=$(busybox awk -F '"' '/"external_ui"/ {print $4}' "${sing_config}" | head -n 1)
      ui_url=$(busybox awk -F '"' '/"external_ui_download_url"/ {print $4; exit}' "${sing_config}" 2>/dev/null)
    fi
    
    if [ -z "${ui_path}" ]; then
      ui_path="./dashboard"
      log Warning "配置文件中未找到 external-ui/external_ui 字段，使用默认路径: ${ui_path}"
    fi
    log Debug "配置文件中的 UI 路径: ${ui_path}"
    
    local dashboard_dir
    if [[ "${ui_path}" == ./* ]]; then
      dashboard_dir="${box_dir}/${bin_name}/${ui_path#./}"
    elif [[ "${ui_path}" == /* ]]; then
      dashboard_dir="${ui_path}"
    else
      dashboard_dir="${box_dir}/${bin_name}/${ui_path}"
    fi
    log Info "Dashboard 目标目录: ${dashboard_dir}"
    
    file_dashboard="${box_dir}/${bin_name}_dashboard.zip"
    if [ -n "${ui_url}" ]; then
      url="${ui_url}"
    else
      url="https://github.com/Zephyruso/zashboard/releases/latest/download/dist.zip"
    fi
    
    if upfile "${file_dashboard}" "${url}"; then
      if [ ! -d "${dashboard_dir}" ]; then
        log Info "面板文件夹不存在, 正在创建: ${dashboard_dir}"
        mkdir -p "${dashboard_dir}"
      else
        log Debug "清理现有面板文件: ${dashboard_dir}"
        rm -rf "${dashboard_dir}/"*
      fi
      
      if command -v unzip >/dev/null; then
        unzip_command="unzip"
      else
        unzip_command="busybox unzip"
      fi
      
      log Info "正在解压 Dashboard..."
      local temp_extract_dir="${box_dir}/${bin_name}_dashboard_temp"
      rm -rf "${temp_extract_dir}"
      mkdir -p "${temp_extract_dir}"
      
      if "${unzip_command}" -oq "${file_dashboard}" -d "${temp_extract_dir}"; then
        local html_path
        html_path=$(busybox find "${temp_extract_dir}" -maxdepth 3 -type f -name index.html | head -n 1)
        if [ -n "${html_path}" ]; then
          local html_dir
          html_dir="${html_path%/*}"
          mv -f "${html_dir}"/* "${dashboard_dir}/"
        else
          local top_dir
          top_dir=$(find "${temp_extract_dir}" -mindepth 1 -maxdepth 1 -type d | head -n 1)
          if [ -n "${top_dir}" ] && [ -z "$(find "${temp_extract_dir}" -mindepth 1 -maxdepth 1 -type d | sed -n '2p')" ]; then
            mv -f "${top_dir}"/* "${dashboard_dir}/"
          else
            mv -f "${temp_extract_dir}"/* "${dashboard_dir}/"
          fi
        fi
      else
        log Error "解压 Dashboard 失败"
        rm -f "${file_dashboard}"
        rm -rf "${temp_extract_dir}"
        return 1
      fi

      if [ -z "$(find "${dashboard_dir}" -mindepth 1 -maxdepth 1 | head -n 1)" ]; then
        log Error "Dashboard 目录为空，更新失败"
        rm -f "${file_dashboard}"
        rm -rf "${temp_extract_dir}"
        return 1
      fi
      rm -f "${file_dashboard}"
      rm -rf "${temp_extract_dir}"
      log Info "Dashboard 更新成功 → ${dashboard_dir}"
    else
      log Error "下载 Dashboard 失败"
      return 1
    fi
    return 0
  else
    log Debug "${bin_name} 不支持面板"
    return 1
  fi
}

cgroup_blkio() {
  local pid_file="$1"
  local fallback_weight="${2:-900}"

  if [ -z "$pid_file" ] || [ ! -f "$pid_file" ]; then
    log Warning "PID 文件丢失或无效: $pid_file"
    return 1
  fi

  local PID=$(<"$pid_file" 2>/dev/null)
  if [ -z "$PID" ] || ! kill -0 "$PID" >/dev/null 2>&1; then
    log Warning "来自 ${pid_file} 的 PID $PID 无效或未运行"
    return 1
  fi

  if [ -z "$blkio_path" ]; then
    blkio_path=$(mount | grep cgroup | busybox awk '/blkio/{print $3}' | head -1)
    if [ -z "$blkio_path" ] || [ ! -d "$blkio_path" ]; then
      log Warning "blkio cgroup 路径未找到"
      return 1
    fi
  fi

  local target="${blkio_path}/box"
  
  if [ ! -d "$target" ]; then
    mkdir -p "$target" 2>/dev/null
    if [ ! -d "$target" ]; then
      log Warning "无法创建 box blkio 目录: $target"
      if [ -d "${blkio_path}/foreground" ]; then
        target="${blkio_path}/foreground"
        log Info "回退使用现有 blkio 目录: foreground"
      elif [ -d "${blkio_path}/top-app" ]; then
        target="${blkio_path}/top-app"
        log Info "回退使用现有 blkio 目录: top-app"
      else
        log Warning "blkio 目标未找到，无法设置 IO 权重"
        return 1
      fi
    else
      log Info "成功创建专用 box blkio 目录: $target"
      echo "$fallback_weight" > "${target}/blkio.weight" 2>/dev/null
      if [ $? -ne 0 ]; then
        log Warning "无法设置 blkio 权重到 ${target}/blkio.weight"
      else
        log Info "已设置 blkio 权重: $fallback_weight"
      fi
    fi
  fi

  echo "$PID" > "${target}/cgroup.procs" 2>/dev/null
  if [ $? -eq 0 ]; then
    log Info "已分配 PID $PID 到 ${target}，IO 权重 [$fallback_weight]"
    return 0
  else
    log Warning "无法将 PID $PID 分配到 ${target}"
    return 1
  fi
}

cgroup_memcg() {
  local pid_file="$1"
  local raw_limit="$2"

  if [ -z "$pid_file" ] || [ ! -f "$pid_file" ]; then
    log Warning "PID 文件丢失或无效: $pid_file"
    return 1
  fi

  if [ -z "$raw_limit" ]; then
    log Warning "未指定 memcg 限制"
    return 1
  fi

  local limit
  case "$raw_limit" in
    *[Mm])
      limit=$(( ${raw_limit%[Mm]} * 1024 * 1024 ))
      ;;
    *[Gg])
      limit=$(( ${raw_limit%[Gg]} * 1024 * 1024 * 1024 ))
      ;;
    *[Kk])
      limit=$(( ${raw_limit%[Kk]} * 1024 ))
      ;;
    *[0-9])
      limit=$raw_limit
      ;;
    *)
      log Warning "无效的 memcg 限制格式: $raw_limit"
      return 1
      ;;
  esac

  local PID
  PID=$(<"$pid_file" 2>/dev/null)
  if [ -z "$PID" ] || ! kill -0 "$PID" >/dev/null 2>&1; then
    log Warning "来自 ${pid_file} 的 PID $PID 无效或未运行"
    return 1
  fi

  if [ -z "$memcg_path" ]; then
    memcg_path=$(mount | grep cgroup | busybox awk '/memory/{print $3}' | head -1)
    if [ -z "$memcg_path" ] || [ ! -d "$memcg_path" ]; then
      log Warning "memory cgroup 路径未找到"
      return 1
    fi
  fi

  local target="${memcg_path}/box"
  
  if [ ! -d "$target" ]; then
    mkdir -p "$target" 2>/dev/null
    if [ ! -d "$target" ]; then
      log Warning "无法创建 box memory 目录: $target"
      local name="${bin_name:-app}"
      target="${memcg_path}/${name}"
      mkdir -p "$target" 2>/dev/null
      if [ ! -d "$target" ]; then
        log Warning "无法创建 memory 目录: $target"
        return 1
      fi
      log Info "回退使用 memory 目录: $target"
    else
      log Info "成功创建专用 box memory 目录: $target"
    fi
  fi

  local hr_limit="$limit B"
  if [ "$limit" -ge 1073741824 ]; then
    hr_limit="$(busybox awk -v b=$limit 'BEGIN{printf "%.2f GiB", b/1073741824}')"
  elif [ "$limit" -ge 1048576 ]; then
    hr_limit="$(busybox awk -v b=$limit 'BEGIN{printf "%.2f MiB", b/1048576}')"
  elif [ "$limit" -ge 1024 ]; then
    hr_limit="$(busybox awk -v b=$limit 'BEGIN{printf "%.2f KiB", b/1024}')"
  fi

  echo "$limit" > "${target}/memory.limit_in_bytes" 2>/dev/null
  if [ $? -ne 0 ]; then
    log Warning "无法设置内存限制到 ${target}/memory.limit_in_bytes"
    return 1
  fi
  log Info "已设置内存限制: ${hr_limit} (${limit} 字节)"

  echo "$PID" > "${target}/cgroup.procs" 2>/dev/null
  if [ $? -eq 0 ]; then
    log Info "已分配 PID $PID 到 ${target}，内存限制 [$hr_limit]"
    return 0
  else
    log Warning "无法将 PID $PID 分配到 ${target}"
    return 1
  fi
}

webroot() {
  path_webroot="/data/adb/modules/box_for_root/webroot/index.html"
  if [ -s "$path_webroot" ] && grep -q 'box-managed-ui' "$path_webroot" 2>/dev/null; then
    return 0
  fi

  ip_port=$(if [ "${bin_name}" = "mihomo" ]; then busybox awk '/external-controller:/ {print $2}' "${mihomo_config}"; else busybox awk -F'[:,]' '/"external_controller"/ {print $2":"$3}' "${sing_config}" | sed 's/^[ \t]*//;s/"//g'; fi;)
  secret=$(if [ "${bin_name}" = "mihomo" ]; then busybox awk '/^secret:/ {print $2}' "${mihomo_config}" | sed 's/"//g'; else busybox awk -F'"' '/"secret"/ {print $4}' "${sing_config}" | head -n 1; fi;)
  touch "$path_webroot"
  if [[ "${bin_name}" = @(mihomo|sing-box) ]]; then
    echo -e '
  <!DOCTYPE html>
  <script>
      document.location = 'http://127.0.0.1:9090/ui/'
  </script>
  </html>
  ' > $path_webroot
    sed -i "s#document\.location =.*#document.location = 'http://$ip_port/ui/'#" $path_webroot
  else
   echo -e '
  <!DOCTYPE html>
  <html lang="zh-CN">
  <head>
      <meta charset="UTF-8">
      <meta name="viewport" content="width=device-width, initial-scale=1.0">
      <title>不支持WebUI</title>
      <style>
          body {
              font-family: Arial, sans-serif;
              text-align: center;
              padding: 50px;
          }
          h1 {
              color: red;
          }
      </style>
  </head>
  <body>
      <h1>不支持WebUI</h1>
      <p>抱歉，xray/v2ray 不支持所需的WebUI功能。</p>
  </body>
  </html>' > $path_webroot
  fi
  log Info "已生成/更新 WebUI 页面: ${path_webroot} → http://${ip_port}/ui/ (内核: ${bin_name})"
}

bond0() {
  sysctl -w net.ipv4.tcp_low_latency=0 >/dev/null 2>&1
  log Debug "tcp 低延迟: 0"

  for dev in /sys/class/net/wlan*; do ip link set dev $(basename $dev) txqueuelen 3000; done
  log Debug "wlan* 传输队列长度: 3000"

  for txqueuelen in /sys/class/net/rmnet_data*; do txqueuelen_name=$(basename $txqueuelen); ip link set dev $txqueuelen_name txqueuelen 1000; done
  log Debug "rmnet_data* 传输队列长度: 1000"

  for mtu in /sys/class/net/rmnet_data*; do mtu_name=$(basename $mtu); ip link set dev $mtu_name mtu 1500; done
  log Debug "rmnet_data* MTU: 1500"
}

bond1() {
  sysctl -w net.ipv4.tcp_low_latency=1 >/dev/null 2>&1
  log Debug "tcp 低延迟: 1"

  for dev in /sys/class/net/wlan*; do ip link set dev $(basename $dev) txqueuelen 4000; done
  log Debug "wlan* 传输队列长度: 4000"

  for txqueuelen in /sys/class/net/rmnet_data*; do txqueuelen_name=$(basename $txqueuelen); ip link set dev $txqueuelen_name txqueuelen 2000; done
  log Debug "rmnet_data* 传输队列长度: 2000"

  for mtu in /sys/class/net/rmnet_data*; do mtu_name=$(basename $mtu); ip link set dev $mtu_name mtu 9000; done
  log Debug "rmnet_data* MTU: 9000"
}

case "$1" in
  check)
    check
    ;;
  memcg|blkio)
    case "$1" in
      memcg)
        memcg_path=""
        cgroup_memcg "${box_pid}" ${memcg_limit}
        ;;
      blkio)
        blkio_path=""
        cgroup_blkio "${box_pid}" "${weight}"
        ;;
    esac
    ;;
  bond0|bond1)
    $1
    ;;
  geosub)
    upsubs
    sub_status=$?
    [ "${sub_status}" -eq 1 ] && exit 1
    upgeox
    reload_if_running
    exit "${sub_status}"
    ;;
  geox|subs)
    sub_status=0
    if [ "$1" = "geox" ]; then
      upgeox
    else
      upsubs
      sub_status=$?
      [ "${sub_status}" -eq 1 ] && exit 1
    fi
    reload_if_running
    exit "${sub_status}"
    ;;
  status)
    if [ "$2" = "--json" ]; then
      status_json
    else
      "${scripts_dir}/box.service" status
    fi
    ;;
  logs)
    if [ "$2" = "clear" ]; then
      logs_clear_json
    elif [ "$2" = "--json" ]; then
      logs_json 200
    elif [ "$3" = "--json" ]; then
      logs_json "${2:-200}"
    else
      tail -n "${2:-200}" "${box_run}/runs.log" 2>/dev/null || true
    fi
    ;;
  settings)
    case "$2" in
      get)
        settings_get_json
        ;;
      apply)
        settings_apply_json "$3"
        ;;
      *)
        log Info "用法: $0 settings {get|apply <json文件>}"
        ;;
    esac
    ;;
  reset)
    case "$2" in
      subscriptions)
        reset_subscriptions_json
        ;;
      *)
        log Info "用法: $0 reset subscriptions"
        ;;
    esac
    ;;
  config)
    case "$2" in
      get)
        config_get_json
        ;;
      apply)
        config_apply_json "$3"
        ;;
      delete)
        config_delete_json
        ;;
      *)
        log Info "用法: $0 config {get|apply <json文件>|delete}"
        ;;
    esac
    ;;
  sub)
    case "$2" in
      add)
        sub_add "$3" "$4"
        ;;
      list|ls)
        if [ "$3" = "--json" ]; then
          sub_list_json
        else
          sub_list
        fi
        ;;
      rm|remove|del)
        sub_rm "$3"
        ;;
      update|up)
        sub_update
        ;;
      *)
        sub_usage
        ;;
    esac
    ;;
  upkernel)
    upkernel "$2"
    ;;  
  upkernels)
    shift
    upkernels "$@"
    ;;
  upgeox_all)
    upgeox_all
    ;;
  upxui)
    upxui
    ;;
  upcnip)
    upcnip
    ;;
  upyq|upcurl)
    $1
    ;;
  reload)
    reload
    ;;
  webroot)
    webroot
    ;;
  all)
    upyq
    upcurl
    upgeox_all
    upkernels sing-box mihomo xray v2fly hysteria
    for bin_name in "${bin_list[@]}"; do
      upsubs
      upxui
    done
    ;;
  *)
    log Error "$0 $1 未找到"
    log Info "用法: $0 {check|memcg|blkio|geosub|geox|subs|sub|upkernel [name]|upkernels [name...]|upgeox_all|upxui|upyq|upcurl|upcnip|reload|webroot|bond0|bond1|all}"
    log Info "upkernel 支持的核心: sing-box, mihomo, mihomo_smart, xray, v2fly, hysteria"
    ;;
esac
