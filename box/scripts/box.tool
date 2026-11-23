#!/system/bin/sh

scripts_dir="${0%/*}"

user_agent="box_for_root"
BOX_DIR="${BOX_HOME:-/data/adb/box}"
if [ ! -f "${BOX_DIR}/settings.base.sh" ]; then
  printf '{"ok":false,"error":{"code":"E_IO","message":"settings.base.sh 不存在","detail":""}}' >&2
  exit 1
fi
. "${BOX_DIR}/settings.base.sh"

for _arg in "$@"; do
  [ "${_arg}" = "--json" ] && export BOX_JSON_MODE=1
done

if ! settings_load; then
  printf '{"ok":false,"error":{"code":"E_VALIDATION","message":"设置加载失败","detail":"settings.json 或 settings.runtime.sh 不可用"}}' >&2
  exit 1
fi

# 使用 settings.base.sh 中提供的 log()
TOOL_LOG="${box_run}/tool.log"
busybox mkdir -p "$(dirname "$TOOL_LOG")"
box_log="$TOOL_LOG"

# 设置 GitHub API 访问配置
github_get() {
  if command -v curl >/dev/null 2>&1; then
    if [ -n "${githubtoken}" ]; then
      curl --insecure -sL -H "Authorization: token ${githubtoken}" "$@"
    else
      curl --insecure -sL "$@"
    fi
  else
    if [ -n "${githubtoken}" ]; then
      busybox wget --no-check-certificate -qO- --header="Authorization: token ${githubtoken}" "$@"
    else
      busybox wget --no-check-certificate -qO- "$@"
    fi
  fi
}

setup_github_api() {
  if [ -n "${githubtoken}" ]; then
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
_log_command="$*"
_log_command="$(printf '%s' "${_log_command}" | sed -E 's#https?://[^[:space:]]+#<url>#g')"
log Info "执行命令: $0 ${_log_command}"

# 以模块运行用户组执行下载：root:net_admin 会跳过 tproxy 的 owner 规则。
download_run_as_box_user() {
  if command -v getprop >/dev/null 2>&1 && busybox setuidgid "${box_user_group}" true >/dev/null 2>&1; then
    busybox setuidgid "${box_user_group}" "$@"
  else
    "$@"
  fi
}

# 解析真实 IPv4，绕过 mihomo fake-ip DNS；输出 host@port@ip。
download_resolve_target() {
  local url="$1"
  local authority host port ip dns
  case "${url}" in
    http://*|https://*) ;;
    *) return 1 ;;
  esac
  authority="${url#*://}"
  authority="${authority%%/*}"
  authority="${authority##*@}"
  [ -n "${authority}" ] || return 1
  host="${authority%%:*}"
  port=443
  case "${url}" in http://*) port=80 ;; esac
  case "${authority}" in *:*) port="${authority##*:}" ;; esac
  case "${host}" in
    ''|*[!A-Za-z0-9.-]*) return 1 ;;
  esac
  case "${host}" in
    *[!0-9.]*) ;;
    *) printf '%s@%s@%s\n' "${host}" "${port}" "${host}"; return 0 ;;
  esac
  for dns in 1.1.1.1 223.5.5.5; do
    ip="$(download_run_as_box_user busybox nslookup "${host}" "${dns}" 2>/dev/null | busybox awk '/^Name:/ { found = 1 } found && /^Address [0-9]+: / { print $3 }' | busybox grep -E '^[0-9]+(\.[0-9]+){3}$' | busybox head -n 1)"
    if [ -n "${ip}" ]; then
      printf '%s@%s@%s\n' "${host}" "${port}" "${ip}"
      return 0
    fi
  done
  return 1
}

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

  UPFILE_ERROR=""
  local curl_bin="" target host port ip resolve_args="" http_code curl_exit_code wget_exit_code curl_ok=false
  if [ -x "${bin_dir}/curl" ]; then
    curl_bin="${bin_dir}/curl"
  elif command -v curl >/dev/null 2>&1; then
    curl_bin="$(command -v curl)"
  fi

  if [ -n "${curl_bin}" ]; then
    target="$(download_resolve_target "${update_url}" 2>/dev/null || true)"
    if [ -n "${target}" ]; then
      host="${target%%@*}"
      target="${target#*@}"
      port="${target%%@*}"
      ip="${target#*@}"
      resolve_args="--resolve ${host}:${port}:${ip}"
    fi
    http_code="$(download_run_as_box_user "${curl_bin}" -L -s --insecure --http1.1 --compressed --connect-timeout 20 ${resolve_args} --user-agent "${current_ua}" -o "${file}" -w "%{http_code}" "${update_url}")"
    curl_exit_code=$?

    if [ "${curl_exit_code}" -eq 0 ] && [ "${http_code}" = "200" ] && [ -s "${file}" ]; then
      curl_ok=true
    else
      if [ "${curl_exit_code}" -ne 0 ]; then
        UPFILE_ERROR="curl=${curl_exit_code}"
        log Error "使用 curl 下载失败 (退出码: ${curl_exit_code})"
      elif [ "${http_code}" != "200" ]; then
        UPFILE_ERROR="http=${http_code:-unknown}"
        log Error "下载失败: 服务器返回 HTTP 状态码 ${http_code}"
      else
        UPFILE_ERROR="curl=empty"
        log Error "curl 下载文件为空"
      fi
      rm -f "${file}" 2>/dev/null
    fi
  fi

  if [ "${curl_ok}" != "true" ]; then
    download_run_as_box_user busybox wget --no-check-certificate -q -T 30 -U "${current_ua}" -O "${file}" "${update_url}"
    wget_exit_code=$?
    if [ "${wget_exit_code}" -ne 0 ] || [ ! -s "${file}" ]; then
      [ "${wget_exit_code}" -eq 0 ] && wget_exit_code=empty
      UPFILE_ERROR="${UPFILE_ERROR:+${UPFILE_ERROR}; }wget=${wget_exit_code}"
      log Error "使用 wget 下载失败"
      rm -f "${file}" 2>/dev/null
      [ -f "${file_bak}" ] && mv "${file_bak}" "${file}"
      return 1
    fi
  fi

  if [ ! -s "${file}" ]; then
    UPFILE_ERROR="empty"
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

  if [ "${cnip_mode}" = "ebpf" ]; then
    if [ "${updated_v4}" = "true" ] || [ "${updated_v6}" = "true" ]; then
      "${scripts_dir}/box.iptables" reload_ebpf >/dev/null 2>&1 || ok="false"
    fi
  else
    if [ "${updated_v4}" = "true" ]; then
      reload_cnip_set cnip inet "${cn_ip_file}" || ok="false"
    fi

    if [ "${updated_v6}" = "true" ]; then
      reload_cnip_set cnip6 inet6 "${cn_ipv6_file}" || ok="false"
    fi
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

  local latest_version=$(github_get "https://api.github.com/repos/stunnel/static-curl/releases" | grep "tag_name" | busybox grep -oE "[0-9.]*" | head -1)
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
  case "${bin_name}" in
    mihomo|sing-box) ;;
    *)
      log Error "不支持的代理核心: ${bin_name}"
      return 1
      ;;
  esac
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
  for core in mihomo sing-box; do
      bin_name=$core
      upgeox
  done
  bin_name=$original_bin_name
}

# 订阅清单：每行 名字|URL
subscriptions_file="${subscriptions_file:-${box_dir}/mihomo/subscriptions.list}"
# 聚合订阅派生出的 provider 清单（内部运行文件）
subscriptions_providers_file="${subscriptions_providers_file:-${box_run}/subscriptions.providers}"

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

sub_manifest_source_exists() {
  local source="$1"
  [ -s "${subscriptions_providers_file}" ] || return 1
  busybox awk -F'|' -v source="${source}" '$1 == source { found = 1 } END { exit(found ? 0 : 1) }' "${subscriptions_providers_file}"
}

sub_provider_count() {
  local source="$1"
  [ -s "${subscriptions_providers_file}" ] || { printf '0'; return 0; }
  busybox awk -F'|' -v source="${source}" '$1 == source && $2 != "" { count++ } END { print count + 0 }' "${subscriptions_providers_file}"
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

      if sub_manifest_source_exists "${provider_name}"; then
        log Debug "跳过聚合订阅自身: ${provider_name}"
        continue
      fi

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

  if [ -s "${subscriptions_providers_file}" ]; then
    while IFS='|' read -r aggregate_source provider_name provider_url local_path; do
      case "${provider_name}" in
        ''|\#*) continue ;;
      esac
      [ -n "${provider_url}" ] || continue

      local safe_name relative_path escaped_url
      safe_name="$(sub_provider_file_name "${provider_name}")"
      relative_path="./$(basename "${mihomo_provide_path}")/${safe_name}.yaml"
      escaped_url="$(printf '%s' "${provider_url}" | busybox sed 's/\\/\\\\/g; s/"/\\"/g')"

      log Debug "添加派生 provider: ${provider_name} -> ${relative_path} (http)"
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
    done < "${subscriptions_providers_file}"
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


# 判断下载内容是否为完整配置聚合：proxies 为空且 proxy-providers 非空
sub_is_aggregate_file() {
  local file="$1"
  local count

  if ${yq} 'has("proxies")' "${file}" >/dev/null 2>&1; then
    count="$(${yq} '.proxies | length' "${file}" 2>/dev/null || printf '0')"
    case "${count}" in ''|*[!0-9]*) count=0 ;; esac
    [ "${count}" -eq 0 ] || return 1
  fi

  ${yq} 'has("proxy-providers")' "${file}" >/dev/null 2>&1 || return 1
  count="$(${yq} '.proxy-providers | length' "${file}" 2>/dev/null || printf '0')"
  case "${count}" in ''|*[!0-9]*) count=0 ;; esac
  [ "${count}" -gt 0 ]
}

# 从普通 provider / base64 订阅中提取 proxies，成功时输出节点数
sub_parse_provider_download() {
  local temp_file="$1"
  local provider_name="$2"
  local provider_file="$3"
  local proxy_count=0
  local temp_proxies="${temp_file}.proxies"

  if ${yq} 'has("proxies")' "${temp_file}" >/dev/null 2>&1; then
    if ${yq} '.proxies' "${temp_file}" > "${temp_proxies}" 2>/dev/null && [ -s "${temp_proxies}" ]; then
      proxy_count="$(${yq} 'length' "${temp_proxies}" 2>/dev/null || printf '0')"
      case "${proxy_count}" in ''|*[!0-9]*) proxy_count=0 ;; esac
      if [ "${proxy_count}" -gt 0 ]; then
        ${yq} -i '{"proxies": .}' "${temp_proxies}" || {
          rm -f "${temp_proxies}" "${temp_file}"
          return 1
        }
        mv "${temp_proxies}" "${temp_file}" || {
          rm -f "${temp_proxies}" "${temp_file}"
          return 1
        }
        mv "${temp_file}" "${provider_file}" || {
          rm -f "${temp_file}"
          return 1
        }
        printf '%s' "${proxy_count}"
        return 0
      fi
    fi
    rm -f "${temp_proxies}" "${temp_file}"
    return 1
  fi

  if [ -n "$(base64 -d "${temp_file}" 2>/dev/null | busybox grep -E "vless://|vmess://|ss://|hysteria://|hysteria2://|anytls://|trojan://" | head -n 1)" ]; then
    local decoded_file="${temp_file}.decoded"
    base64 -d "${temp_file}" > "${decoded_file}" 2>/dev/null || {
      rm -f "${decoded_file}" "${temp_file}"
      return 1
    }
    proxy_count="$(busybox grep -cE "vless://|vmess://|ss://|hysteria://|hysteria2://|anytls://|trojan://" "${decoded_file}" 2>/dev/null || true)"
    [ -n "${proxy_count}" ] || proxy_count=0
    mv "${decoded_file}" "${temp_file}" || {
      rm -f "${decoded_file}" "${temp_file}"
      return 1
    }
    mv "${temp_file}" "${provider_file}" || {
      rm -f "${temp_file}"
      return 1
    }
    printf '%s' "${proxy_count}"
    return 0
  fi

  rm -f "${temp_proxies}" "${temp_file}"
  return 1
}

# provider 名称是否与现有订阅或派生 provider 冲突
sub_provider_name_conflicts() {
  local source="$1"
  local name="$2"
  local safe_name="$3"
  local existing_name existing_url
  local manifest_source manifest_name manifest_url manifest_path

  while IFS='|' read -r existing_name existing_url; do
    case "${existing_name}" in
      ''|\#*) continue ;;
    esac
    [ "${existing_name}" = "${name}" ] && return 0
    [ "$(sub_provider_file_name "${existing_name}")" = "${safe_name}" ] && return 0
  done < "${subscriptions_file}"

  if [ -s "${subscriptions_providers_file}" ]; then
    while IFS='|' read -r manifest_source manifest_name manifest_url manifest_path; do
      [ "${manifest_source}" = "${source}" ] && continue
      [ "${manifest_name}" = "${name}" ] && return 0
      [ "$(sub_provider_file_name "${manifest_name}")" = "${safe_name}" ] && return 0
    done < "${subscriptions_providers_file}"
  fi

  return 1
}

# 写派生 provider 清单；失败的 provider 保留旧条目，成功 provider 覆盖旧条目
sub_write_provider_manifest() {
  local source="$1"
  local provider_list="$2"
  local success_lines="$3"
  local names_file="${provider_list}.names"
  local success_names="${success_lines}.names"
  local temp_manifest="${subscriptions_providers_file}.tmp.$$"

  busybox cut -f1 "${provider_list}" > "${names_file}" || return 1
  busybox cut -d'|' -f2 "${success_lines}" > "${success_names}" || return 1
  : > "${temp_manifest}" || return 1

  if [ -s "${subscriptions_providers_file}" ]; then
    busybox awk -F'|' -v source="${source}" -v names="${names_file}" -v success="${success_names}" '
      BEGIN {
        while ((getline line < names) > 0) current[line] = 1
        while ((getline line < success) > 0) updated[line] = 1
      }
      {
        if ($1 != source) {
          print
          next
        }
        if (updated[$2]) next
        print
      }
    ' "${subscriptions_providers_file}" > "${temp_manifest}" || {
      rm -f "${names_file}" "${success_names}" "${temp_manifest}"
      return 1
    }
  fi

  [ -s "${success_lines}" ] && cat "${success_lines}" >> "${temp_manifest}"
  mv "${temp_manifest}" "${subscriptions_providers_file}" || {
    rm -f "${names_file}" "${success_names}" "${temp_manifest}"
    return 1
  }
  chmod 0600 "${subscriptions_providers_file}" >/dev/null 2>&1 || true
  rm -f "${names_file}" "${success_names}"
  return 0
}

# 更新完整配置聚合中的 provider；0=全部成功，2=部分成功，1=全部失败/格式错误
sub_update_aggregate() {
  local source="$1"
  local temp_file="$2"
  local provider_list="${temp_file}.providers"
  local success_lines="${temp_file}.success"
  local p_name p_url provider_file provider_count success_count failure_count=0 duplicate=""

  ${yq} -r '.proxy-providers | to_entries[] | [.key, (.value.url // "")] | @tsv' "${temp_file}" > "${provider_list}" 2>/dev/null || {
    log Error "聚合订阅 provider 列表解析失败: ${source}"
    rm -f "${provider_list}"
    return 1
  }
  if [ ! -s "${provider_list}" ]; then
    log Error "聚合订阅没有可用的 proxy-providers: ${source}"
    rm -f "${provider_list}"
    return 1
  fi

  duplicate="$(busybox cut -f1 "${provider_list}" | busybox sort | busybox uniq -d | head -n 1)"
  if [ -n "${duplicate}" ]; then
    log Error "聚合订阅 provider 名称重复: ${duplicate}"
    rm -f "${provider_list}"
    return 1
  fi

  # 先做全量校验，冲突或非法项不产生任何文件修改
  while IFS="$(printf '\t')" read -r p_name p_url; do
    case "${p_name}" in
      ''|*'|'*) log Error "聚合订阅 provider 名称非法: ${p_name}"; rm -f "${provider_list}"; return 1 ;;
    esac
    if printf '%s' "${p_name}" | busybox grep -q '[[:cntrl:]]'; then
      log Error "聚合订阅 provider 名称包含控制字符: ${p_name}"
      rm -f "${provider_list}"
      return 1
    fi
    case "${p_url}" in
      *'|'*) log Error "聚合订阅 provider URL 包含非法字符: ${p_name}"; rm -f "${provider_list}"; return 1 ;;
      http://*|https://*) ;;
      *) log Error "聚合订阅 provider URL 非法: ${p_name}"; rm -f "${provider_list}"; return 1 ;;
    esac
    if sub_provider_name_conflicts "${source}" "${p_name}" "$(sub_provider_file_name "${p_name}")"; then
      log Error "聚合订阅 provider 名称冲突: ${p_name}"
      rm -f "${provider_list}"
      return 1
    fi
  done < "${provider_list}"

  : > "${success_lines}" || {
    rm -f "${provider_list}"
    return 1
  }

  while IFS="$(printf '\t')" read -r p_name p_url; do
    provider_file="${mihomo_provide_path}/$(sub_provider_file_name "${p_name}").yaml"
    rm -f "${provider_file}.download"
    if ! LOG_MASK_URL=mask upfile "${provider_file}.download" "${p_url}" "ClashMeta"; then
      log Error "provider 下载失败: ${p_name}"
      rm -f "${provider_file}.download"
      failure_count=$((failure_count + 1))
      continue
    fi
    if ! provider_count="$(sub_parse_provider_download "${provider_file}.download" "${p_name}" "${provider_file}")"; then
      log Error "provider 解析失败或 proxies 为空: ${p_name}"
      rm -f "${provider_file}.download"
      failure_count=$((failure_count + 1))
      continue
    fi
    printf '%s|%s|%s|%s\n' "${source}" "${p_name}" "${p_url}" "${provider_file}" >> "${success_lines}"
    success_count=$((success_count + 1))
    log Info "provider ${p_name} 已更新（${provider_count} 个节点）"
  done < "${provider_list}"

  if [ "${success_count:-0}" -eq 0 ]; then
    log Error "聚合订阅全部 provider 失败: ${source}"
    rm -f "${provider_list}" "${success_lines}"
    return 1
  fi
  if ! sub_write_provider_manifest "${source}" "${provider_list}" "${success_lines}"; then
    log Error "派生 provider 清单写入失败: ${source}"
    rm -f "${provider_list}" "${success_lines}"
    return 1
  fi
  rm -f "${provider_list}" "${success_lines}"
  if [ "${failure_count}" -gt 0 ]; then
    return 2
  fi
  return 0
}

# 检查并更新订阅
# 参数: [force] true 时忽略 update_subscription 开关
upsubs() {
  local force="${1:-false}"
  local only_name="${2:-}"

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
      local subscription_mode="${subscription_mode:-auto}"

      while IFS='|' read -r provider_name provider_url; do
        case "${provider_name}" in
          ''|\#*) continue ;;
        esac

        if [ -z "${provider_url}" ]; then
          log Warning "订阅地址为空，跳过: ${provider_name}"
          failure_count=$((failure_count + 1))
          continue
        fi

        if [ -n "${only_name}" ] && [ "${provider_name}" != "${only_name}" ]; then
          continue
        fi

        total_count=$((total_count + 1))
        local safe_name
        safe_name="$(sub_provider_file_name "${provider_name}")"
        local provider_file="${mihomo_provide_path}/${safe_name}.yaml"
        local temp_file="${provider_file}.download"

        log Info "--> 正在处理订阅: ${provider_name}"
        rm -f "${temp_file}"

        if ! LOG_MASK_URL=mask upfile "${temp_file}" "${provider_url}" "ClashMeta"; then
          log Error "订阅下载失败: ${provider_name}"
          rm -f "${temp_file}"
          if [ "${subscription_keep_last_good}" != "true" ]; then
            rm -f "${provider_file}"
          fi
          failure_count=$((failure_count + 1))
          continue
        fi

        if sub_is_aggregate_file "${temp_file}"; then
          if [ "${subscription_mode}" = "provider" ]; then
            log Error "订阅 ${provider_name} 是完整配置聚合，请将 subscription_mode 设为 auto"
            rm -f "${temp_file}"
            failure_count=$((failure_count + 1))
            continue
          fi

          log Info "检测到完整配置聚合，开始拆分 provider: ${provider_name}"
          sub_update_aggregate "${provider_name}" "${temp_file}"
          local aggregate_status=$?
          rm -f "${temp_file}"
          case "${aggregate_status}" in
            0)
              success_count=$((success_count + 1))
              ;;
            2)
              success_count=$((success_count + 1))
              failure_count=$((failure_count + 1))
              ;;
            *)
              failure_count=$((failure_count + 1))
              ;;
          esac
          continue
        fi

        local proxy_count=0
        if ! proxy_count="$(sub_parse_provider_download "${temp_file}" "${provider_name}" "${provider_file}")"; then
          log Error "订阅格式无法识别或 proxies 为空: ${provider_name}"
          rm -f "${temp_file}"
          if [ "${subscription_keep_last_good}" != "true" ]; then
            rm -f "${provider_file}"
          fi
          failure_count=$((failure_count + 1))
          continue
        fi

        rm -f "${temp_file}"
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
        log Error "${failure_count} 个订阅/provider 更新失败"
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
  box.tool sub get <名字>
  box.tool sub set <旧名字> <新名字> <订阅地址>
  box.tool sub list
  box.tool sub rm <名字>
  box.tool sub update [名字]
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
  case "${url}" in
    *"|"*)
      log Error "订阅地址不能包含 |"
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

    local safe_name provider_file state provider_count
    safe_name="$(sub_provider_file_name "${name}")"
    provider_file="${mihomo_provide_path}/${safe_name}.yaml"
    provider_count="$(sub_provider_count "${name}")"

    if [ "${provider_count}" -gt 0 ]; then
      state="${provider_count} 个 provider"
    elif [ -s "${provider_file}" ]; then
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
  local only_name="${1:-}"
  upsubs true "${only_name}"
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
  escaped="$(printf '%s' "$1" | busybox sed 's/\\/\\\\/g; s/"/\\"/g; s/\t/\\t/g; s/\r//g' | busybox awk 'BEGIN { ORS = "" } { if (NR > 1) printf "\\n"; printf "%s", $0 }')"
  printf '"%s"' "${escaped}"
}

json_bool() {
  if [ "$1" = "true" ]; then printf 'true'; else printf 'false'; fi
}

json_ok() {
  printf '{"ok":true,"data":%s,"meta":{"version":1}}\n' "${1:-null}"
}

has_json_arg() {
  local arg
  for arg in "$@"; do
    [ "${arg}" = "--json" ] && return 0
  done
  return 1
}

json_error() {
  local code="$1"
  local message="$2"
  local detail="${3:-}"
  printf '{"ok":false,"error":{"code":%s,"message":%s,"detail":%s},"meta":{"version":1}}\n' \
    "$(json_quote "${code}")" "$(json_quote "${message}")" "$(json_quote "${detail}")"
}

sh_single_quote() {
  local escaped
  escaped="$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
  printf "'%s'" "${escaped}"
}

mask_secret() {
  local value="$1"
  [ -n "${value}" ] || return 0
  if [ "${#value}" -le 8 ]; then
    printf '********'
    return 0
  fi
  printf '%s' "${value}" | busybox awk '{ print substr($0, 1, 4) "****" substr($0, length($0) - 3) }'
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
  case "${bin_name}" in
    mihomo|sing-box) ;;
    *)
      json_error "E_VALIDATION" "不支持的代理核心: ${bin_name}"
      return 1
      ;;
  esac

  local running=false
  local pid="null"
  local config_exists=false
  local config_file="${mihomo_config}"
  local config_name="${name_mihomo_config}"
  if [ "${bin_name}" = "sing-box" ]; then
    config_file="${sing_config}"
    config_name="${name_sing_config}"
  fi
  if [ -f "${box_pid}" ]; then
    local current_pid
    current_pid="$(cat "${box_pid}" 2>/dev/null)"
    if [ -n "${current_pid}" ] && kill -0 "${current_pid}" 2>/dev/null; then
      running=true
      pid="${current_pid}"
    fi
  fi
  [ -f "${config_file}" ] && config_exists=true

  json_ok "$(printf '{"running":%s,"pid":%s,"core":%s,"mode":%s,"config":%s,"config_exists":%s,"subscriptions":%s,"active_tasks":%s}' \
    "$(json_bool "${running}")" \
    "${pid}" \
    "$(json_quote "${bin_name}")" \
    "$(json_quote "${network_mode}")" \
    "$(json_quote "${config_name}")" \
    "$(json_bool "${config_exists}")" \
    "$(subscription_count)" \
    "$(task_running_count)")"
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

  printf '{"ok":true,"data":{"file":%s,"lines":[' "$(json_quote "${file}")"
  if [ -f "${file}" ]; then
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
      END { print "]" }
    '
  else
    printf '] '
  fi
  printf '},"meta":{"version":1}}\n'
}

sub_list_json() {
  printf '{"ok":true,"data":{"items":['
  local first=1
  if [ -s "${subscriptions_file}" ]; then
    while IFS='|' read -r name url; do
      case "${name}" in
        ''|\#*) continue ;;
      esac
      local safe_name provider_file state provider_size updated provider_count
      safe_name="$(sub_provider_file_name "${name}")"
      provider_file="${mihomo_provide_path}/${safe_name}.yaml"
      provider_count="$(sub_provider_count "${name}")"
      if [ "${provider_count}" -gt 0 ]; then
        state="aggregated"
      elif [ -s "${provider_file}" ]; then
        state="downloaded"
      else
        state="missing"
      fi
      provider_size=0
      updated=""
      if [ -f "${provider_file}" ]; then
        provider_size="$(busybox wc -c < "${provider_file}" 2>/dev/null | busybox tr -d '[:space:]')"
        [ -n "${provider_size}" ] || provider_size=0
        updated="$(busybox stat -c '%y' "${provider_file}" 2>/dev/null | busybox cut -d. -f1 || true)"
      fi
      [ "${first}" -eq 1 ] || printf ','
      printf '{"name":%s,"url_masked":%s,"provider":%s,"provider_size":%s,"provider_count":%s,"updated_at":%s}' \
        "$(json_quote "${name}")" "$(json_quote "$(mask_url "${url}")")" "$(json_quote "${state}")" \
        "${provider_size}" "${provider_count}" "$(json_quote "${updated}")"
      first=0
    done < "${subscriptions_file}"
  fi
  printf ']},"meta":{"version":1}}\n'
}

sub_get_json() {
  local wanted="$1"
  if [ -z "${wanted}" ]; then
    json_error "E_USAGE" "缺少订阅名字"
    return 1
  fi
  if ! subscription_exists "${wanted}"; then
    json_error "E_NOT_FOUND" "订阅不存在: ${wanted}"
    return 1
  fi

  local name url found=1 safe_name provider_file provider_size provider_count updated
  while IFS='|' read -r name url; do
    [ "${name}" = "${wanted}" ] || continue
    found=0
    safe_name="$(sub_provider_file_name "${name}")"
    provider_file="${mihomo_provide_path}/${safe_name}.yaml"
    provider_count="$(sub_provider_count "${name}")"
    provider_size=0
    updated=""
    if [ -f "${provider_file}" ]; then
      provider_size="$(busybox wc -c < "${provider_file}" 2>/dev/null | busybox tr -d '[:space:]')"
      [ -n "${provider_size}" ] || provider_size=0
      updated="$(busybox stat -c '%y' "${provider_file}" 2>/dev/null | busybox cut -d. -f1 || true)"
    fi
    json_ok "$(printf '{"name":%s,"url":%s,"url_masked":%s,"provider":%s,"provider_size":%s,"provider_count":%s,"updated_at":%s}' \
      "$(json_quote "${name}")" "$(json_quote "${url}")" "$(json_quote "$(mask_url "${url}")")" \
      "$(json_quote "$([ "${provider_count}" -gt 0 ] && printf aggregated || { [ -s "${provider_file}" ] && printf downloaded || printf missing; })")" \
      "${provider_size}" "${provider_count}" "$(json_quote "${updated}")")"
    return 0
  done < "${subscriptions_file}"

  if [ "${found}" -eq 1 ]; then
    json_error "E_NOT_FOUND" "订阅不存在: ${wanted}"
    return 1
  fi
}

sub_add_json() {
  local url="$1"
  local name="$2"
  if [ -z "${url}" ]; then
    json_error "E_USAGE" "缺少订阅地址"
    return 1
  fi
  if [ -z "${name}" ]; then
    name="$(printf '%s' "${url}" | busybox sed -E 's#^https?://([^/]+).*#\1#')"
  fi

  local output rc
  output="$(sub_add "${url}" "${name}" 2>&1 1>/dev/null)"
  rc=$?
  if [ "${rc}" -ne 0 ]; then
    json_error "E_VALIDATION" "添加订阅失败" "$(printf '%s' "${output}" | busybox sed -E 's/^[0-9-]+ [0-9:]+ \[[A-Za-z]+\]: //')"
    return 1
  fi
  json_ok "$(printf '{"name":%s,"url_masked":%s,"provider":"missing","provider_size":0,"provider_count":0,"updated_at":""}' \
    "$(json_quote "${name}")" "$(json_quote "$(mask_url "${url}")")")"
}

sub_rm_json() {
  local name="$1"
  if [ -z "${name}" ]; then
    json_error "E_USAGE" "缺少订阅名字"
    return 1
  fi
  if ! subscription_exists "${name}"; then
    json_error "E_NOT_FOUND" "订阅不存在: ${name}"
    return 1
  fi

  local output rc
  output="$(sub_rm "${name}" 2>&1 1>/dev/null)"
  rc=$?
  if [ "${rc}" -ne 0 ]; then
    json_error "E_IO" "删除订阅失败" "$(printf '%s' "${output}" | busybox sed -E 's/^[0-9-]+ [0-9:]+ \[[A-Za-z]+\]: //')"
    return 1
  fi
  json_ok "$(printf '{"name":%s}' "$(json_quote "${name}")")"
}

sub_set_json() {
  local old_name="$1"
  local new_name="$2"
  local url="$3"
  if [ -z "${old_name}" ] || [ -z "${url}" ]; then
    json_error "E_USAGE" "用法: sub set <旧名字> <新名字> <订阅地址> --json"
    return 1
  fi
  [ -n "${new_name}" ] || new_name="${old_name}"
  case "${url}" in
    http://*|https://*) ;;
    *)
      json_error "E_VALIDATION" "订阅地址必须以 http:// 或 https:// 开头"
      return 1
      ;;
  esac
  case "${url}" in
    *"|"*)
      json_error "E_VALIDATION" "订阅地址不能包含 |"
      return 1
      ;;
  esac
  case "${new_name}" in
    *"|"*)
      json_error "E_VALIDATION" "名字不能包含 |"
      return 1
      ;;
  esac
  if ! subscription_exists "${old_name}"; then
    json_error "E_NOT_FOUND" "订阅不存在: ${old_name}"
    return 1
  fi
  if [ "${new_name}" != "${old_name}" ] && subscription_exists "${new_name}"; then
    json_error "E_CONFLICT" "订阅已存在: ${new_name}"
    return 1
  fi

  local old_safe new_safe temp_file provider_file renamed=false
  old_safe="$(sub_provider_file_name "${old_name}")"
  new_safe="$(sub_provider_file_name "${new_name}")"
  if [ "${new_safe}" != "${old_safe}" ] && [ -f "${mihomo_provide_path}/${new_safe}.yaml" ]; then
    json_error "E_CONFLICT" "订阅名生成的节点文件名冲突"
    return 1
  fi

  temp_file="${subscriptions_file}.tmp.$$"
  if ! busybox awk -F'|' -v old="${old_name}" -v new="${new_name}" -v url="${url}" '
    BEGIN { OFS = "|"; found = 0 }
    $1 == old { $1 = new; $2 = url; found = 1 }
    { print }
    END { exit(found ? 0 : 1) }
  ' "${subscriptions_file}" > "${temp_file}"; then
    rm -f "${temp_file}"
    json_error "E_IO" "更新订阅失败"
    return 1
  fi
  mv "${temp_file}" "${subscriptions_file}" || {
    rm -f "${temp_file}"
    json_error "E_IO" "写入订阅清单失败"
    return 1
  }

  if [ "${old_safe}" != "${new_safe}" ]; then
    provider_file="${mihomo_provide_path}/${old_safe}.yaml"
    if [ -f "${provider_file}" ]; then
      mv "${provider_file}" "${mihomo_provide_path}/${new_safe}.yaml"
      renamed=true
    fi
    rm -f "${mihomo_provide_path}/${old_safe}.yaml.download"
  fi

  if [ "${auto_modify_config}" = "true" ] && [ -x "${box_dir}/bin/yq" -o "$(command -v yq 2>/dev/null)" ]; then
    update_mihomo_providers >/dev/null 2>&1 || log Warning "订阅已更新，但 proxy-providers 配置更新失败"
  fi

  json_ok "$(printf '{"name":%s,"url_masked":%s,"renamed":%s}' \
    "$(json_quote "${new_name}")" "$(json_quote "$(mask_url "${url}")")" "$(json_bool "${renamed}")")"
}

sub_update_json() {
  local name="${1:-}"
  if [ -n "${name}" ] && ! subscription_exists "${name}"; then
    json_error "E_NOT_FOUND" "订阅不存在: ${name}"
    return 1
  fi

  sub_update "${name}"
  local rc=$?
  if [ "${rc}" -eq 1 ]; then
    json_error "E_COMMAND" "订阅更新失败" "请查看任务日志或 runs.log"
    return 1
  fi
  if [ "${rc}" -eq 2 ]; then
    json_ok "$(printf '{"updated":true,"partial":true,"failed":true}')"
    return 0
  fi
  json_ok "$(printf '{"updated":true,"partial":false,"failed":false}')"
}

settings_validate_scalar() {
  local key="$1"
  local value="$2"
  if printf '%s' "${value}" | busybox grep -q '[[:cntrl:]]'; then
    return 1
  fi
  case "${key}" in
    auto_modify_config|boot_auto_start|ipv6|performance_mode|proxy_tcp|proxy_udp|dns_hijack_tcp|dns_hijack_udp|bypass_cn_ip|bypass_cn_ip_v4|bypass_cn_ip_v6|update_subscription|update_geo|use_ghproxy|run_crontab|cgroup_memcg|taskset_cpu|cgroup_blkio|mac_filter|subscription_keep_last_good|enable_network_service_control|use_module_on_wifi|use_module_on_wifi_disconnect|use_ssid_matching|inotify_log_enabled)
      case "${value}" in true|false) return 0 ;; *) return 1 ;; esac
      ;;
    quic|mihomo_dns_forward|mihomo_stable|singbox_stable)
      case "${value}" in enable|disable) return 0 ;; *) return 1 ;; esac
      ;;
    network_mode)
      case "${value}" in tun|tproxy|ebpf|redirect|mixed|enhance) return 0 ;; *) return 1 ;; esac
      ;;
    subscription_mode)
      case "${value}" in auto|provider) return 0 ;; *) return 1 ;; esac
      ;;
    proxy_mode)
      case "${value}" in core|blacklist|whitelist) return 0 ;; *) return 1 ;; esac
      ;;
    dns_hijack_mode)
      case "${value}" in disable|tproxy|redirect) return 0 ;; *) return 1 ;; esac
      ;;
    cnip_mode)
      case "${value}" in ipset|ebpf) return 0 ;; *) return 1 ;; esac
      ;;
    mac_mode|use_wifi_list_mode)
      case "${value}" in blacklist|whitelist) return 0 ;; *) return 1 ;; esac
      ;;
    bin_name)
      case "${value}" in mihomo|sing-box) return 0 ;; *) return 1 ;; esac
      ;;
    tproxy_port|redir_port)
      case "${value}" in ''|*[!0-9]*) return 1 ;; esac
      [ "${value}" -ge 1 ] 2>/dev/null && [ "${value}" -le 65535 ] 2>/dev/null || return 1
      return 0
      ;;
    memcg_limit)
      printf '%s' "${value}" | busybox grep -Eq '^[0-9]+[KMGkmg]?$' || return 1
      return 0
      ;;
    allow_cpu)
      printf '%s' "${value}" | busybox grep -Eq '^[0-9]+(-[0-9]+)?(,[0-9]+(-[0-9]+)?)*$' || return 1
      return 0
      ;;
    weight)
      [ -z "${value}" ] && return 0
      case "${value}" in *[!0-9]*) return 1 ;; esac
      [ "${value}" -ge 1 ] 2>/dev/null && [ "${value}" -le 1000 ] 2>/dev/null || return 1
      return 0
      ;;
    interva_update)
      [ "$(printf '%s' "${value}" | busybox awk '{ print NF }')" -eq 5 ] || return 1
      return 0
      ;;
    url_ghproxy|ruleset_prefix|cn_ip_url|cn_ipv6_url|subscription_url_singbox)
      [ -z "${value}" ] && return 0
      case "${value}" in http://*|https://*) return 0 ;; *) return 1 ;; esac
      ;;
    githubtoken)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}


settings_json_data() {
  local default_file="${settings_default_file}"
  local allowed

  [ -f "${settings_json_file}" ] || return 1
  [ -f "${default_file}" ] || default_file="${settings_json_file}"
  allowed=" $(settings_allowed_keys | busybox tr '\n' ' ') "
  busybox awk -v mode=json -v allowed="${allowed}" -f "${box_dir}/scripts/settings-gen.awk" \
    "${default_file}" "${settings_json_file}" 2>/dev/null
}

settings_token_value() {
  if [ -f "${settings_token_file}" ] && [ -s "${settings_token_file}" ]; then
    cat "${settings_token_file}"
  fi
}

settings_get_json() {
  local settings_json token token_set=false token_preview=""

  if ! settings_json="$(settings_json_data)"; then
    json_error "E_VALIDATION" "settings.json 格式或字段不合法" "请通过设置页修复"
    return 1
  fi
  token="$(settings_token_value)"
  if [ -n "${token}" ]; then
    token_set=true
    token_preview="$(mask_secret "${token}")"
  fi
  printf '{"ok":true,"data":%s,"sensitive":{"githubtoken":{"set":%s,"preview":%s}}},"meta":{"version":1}}\n' \
    "${settings_json%?}" "$(json_bool "${token_set}")" "$(json_quote "${token_preview}")"
}

settings_validate_array() {
  local key="$1"
  local item="$2"
  case "${key}" in
    cnip_force_proxy_apps)
      printf '%s' "${item}" | busybox grep -Eq '^([0-9]+:)?[A-Za-z0-9._]+$' || return 1
      ;;
    force_proxy_cidrs)
      printf '%s' "${item}" | busybox grep -Eq '^([0-9]{1,3}\.){3}[0-9]{1,3}/[0-9]{1,2}$' || return 1
      ;;
    force_proxy_cidrs6)
      case "${item}" in *:*) ;; *) return 1 ;; esac
      ;;
    macs_list|wifi_bssids_list)
      printf '%s' "${item}" | busybox grep -Eiq '^([0-9a-f]{2}:){5}[0-9a-f]{2}$' || return 1
      ;;
  esac
}

settings_apply_json() {
  local file="$1"
  local default_file="${settings_default_file}"
  local allowed tmp_json tmp_runtime tmp_assign tmp_token token_value
  local key value item changed="" first=1

  if [ ! -s "${file}" ]; then
    json_error "E_USAGE" "settings apply: 缺少 JSON 文件"
    return 1
  fi
  [ -f "${settings_json_file}" ] || {
    json_error "E_IO" "settings.json 不存在"
    return 1
  }
  [ -f "${default_file}" ] || default_file="${settings_json_file}"
  allowed=" $(settings_allowed_keys | busybox tr '\n' ' ') "

  tmp_json="${settings_json_file}.tmp.$$"
  tmp_runtime="${settings_runtime_file}.tmp.$$"
  tmp_assign="${box_run}/settings.assign.$$"

  if ! busybox awk -v mode=json -v allowed="${allowed}" -f "${box_dir}/scripts/settings-gen.awk" \
      "${default_file}" "${file}" > "${tmp_json}" 2>/dev/null; then
    rm -f "${tmp_json}" "${tmp_assign}"
    json_error "E_VALIDATION" "设置 JSON 无法解析或包含未知键"
    return 1
  fi
  if ! busybox awk -v allowed="${allowed}" -f "${box_dir}/scripts/settings-gen.awk" \
      "${default_file}" "${file}" > "${tmp_assign}" 2>/dev/null; then
    rm -f "${tmp_json}" "${tmp_assign}"
    json_error "E_VALIDATION" "设置 JSON 无法解析"
    return 1
  fi

  settings_token_present=0
  unset settings_input_token
  . "${tmp_assign}" || {
    rm -f "${tmp_json}" "${tmp_assign}"
    json_error "E_VALIDATION" "设置 JSON 无法加载"
    return 1
  }

  for key in $(settings_scalar_keys); do
    eval "value=\${${key}}"
    if ! settings_validate_scalar "${key}" "${value}"; then
      rm -f "${tmp_json}" "${tmp_assign}"
      json_error "E_VALIDATION" "设置 ${key} 的值不合法" "${value}"
      return 1
    fi
  done
  for key in $(settings_array_keys); do
    eval "items=(\"\${${key}[@]}\")"
    for item in "${items[@]}"; do
      if ! settings_validate_array "${key}" "${item}"; then
        rm -f "${tmp_json}" "${tmp_assign}"
        json_error "E_VALIDATION" "设置 ${key} 包含非法数组项" "${item}"
        return 1
      fi
    done
  done

  if [ "${settings_token_present:-0}" = "1" ]; then
    token_value="${settings_input_token}"
    if printf '%s' "${token_value}" | busybox grep -q '[[:cntrl:]]'; then
      rm -f "${tmp_json}" "${tmp_assign}"
      json_error "E_VALIDATION" "githubtoken 包含非法控制字符"
      return 1
    fi
    tmp_token="${settings_token_file}.tmp.$$"
    printf '%s' "${token_value}" > "${tmp_token}" || {
      rm -f "${tmp_json}" "${tmp_assign}" "${tmp_token}"
      json_error "E_IO" "无法写入 githubtoken"
      return 1
    }
  else
    tmp_token="${settings_token_file}"
  fi

  if ! settings_render_runtime_file "${tmp_json}" "${tmp_runtime}" "${tmp_token}" "${default_file}"; then
    rm -f "${tmp_json}" "${tmp_assign}" "${tmp_runtime}" "${settings_token_file}.tmp.$$"
    json_error "E_VALIDATION" "无法生成 settings.runtime.sh"
    return 1
  fi

  # Two-phase replacement: all temporary artifacts are ready before any target is replaced.
  mv -f "${tmp_json}" "${settings_json_file}" || {
    rm -f "${tmp_assign}" "${tmp_runtime}" "${settings_token_file}.tmp.$$"
    json_error "E_IO" "无法替换 settings.json"
    return 1
  }
  chmod 0600 "${settings_json_file}" >/dev/null 2>&1 || true
  if [ "${settings_token_present:-0}" = "1" ]; then
    mv -f "${tmp_token}" "${settings_token_file}" || {
      rm -f "${tmp_assign}" "${tmp_runtime}"
      json_error "E_IO" "无法替换 githubtoken"
      return 1
    }
    chmod 0600 "${settings_token_file}" >/dev/null 2>&1 || true
  fi
  mv -f "${tmp_runtime}" "${settings_runtime_file}" || {
    rm -f "${tmp_assign}"
    json_error "E_IO" "无法替换 settings.runtime.sh"
    return 1
  }
  chmod 0600 "${settings_runtime_file}" >/dev/null 2>&1 || true
  rm -f "${tmp_assign}"

  for key in $(settings_allowed_keys); do
    [ -n "${key}" ] || continue
    if [ "${first}" -eq 1 ]; then
      changed="$(json_quote "${key}")"
      first=0
    else
      changed="${changed},$(json_quote "${key}")"
    fi
  done
  printf '{"ok":true,"data":{"changed":[%s]},"meta":{"version":1}}\n' "${changed}"
}

settings_set_json() {
  local key="$1"
  local value="$2"
  local file="${box_run}/settings-set.$$.json"
  case " $(settings_scalar_keys | busybox tr '\n' ' ') " in
    *" ${key} "*) ;;
    *)
      json_error "E_VALIDATION" "未知设置键: ${key}"
      return 1
      ;;
  esac
  mkdir -p "${box_run}" >/dev/null 2>&1 || true
  printf '{"%s":%s}' "${key}" "$(json_quote "${value}")" > "${file}" || {
    json_error "E_IO" "无法创建设置临时文件"
    return 1
  }
  settings_apply_json "${file}"
  local rc=$?
  rm -f "${file}"
  return "${rc}"
}

config_validate_json() {
  local file="$1"
  if ! ${yq} -e 'tag == "!!map" and has("proxy-groups") and has("rule-providers") and has("rules")' "${file}" >/dev/null 2>&1; then
    json_error "E_VALIDATION" "配置必须包含 proxy-groups、rule-providers、rules"
    return 1
  fi
  if ! ${yq} -e '."proxy-groups" | tag == "!!seq"' "${file}" >/dev/null 2>&1; then
    json_error "E_VALIDATION" "proxy-groups 必须是数组"
    return 1
  fi
  if ! ${yq} -e '."rule-providers" | tag == "!!map"' "${file}" >/dev/null 2>&1; then
    json_error "E_VALIDATION" "rule-providers 必须是对象"
    return 1
  fi
  if ! ${yq} -e '.rules | tag == "!!seq"' "${file}" >/dev/null 2>&1; then
    json_error "E_VALIDATION" "rules 必须是数组"
    return 1
  fi

  local tmpdir="${box_run_state}/config-validate.$$"
  rm -rf "${tmpdir}"
  mkdir -p "${tmpdir}" || {
    json_error "E_IO" "无法创建配置校验临时目录"
    return 1
  }

  ${yq} -r '."proxy-groups"[] | .name // ""' "${file}" > "${tmpdir}/group_names" 2>/dev/null
  ${yq} -r '."proxy-groups"[] | .type // ""' "${file}" > "${tmpdir}/group_types" 2>/dev/null
  ${yq} -r '."rule-providers" | to_entries[] | [.key, (.value.type // ""), (.value.behavior // ""), (.value.format // ""), (.value.url // ""), (.value.path // ""), (.value.interval // 0)] | @tsv' "${file}" > "${tmpdir}/provider_meta" 2>/dev/null
  busybox cut -f1 "${tmpdir}/provider_meta" > "${tmpdir}/provider_names"
  ${yq} -r '.rules[]' "${file}" > "${tmpdir}/rules" 2>/dev/null

  local group_count name_count
  group_count="$(${yq} -r '."proxy-groups" | length' "${file}" 2>/dev/null)"
  name_count="$(busybox awk 'length($0) > 0 { count++ } END { print count + 0 }' "${tmpdir}/group_names" 2>/dev/null)"
  if [ "${group_count}" != "${name_count}" ]; then
    rm -rf "${tmpdir}"
    json_error "E_VALIDATION" "每个策略组必须有非空名称"
    return 1
  fi

  local duplicate
  duplicate="$(busybox sort "${tmpdir}/group_names" | busybox uniq -d | busybox head -n 1)"
  if [ -n "${duplicate}" ]; then
    rm -rf "${tmpdir}"
    json_error "E_VALIDATION" "策略组名称重复: ${duplicate}"
    return 1
  fi

  local type
  while IFS= read -r type; do
    case "${type}" in
      select|url-test|fallback|load-balance|relay) ;;
      *)
        rm -rf "${tmpdir}"
        json_error "E_VALIDATION" "不支持的策略组类型: ${type:-空}"
        return 1
        ;;
    esac
  done < "${tmpdir}/group_types"

  local provider_name provider_type provider_behavior provider_format provider_url provider_path provider_interval
  while IFS="$(printf '\t')" read -r provider_name provider_type provider_behavior provider_format provider_url provider_path provider_interval; do
    [ -n "${provider_name}" ] || continue
    case "${provider_type}" in http|file|inline) ;; *)
      rm -rf "${tmpdir}"
      json_error "E_VALIDATION" "规则集 ${provider_name} 的 type 非法"
      return 1
    ;; esac
    case "${provider_behavior}" in domain|ipcidr|classical) ;; *)
      rm -rf "${tmpdir}"
      json_error "E_VALIDATION" "规则集 ${provider_name} 的 behavior 非法"
      return 1
    ;; esac
    case "${provider_format}" in yaml|mrs|text|"") ;; *)
      rm -rf "${tmpdir}"
      json_error "E_VALIDATION" "规则集 ${provider_name} 的 format 非法"
      return 1
    ;; esac
    case "${provider_interval}" in ''|*[!0-9]*)
      rm -rf "${tmpdir}"
      json_error "E_VALIDATION" "规则集 ${provider_name} 的 interval 必须是数字"
      return 1
    ;; esac
    if [ "${provider_type}" = "http" ]; then
      case "${provider_url}" in
        http://*|https://*) ;;
        *)
          rm -rf "${tmpdir}"
          json_error "E_VALIDATION" "规则集 ${provider_name} 的 URL 非法"
          return 1
          ;;
      esac
    fi
    if [ "${provider_type}" != "inline" ] && [ -z "${provider_path}" ]; then
      rm -rf "${tmpdir}"
      json_error "E_VALIDATION" "规则集 ${provider_name} 缺少 path"
      return 1
    fi
  done < "${tmpdir}/provider_meta"

  local rule rule_provider rule_target
  while IFS= read -r rule; do
    [ -n "${rule}" ] || continue
    case "${rule}" in
      RULE-SET,*)
        rule_provider="$(printf '%s' "${rule}" | busybox cut -d, -f2)"
        rule_target="$(printf '%s' "${rule}" | busybox cut -d, -f3)"
        if ! busybox grep -Fxq "${rule_provider}" "${tmpdir}/provider_names"; then
          rm -rf "${tmpdir}"
          json_error "E_VALIDATION" "规则引用了不存在的规则集: ${rule_provider}"
          return 1
        fi
        if ! busybox grep -Fxq "${rule_target}" "${tmpdir}/group_names"; then
          case "${rule_target}" in
            DIRECT|REJECT|REJECT-DROP|PASS|COMPATIBLE) ;;
            *)
              rm -rf "${tmpdir}"
              json_error "E_VALIDATION" "规则引用了不存在的策略组: ${rule_target}"
              return 1
              ;;
          esac
        fi
        ;;
      MATCH,*)
        rule_target="$(printf '%s' "${rule}" | busybox cut -d, -f2)"
        if ! busybox grep -Fxq "${rule_target}" "${tmpdir}/group_names"; then
          case "${rule_target}" in
            DIRECT|REJECT|REJECT-DROP|PASS|COMPATIBLE) ;;
            *)
              rm -rf "${tmpdir}"
              json_error "E_VALIDATION" "MATCH 引用了不存在的策略组: ${rule_target}"
              return 1
              ;;
          esac
        fi
        ;;
    esac
  done < "${tmpdir}/rules"

  rm -rf "${tmpdir}"
  return 0
}

core_get_json() {
  ensure_yq
  if [ "${bin_name}" != "mihomo" ]; then
    json_error "E_VALIDATION" "核心配置仅支持 mihomo"
    return 1
  fi
  if [ ! -f "${mihomo_config}" ]; then
    json_error "E_NOT_FOUND" "配置文件不存在: ${mihomo_config}"
    return 1
  fi

  local structured raw
  structured="$(${yq} -o=json '{
    "mode": ((.mode // "rule") | downcase),
    "log-level": ((."log-level" // "error") | downcase),
    "allow-lan": (."allow-lan" // false),
    "external-controller": (."external-controller" // ""),
    "external-ui": (."external-ui" // ""),
    "profile.store-selected": (.profile."store-selected" // true),
    "profile.store-fake-ip": (.profile."store-fake-ip" // true),
    "unified-delay": (."unified-delay" // false),
    "tcp-concurrent": (."tcp-concurrent" // true),
    "find-process-mode": ((."find-process-mode" // "always") | downcase)
  }' "${mihomo_config}" 2>/dev/null)" || {
    json_error "E_IO" "读取 mihomo 配置失败"
    return 1
  }
  raw="$(cat "${mihomo_config}" 2>/dev/null)" || {
    json_error "E_IO" "读取 mihomo 原始配置失败"
    return 1
  }
  printf '{"ok":true,"data":{"core":"mihomo","file":%s,"structured":%s,"raw":%s},"meta":{"version":1}}\n' \
    "$(json_quote "${mihomo_config}")" "${structured}" "$(json_quote "${raw}")"
}

core_apply_json() {
  local file="$1"
  if [ ! -s "${file}" ]; then
    json_error "E_USAGE" "core apply: 缺少 JSON 文件"
    return 1
  fi
  ensure_yq
  if [ "${bin_name}" != "mihomo" ]; then
    json_error "E_VALIDATION" "核心配置仅支持 mihomo"
    return 1
  fi
  if [ ! -f "${mihomo_config}" ]; then
    json_error "E_NOT_FOUND" "配置文件不存在: ${mihomo_config}"
    return 1
  fi
  if ! ${yq} -e 'tag == "!!map"' "${file}" >/dev/null 2>&1; then
    json_error "E_VALIDATION" "core apply 必须是 JSON 对象"
    return 1
  fi

  local has_raw has_structured
  has_raw="$(${yq} -r 'has("raw")' "${file}" 2>/dev/null)"
  has_structured="$(${yq} -r 'has("structured")' "${file}" 2>/dev/null)"
  if [ "${has_raw}" = "true" ] && [ "${has_structured}" = "true" ]; then
    json_error "E_VALIDATION" "raw 与 structured 不能同时提交"
    return 1
  fi
  if [ "${has_raw}" != "true" ] && [ "${has_structured}" != "true" ]; then
    json_error "E_VALIDATION" "缺少 raw 或 structured"
    return 1
  fi

  local temp_file="${mihomo_config}.tmp.$$"
  local mode key tag value path
  if [ "${has_raw}" = "true" ]; then
    tag="$(${yq} -r '.raw | tag' "${file}" 2>/dev/null)"
    if [ "${tag}" != "!!str" ]; then
      json_error "E_VALIDATION" "raw 必须是字符串"
      return 1
    fi
    ${yq} -r '.raw' "${file}" > "${temp_file}" 2>/dev/null || {
      rm -f "${temp_file}"
      json_error "E_IO" "写入临时配置失败"
      return 1
    }
    if ! ${yq} -e '.' "${temp_file}" >/dev/null 2>&1; then
      rm -f "${temp_file}"
      json_error "E_VALIDATION" "YAML 语法校验失败"
      return 1
    fi
    mode="raw"
  else
    if ! ${yq} -e '.structured | tag == "!!map"' "${file}" >/dev/null 2>&1; then
      json_error "E_VALIDATION" "structured 必须是对象"
      return 1
    fi
    cp -f "${mihomo_config}" "${temp_file}" || {
      json_error "E_IO" "无法创建临时配置"
      return 1
    }
    ${yq} -r '.structured | keys | .[]' "${file}" > "${temp_file}.keys" 2>/dev/null
    while IFS= read -r key; do
      [ -n "${key}" ] || continue
      case "${key}" in
        mode) path='.mode' ;;
        log-level) path='."log-level"' ;;
        allow-lan) path='."allow-lan"' ;;
        external-controller) path='."external-controller"' ;;
        external-ui) path='."external-ui"' ;;
        profile.store-selected) path='.profile."store-selected"' ;;
        profile.store-fake-ip) path='.profile."store-fake-ip"' ;;
        unified-delay) path='."unified-delay"' ;;
        tcp-concurrent) path='."tcp-concurrent"' ;;
        find-process-mode) path='."find-process-mode"' ;;
        *)
          rm -f "${temp_file}" "${temp_file}.keys"
          json_error "E_VALIDATION" "不支持的 mihomo 配置字段: ${key}"
          return 1
          ;;
      esac
      tag="$(${yq} -r ".structured.\"${key}\" | tag" "${file}" 2>/dev/null)"
      value="$(${yq} -r ".structured.\"${key}\"" "${file}" 2>/dev/null)"
      case "${key}" in
        allow-lan|profile.store-selected|profile.store-fake-ip|unified-delay|tcp-concurrent)
          case "${tag}" in
            '!!bool') ;;
            *) rm -f "${temp_file}" "${temp_file}.keys"; json_error "E_VALIDATION" "${key} 必须是布尔值"; return 1 ;;
          esac
          ;;
        mode)
          case "${value}" in rule|global|direct) ;; *) rm -f "${temp_file}" "${temp_file}.keys"; json_error "E_VALIDATION" "mode 必须是 rule/global/direct"; return 1 ;; esac
          ;;
        log-level)
          case "${value}" in silent|error|warning|info|debug) ;; *) rm -f "${temp_file}" "${temp_file}.keys"; json_error "E_VALIDATION" "log-level 非法"; return 1 ;; esac
          ;;
        find-process-mode)
          case "${value}" in always|strict|off) ;; *) rm -f "${temp_file}" "${temp_file}.keys"; json_error "E_VALIDATION" "find-process-mode 非法"; return 1 ;; esac
          ;;
        external-controller|external-ui)
          case "${tag}" in
            '!!str') ;;
            *) rm -f "${temp_file}" "${temp_file}.keys"; json_error "E_VALIDATION" "${key} 必须是字符串"; return 1 ;;
          esac
          ;;
      esac
      local literal
      case "${key}" in
        allow-lan|profile.store-selected|profile.store-fake-ip|unified-delay|tcp-concurrent)
          literal="${value}"
          ;;
        *)
          literal="$(json_quote "${value}")"
          ;;
      esac
      if ! ${yq} -i "${path} = ${literal}" "${temp_file}" 2>/dev/null; then
        rm -f "${temp_file}" "${temp_file}.keys"
        json_error "E_IO" "写入临时配置失败"
        return 1
      fi
    done < "${temp_file}.keys"
    rm -f "${temp_file}.keys"
    if ! ${yq} -e '.' "${temp_file}" >/dev/null 2>&1; then
      rm -f "${temp_file}"
      json_error "E_VALIDATION" "YAML 语法校验失败"
      return 1
    fi
    mode="structured"
  fi

  mv "${temp_file}" "${mihomo_config}" || {
    rm -f "${temp_file}"
    json_error "E_IO" "替换 mihomo 配置失败"
    return 1
  }
  json_ok "$(printf '{"changed":["mihomo"],"mode":%s}' "$(json_quote "${mode}")")"
}

lists_get_json() {
  local package_file="${box_dir}/package.list.cfg"
  local gid_file="${box_dir}/gid.list.cfg"
  local ap_file="${box_dir}/ap.list.cfg"

  printf '{"ok":true,"data":{"package":['
  local first=1 line uid package
  if [ -f "${package_file}" ]; then
    while IFS= read -r line; do
      line="$(printf '%s' "${line}" | busybox sed -E 's/^[[:space:]]+//;s/[[:space:]]+$//')"
      case "${line}" in ''|'#'*|mode:*) continue ;; esac
      case "${line}" in
        *:*) uid="${line%%:*}"; package="${line#*:}" ;;
        *) uid=""; package="${line}" ;;
      esac
      [ -n "${package}" ] || continue
      [ "${first}" -eq 1 ] || printf ','
      printf '{"uid":%s,"package":%s}' "$(json_quote "${uid}")" "$(json_quote "${package}")"
      first=0
    done < "${package_file}"
  fi
  printf '],"gid":['
  first=1
  if [ -f "${gid_file}" ]; then
    while IFS= read -r line; do
      line="$(printf '%s' "${line}" | busybox sed -E 's/^[[:space:]]+//;s/[[:space:]]+$//')"
      case "${line}" in ''|'#'*) continue ;; esac
      [ "${first}" -eq 1 ] || printf ','
      printf '%s' "$(json_quote "${line}")"
      first=0
    done < "${gid_file}"
  fi
  printf '],"interfaces":{"allow":['
  first=1
  if [ -f "${ap_file}" ]; then
    while IFS=' ' read -r action iface rest; do
      [ "${action}" = "allow" ] || continue
      [ -n "${iface}" ] || continue
      [ "${first}" -eq 1 ] || printf ','
      printf '%s' "$(json_quote "${iface}")"
      first=0
    done < "${ap_file}"
  fi
  printf '],"ignore":['
  first=1
  if [ -f "${ap_file}" ]; then
    while IFS=' ' read -r action iface rest; do
      [ "${action}" = "ignore" ] || continue
      [ -n "${iface}" ] || continue
      [ "${first}" -eq 1 ] || printf ','
      printf '%s' "$(json_quote "${iface}")"
      first=0
    done < "${ap_file}"
  fi
  printf ']},"files":{"package":%s,"gid":%s,"interfaces":%s}},"meta":{"version":1}}\n' \
    "$(json_quote "${package_file}")" "$(json_quote "${gid_file}")" "$(json_quote "${ap_file}")"
}

lists_apply_json() (
  local file="$1"
  if [ ! -s "${file}" ]; then
    json_error "E_USAGE" "lists apply: 缺少 JSON 文件"
    return 1
  fi
  ensure_yq
  if ! ${yq} -e 'tag == "!!map"' "${file}" >/dev/null 2>&1; then
    json_error "E_VALIDATION" "lists apply 必须是 JSON 对象"
    return 1
  fi

  local package_file="${box_dir}/package.list.cfg"
  local gid_file="${box_dir}/gid.list.cfg"
  local ap_file="${box_dir}/ap.list.cfg"
  local package_tmp="${package_file}.tmp.$$"
  local gid_tmp="${gid_file}.tmp.$$"
  local ap_tmp="${ap_file}.tmp.$$"
  local changed="" uid package line entry
  trap 'rm -f "${package_tmp}" "${gid_tmp}" "${ap_tmp}" "${package_tmp}.rows" "${gid_tmp}.rows" "${ap_tmp}.allow" "${ap_tmp}.ignore"' EXIT

  if ${yq} -e 'has("package")' "${file}" >/dev/null 2>&1; then
    if ! ${yq} -e '.package | tag == "!!seq"' "${file}" >/dev/null 2>&1; then
      json_error "E_VALIDATION" "package 必须是数组"
      return 1
    fi
    printf '# 应用包名配置文件\n# 格式: <用户ID>:<包名> 或 <包名>\n' > "${package_tmp}" || {
      json_error "E_IO" "写入 package.list.cfg 临时文件失败"
      return 1
    }
    ${yq} -r '.package[] | [(.uid // ""), (.package // "")] | join("|")' "${file}" > "${package_tmp}.rows" 2>/dev/null
    while IFS='|' read -r uid package; do
      case "${uid}" in
        ''|*[!0-9]*) [ -z "${uid}" ] || { rm -f "${package_tmp}" "${package_tmp}.rows"; json_error "E_VALIDATION" "非法 UID: ${uid}"; return 1; } ;;
      esac
      printf '%s' "${package}" | busybox grep -Eq '^[A-Za-z0-9_]+(\.[A-Za-z0-9_]+)+$' || {
        rm -f "${package_tmp}" "${package_tmp}.rows"
        json_error "E_VALIDATION" "非法包名: ${package}"
        return 1
      }
      if [ -n "${uid}" ]; then
        printf '%s:%s\n' "${uid}" "${package}" >> "${package_tmp}"
      else
        printf '%s\n' "${package}" >> "${package_tmp}"
      fi
    done < "${package_tmp}.rows"
    rm -f "${package_tmp}.rows"
    changed="${changed}${changed:+,}\"package\""
  fi

  if ${yq} -e 'has("gid")' "${file}" >/dev/null 2>&1; then
    if ! ${yq} -e '.gid | tag == "!!seq"' "${file}" >/dev/null 2>&1; then
      json_error "E_VALIDATION" "gid 必须是数组"
      return 1
    fi
    printf '# GID 配置文件\n# 每行一个 GID 数字\n' > "${gid_tmp}" || {
      json_error "E_IO" "写入 gid.list.cfg 临时文件失败"
      return 1
    }
    ${yq} -r '.gid[]' "${file}" > "${gid_tmp}.rows" 2>/dev/null
    while IFS= read -r line; do
      [ -n "${line}" ] || continue
      printf '%s' "${line}" | busybox grep -Eq '^[0-9]+$' || {
        rm -f "${gid_tmp}" "${gid_tmp}.rows"
        json_error "E_VALIDATION" "非法 GID: ${line}"
        return 1
      }
      printf '%s\n' "${line}" >> "${gid_tmp}"
    done < "${gid_tmp}.rows"
    rm -f "${gid_tmp}.rows"
    changed="${changed}${changed:+,}\"gid\""
  fi

  if ${yq} -e 'has("interfaces")' "${file}" >/dev/null 2>&1; then
    if ! ${yq} -e '.interfaces | tag == "!!map"' "${file}" >/dev/null 2>&1; then
      json_error "E_VALIDATION" "interfaces 必须是对象"
      return 1
    fi
    printf '# 网络接口 allow/ignore 配置\n# 格式: allow <接口名> 或 ignore <接口名>\n' > "${ap_tmp}" || {
      json_error "E_IO" "写入 ap.list.cfg 临时文件失败"
      return 1
    }
    if ${yq} -e '.interfaces | has("allow")' "${file}" >/dev/null 2>&1; then
      if ! ${yq} -e '.interfaces.allow | tag == "!!seq"' "${file}" >/dev/null 2>&1; then
        rm -f "${ap_tmp}"
        json_error "E_VALIDATION" "interfaces.allow 必须是数组"
        return 1
      fi
      ${yq} -r '.interfaces.allow[]' "${file}" > "${ap_tmp}.allow" 2>/dev/null
      while IFS= read -r entry; do
        [ -n "${entry}" ] || continue
        printf '%s' "${entry}" | busybox grep -Eq '^[A-Za-z0-9_.:+-]+$' || {
          rm -f "${ap_tmp}" "${ap_tmp}.allow"
          json_error "E_VALIDATION" "非法接口名: ${entry}"
          return 1
        }
        printf 'allow %s\n' "${entry}" >> "${ap_tmp}"
      done < "${ap_tmp}.allow"
      rm -f "${ap_tmp}.allow"
    fi
    if ${yq} -e '.interfaces | has("ignore")' "${file}" >/dev/null 2>&1; then
      if ! ${yq} -e '.interfaces.ignore | tag == "!!seq"' "${file}" >/dev/null 2>&1; then
        rm -f "${ap_tmp}"
        json_error "E_VALIDATION" "interfaces.ignore 必须是数组"
        return 1
      fi
      ${yq} -r '.interfaces.ignore[]' "${file}" > "${ap_tmp}.ignore" 2>/dev/null
      while IFS= read -r entry; do
        [ -n "${entry}" ] || continue
        printf '%s' "${entry}" | busybox grep -Eq '^[A-Za-z0-9_.:+-]+$' || {
          rm -f "${ap_tmp}" "${ap_tmp}.ignore"
          json_error "E_VALIDATION" "非法接口名: ${entry}"
          return 1
        }
        printf 'ignore %s\n' "${entry}" >> "${ap_tmp}"
      done < "${ap_tmp}.ignore"
      rm -f "${ap_tmp}.ignore"
    fi
    changed="${changed}${changed:+,}\"interfaces\""
  fi

  if [ -f "${package_tmp}" ]; then mv "${package_tmp}" "${package_file}" || { rm -f "${package_tmp}" "${gid_tmp}" "${ap_tmp}"; json_error "E_IO" "替换 package.list.cfg 失败"; return 1; }; fi
  if [ -f "${gid_tmp}" ]; then mv "${gid_tmp}" "${gid_file}" || { rm -f "${gid_tmp}" "${ap_tmp}"; json_error "E_IO" "替换 gid.list.cfg 失败"; return 1; }; fi
  if [ -f "${ap_tmp}" ]; then mv "${ap_tmp}" "${ap_file}" || { rm -f "${ap_tmp}"; json_error "E_IO" "替换 ap.list.cfg 失败"; return 1; }; fi
  json_ok "$(printf '{"changed":[%s]}' "${changed}")"
)

config_get_json() {
  ensure_yq
  if [ "${bin_name}" != "mihomo" ]; then
    json_error "E_VALIDATION" "策略与规则配置仅支持 mihomo"
    return 1
  fi
  if [ ! -f "${mihomo_config}" ]; then
    json_error "E_NOT_FOUND" "配置文件不存在: ${mihomo_config}"
    return 1
  fi
  local data
  data="$(${yq} -o=json '{ "proxy-groups": (."proxy-groups" // []), "rule-providers": (."rule-providers" // {}), "rules": (.rules // []) }' "${mihomo_config}" 2>/dev/null)" || {
    json_error "E_IO" "读取配置失败"
    return 1
  }
  json_ok "${data}"
}

config_apply_json() {
  local file="$1"
  if [ "${bin_name}" != "mihomo" ]; then
    json_error "E_VALIDATION" "策略与规则配置仅支持 mihomo"
    return 1
  fi
  if [ ! -s "${file}" ]; then
    json_error "E_USAGE" "config apply: 缺少 JSON 文件"
    return 1
  fi
  if [ ! -f "${mihomo_config}" ]; then
    json_error "E_NOT_FOUND" "配置文件不存在: ${mihomo_config}"
    return 1
  fi

  ensure_yq
  config_validate_json "${file}" || return 1

  local backup="${mihomo_config}.bak"
  cp -f "${mihomo_config}" "${backup}" 2>/dev/null || {
    json_error "E_IO" "无法备份配置文件"
    return 1
  }

  if ! ${yq} -i '."proxy-groups" = (load("'"${file}"'") | ."proxy-groups") | ."rule-providers" = (load("'"${file}"'") | ."rule-providers") | .rules = (load("'"${file}"'") | .rules)' "${mihomo_config}" 2>/dev/null; then
    cp -f "${backup}" "${mihomo_config}" 2>/dev/null
    json_error "E_IO" "写入配置失败，已还原"
    return 1
  fi

  if ! ${yq} '.' "${mihomo_config}" >/dev/null 2>&1; then
    cp -f "${backup}" "${mihomo_config}" 2>/dev/null
    json_error "E_IO" "YAML 校验失败，已还原"
    return 1
  fi

  printf '{"ok":true,"data":{"changed":["proxy-groups","rule-providers","rules"]},"meta":{"version":1}}\n'
}

config_delete_json() {
  if [ ! -f "${mihomo_config}" ]; then
    json_error "E_NOT_FOUND" "配置文件不存在: ${mihomo_config}"
    return 1
  fi
  local deleted="${mihomo_config}.deleted.$(date +%Y%m%d%H%M%S)"
  mv "${mihomo_config}" "${deleted}" || {
    json_error "E_IO" "删除配置失败"
    return 1
  }
  json_ok "$(printf '{"path":%s}' "$(json_quote "${deleted}")")"
}

# ---------------------------------------------------------------------------
# 全局规则与策略订阅
# ---------------------------------------------------------------------------

rule_profile_url_file() {
  printf '%s/rule-profile.url' "${mihomo_config%/*}"
}

rule_profile_updated_file() {
  printf '%s/rule-profile.updated_at' "${box_run_state}"
}

rule_profile_pending_file() {
  printf '%s/rule-profile.pending.yaml' "${box_run_state}"
}

rule_profile_counts_json() {
  local file="${1:-${mihomo_config}}"
  local groups=0 providers=0 rules=0
  if [ -f "${file}" ]; then
    groups="$(${yq} -r '."proxy-groups" | length' "${file}" 2>/dev/null)" || groups=0
    providers="$(${yq} -r '."rule-providers" | length' "${file}" 2>/dev/null)" || providers=0
    rules="$(${yq} -r '.rules | length' "${file}" 2>/dev/null)" || rules=0
  fi
  case "${groups}" in ''|*[!0-9]*) groups=0 ;; esac
  case "${providers}" in ''|*[!0-9]*) providers=0 ;; esac
  case "${rules}" in ''|*[!0-9]*) rules=0 ;; esac
  printf '{"groups":%s,"rule_providers":%s,"rules":%s}' "${groups}" "${providers}" "${rules}"
}

rule_profile_get_json() {
  ensure_yq
  local supported=false
  [ "${bin_name}" = "mihomo" ] && supported=true
  local url_file configured=false url_masked=""
  url_file="$(rule_profile_url_file)"
  if [ -s "${url_file}" ]; then
    configured=true
    url_masked="$(mask_url "$(cat "${url_file}" 2>/dev/null)")"
  fi
  local updated=""
  [ -f "$(rule_profile_updated_file)" ] && updated="$(cat "$(rule_profile_updated_file)" 2>/dev/null)"
  local data
  data="$(printf '{"supported":%s,"configured":%s,"url_masked":%s,"updated_at":%s,"counts":%s}' \
    "$(json_bool "${supported}")" "$(json_bool "${configured}")" \
    "$(json_quote "${url_masked}")" "$(json_quote "${updated}")" \
    "$(rule_profile_counts_json)")"
  json_ok "${data}"
}

rule_profile_set_json() {
  local url="$1"
  if [ "${bin_name}" != "mihomo" ]; then
    json_error "E_VALIDATION" "规则与策略订阅仅支持 mihomo"
    return 1
  fi
  case "${url}" in
    http://*|https://*) ;;
    *)
      json_error "E_VALIDATION" "规则模板地址必须以 http:// 或 https:// 开头"
      return 1
      ;;
  esac
  local url_file tmp_file
  url_file="$(rule_profile_url_file)"
  mkdir -p "$(dirname "${url_file}")" || {
    json_error "E_IO" "无法创建规则订阅目录"
    return 1
  }
  tmp_file="${url_file}.tmp.$$"
  printf '%s\n' "${url}" > "${tmp_file}" || {
    json_error "E_IO" "无法保存规则模板地址"
    return 1
  }
  chmod 0600 "${tmp_file}" >/dev/null 2>&1 || true
  mv "${tmp_file}" "${url_file}" || {
    rm -f "${tmp_file}"
    json_error "E_IO" "无法保存规则模板地址"
    return 1
  }
  rm -f "$(rule_profile_pending_file)" "$(rule_profile_pending_file).bak" 2>/dev/null
  rule_profile_get_json
}

rule_profile_clear_json() {
  local url_file
  url_file="$(rule_profile_url_file)"
  rm -f "${url_file}" "$(rule_profile_updated_file)" \
    "$(rule_profile_pending_file)" "$(rule_profile_pending_file).bak" 2>/dev/null
  json_ok '{"cleared":true}'
}

rule_profile_url_json() {
  local url_file
  url_file="$(rule_profile_url_file)"
  if [ ! -s "${url_file}" ]; then
    json_error "E_NOT_FOUND" "还没有配置规则与策略订阅"
    return 1
  fi
  json_ok "$(printf '{"url":%s}' "$(json_quote "$(cat "${url_file}" 2>/dev/null)")")"
}

rule_profile_preserve_dns_providers() {
  local file="$1"
  local old_file="${mihomo_config}"
  local names="${box_run_state}/rule-profile.refs.$$"
  local name
  [ -f "${file}" ] && [ -f "${old_file}" ] || return 0

  {
    ${yq} -r '.dns."fake-ip-filter"[]?' "${old_file}" 2>/dev/null
    ${yq} -r '.dns."nameserver-policy" | keys[]?' "${old_file}" 2>/dev/null
  } | busybox grep '^rule-set:' | busybox sed 's/^rule-set://' | busybox sort -u > "${names}" 2>/dev/null || true

  while IFS= read -r name; do
    [ -n "${name}" ] || continue
    case "${name}" in
      ''|*[!A-Za-z0-9_.-]*) continue ;;
    esac
    if ${yq} -e ".rule-providers | has(\"${name}\")" "${file}" >/dev/null 2>&1; then
      continue
    fi
    if ${yq} -e ".rule-providers | has(\"${name}\")" "${old_file}" >/dev/null 2>&1; then
      if ! ${yq} -i ".rule-providers[\"${name}\"] = (load(\"${old_file}\") | .rule-providers[\"${name}\"])" "${file}" 2>/dev/null; then
        rm -f "${names}"
        json_error "E_IO" "保留 DNS 引用的规则集失败: ${name}"
        return 1
      fi
    fi
  done < "${names}"
  rm -f "${names}" 2>/dev/null
  return 0
}

rule_profile_download_json() {
  local dest="$1"
  local url="$2"
  UPFILE_ERROR=""
  if ! LOG_MASK_URL=mask upfile "${dest}" "${url}" "mihomo"; then
    json_error "E_DOWNLOAD" "下载规则模板失败" "${UPFILE_ERROR:-unknown}"
    return 1
  fi
  return 0
}

rule_profile_fetch_json() {
  if [ "${bin_name}" != "mihomo" ]; then
    json_error "E_VALIDATION" "规则与策略订阅仅支持 mihomo"
    return 1
  fi
  local url_file url
  url_file="$(rule_profile_url_file)"
  if [ ! -s "${url_file}" ]; then
    json_error "E_NOT_FOUND" "还没有配置规则与策略订阅"
    return 1
  fi
  url="$(cat "${url_file}" 2>/dev/null)"
  ensure_yq
  local pending tmp_file
  pending="$(rule_profile_pending_file)"
  tmp_file="${pending}.tmp.$$"
  rm -f "${tmp_file}" "${tmp_file}.bak" 2>/dev/null
  if ! rule_profile_download_json "${tmp_file}" "${url}"; then
    rm -f "${tmp_file}" "${tmp_file}.bak" 2>/dev/null
    return 1
  fi
  if ! config_validate_json "${tmp_file}"; then
    rm -f "${tmp_file}" "${tmp_file}.bak" 2>/dev/null
    return 1
  fi
  if ! rule_profile_preserve_dns_providers "${tmp_file}"; then
    rm -f "${tmp_file}" "${tmp_file}.bak" 2>/dev/null
    return 1
  fi
  if ! config_validate_json "${tmp_file}"; then
    rm -f "${tmp_file}" "${tmp_file}.bak" 2>/dev/null
    return 1
  fi
  mv "${tmp_file}" "${pending}" || {
    rm -f "${tmp_file}" "${tmp_file}.bak" 2>/dev/null
    json_error "E_IO" "无法保存待应用的规则模板"
    return 1
  }
  local data
  data="$(printf '{"url_masked":%s,"fetched_at":%s,"counts":%s}' \
    "$(json_quote "$(mask_url "${url}")")" \
    "$(json_quote "$(date '+%Y-%m-%dT%H:%M:%S%z')")" \
    "$(rule_profile_counts_json "${pending}")")"
  json_ok "${data}"
}

rule_profile_apply_json() {
  local do_reload="${1:-false}"
  if [ "${bin_name}" != "mihomo" ]; then
    json_error "E_VALIDATION" "规则与策略订阅仅支持 mihomo"
    return 1
  fi
  local pending apply_output updated
  pending="$(rule_profile_pending_file)"
  if [ ! -s "${pending}" ]; then
    json_error "E_NOT_FOUND" "没有待应用的规则模板，请先更新"
    return 1
  fi
  ensure_yq
  if ! rule_profile_preserve_dns_providers "${pending}"; then
    return 1
  fi
  if ! apply_output="$(config_apply_json "${pending}" 2>/dev/null)"; then
    if [ -n "${apply_output}" ]; then
      printf '%s\n' "${apply_output}"
    else
      json_error "E_IO" "应用规则模板失败"
    fi
    return 1
  fi
  update_mihomo_rulesets >/dev/null 2>&1 || log Warning "规则集前缀更新失败，请手动检查配置文件"
  updated="$(date '+%Y-%m-%dT%H:%M:%S%z')"
  printf '%s\n' "${updated}" > "$(rule_profile_updated_file)" 2>/dev/null || true
  rm -f "${pending}" "${pending}.bak" 2>/dev/null

  local reloaded=false reload_output data
  if [ "${do_reload}" = "true" ] && is_box_running; then
    if reload_output="$(reload 2>&1)"; then
      reloaded=true
    else
      json_error "E_RELOAD" "配置已保存，但 mihomo 重载失败" "${reload_output}"
      return 1
    fi
  fi

  data="$(printf '{"changed":["proxy-groups","rule-providers","rules"],"reloaded":%s,"updated_at":%s,"counts":%s}' \
    "$(json_bool "${reloaded}")" "$(json_quote "${updated}")" \
    "$(rule_profile_counts_json)")"
  json_ok "${data}"
}

logs_clear_json() {
  local file cleared=""
  for file in "${box_run}/runs.log" "${box_run}/tool.log" "${box_run}/${bin_name}.log"; do
    if [ -f "${file}" ]; then
      : > "${file}"
      cleared="${cleared}${cleared:+,}$(json_quote "${file}")"
    fi
  done
  printf '{"ok":true,"data":{"cleared":[%s]},"meta":{"version":1}}\n' "${cleared}"
}

reset_subscriptions_json() {
  : > "${subscriptions_file}"
  rm -f "${mihomo_provide_path}"/*.yaml 2>/dev/null
  json_ok '{"cleared":true}'
}

service_json() {
  local action="$1"
  case "${action}" in
    start|stop|restart|reload) ;;
    *)
      json_error "E_USAGE" "服务操作必须是 start、stop、restart 或 reload"
      return 1
      ;;
  esac

  local output rc
  if [ "${action}" = "reload" ]; then
    output="$(reload 2>&1)"
    rc=$?
  else
    output="$("${scripts_dir}/box.service" "${action}" 2>&1)"
    rc=$?
  fi
  if [ "${rc}" -ne 0 ]; then
    json_error "E_COMMAND" "服务操作失败: ${action}" "${output}"
    return 1
  fi
  json_ok "$(printf '{"action":%s,"output":%s}' "$(json_quote "${action}")" "$(json_quote "${output}")")"
}

task_tasks_dir() {
  printf '%s/tasks' "${box_run}"
}

task_valid_id() {
  case "$1" in
    ''|*[!0-9-]*) return 1 ;;
    *) return 0 ;;
  esac
}

task_read_value() {
  local dir="$1"
  local name="$2"
  [ -f "${dir}/${name}" ] && cat "${dir}/${name}" 2>/dev/null || true
}

task_update_interrupted() {
  local dir="$1"
  [ -f "${dir}/state" ] || return 0
  [ "$(cat "${dir}/state" 2>/dev/null)" = "running" ] || return 0
  local pid
  pid="$(cat "${dir}/pid" 2>/dev/null)"
  case "${pid}" in
    ''|*[!0-9]*)
      printf 'interrupted\n' > "${dir}/state"
      date '+%Y-%m-%dT%H:%M:%S%z' > "${dir}/finished_at"
      return 0
      ;;
  esac
  if ! kill -0 "${pid}" 2>/dev/null; then
    printf 'interrupted\n' > "${dir}/state"
    date '+%Y-%m-%dT%H:%M:%S%z' > "${dir}/finished_at"
  fi
}

task_running_count() {
  local dir count=0
  [ -d "$(task_tasks_dir)" ] || {
    printf '0'
    return 0
  }
  for dir in "$(task_tasks_dir)"/*; do
    [ -d "${dir}" ] || continue
    task_update_interrupted "${dir}"
    [ "$(cat "${dir}/state" 2>/dev/null)" = "running" ] && count=$((count + 1))
  done
  printf '%s' "${count}"
}

task_running_id() {
  local dir
  [ -d "$(task_tasks_dir)" ] || return 0
  for dir in "$(task_tasks_dir)"/*; do
    [ -d "${dir}" ] || continue
    task_update_interrupted "${dir}"
    if [ "$(cat "${dir}/state" 2>/dev/null)" = "running" ]; then
      basename "${dir}"
      return 0
    fi
  done
}

task_cleanup() {
  local dirs
  dirs="$(busybox ls -1dt "$(task_tasks_dir)"/* 2>/dev/null | busybox tail -n +21)"
  for dir in ${dirs}; do
    rm -rf "${dir}"
  done
}

task_json_for_dir() {
  local dir="$1"
  local id kind title state pid started finished exit_code cancelable output_size
  id="$(basename "${dir}")"
  kind="$(task_read_value "${dir}" "kind")"
  title="$(task_read_value "${dir}" "title")"
  state="$(task_read_value "${dir}" "state")"
  [ -n "${state}" ] || state="unknown"
  pid="$(task_read_value "${dir}" "pid")"
  case "${pid}" in ''|*[!0-9]*) pid=null ;; esac
  started="$(task_read_value "${dir}" "started_at")"
  finished="$(task_read_value "${dir}" "finished_at")"
  exit_code="$(task_read_value "${dir}" "exit_code")"
  case "${exit_code}" in ''|*[!0-9]*) exit_code=null ;; esac
  cancelable="$(task_read_value "${dir}" "cancelable")"
  [ "${cancelable}" = "true" ] || cancelable=false
  output_size=0
  if [ -f "${dir}/output.log" ]; then
    output_size="$(busybox wc -c < "${dir}/output.log" 2>/dev/null | busybox tr -d '[:space:]')"
    [ -n "${output_size}" ] || output_size=0
  fi
  printf '{"id":%s,"kind":%s,"title":%s,"state":%s,"pid":%s,"started_at":%s,"finished_at":%s,"exit_code":%s,"cancelable":%s,"output_size":%s}' \
    "$(json_quote "${id}")" "$(json_quote "${kind}")" "$(json_quote "${title}")" \
    "$(json_quote "${state}")" "${pid}" "$(json_quote "${started}")" "$(json_quote "${finished}")" \
    "${exit_code}" "$(json_bool "${cancelable}")" "${output_size}"
}

task_start_json() {
  local kind="$1"
  local arg="${2:-}"
  local title="" cancelable=true

  case "${kind}" in
    sub-update)
      title="更新订阅"
      ;;
    upkernel)
      title="更新 ${arg} 核心"
      case "${arg}" in mihomo|sing-box|mihomo_smart|sing-box_ref1nd) ;; *)
        json_error "E_VALIDATION" "不支持的内核: ${arg:-空}"
        return 1
      ;; esac
      cancelable=false
      ;;
    upcnip)
      title="更新 CNIP 数据"
      ;;
    geox)
      title="更新 Geo 数据"
      ;;
    xui)
      title="更新面板资源"
      ;;
    *)
      json_error "E_VALIDATION" "不支持的任务类型: ${kind:-空}"
      return 1
      ;;
  esac

  local running_id
  running_id="$(task_running_id)"
  if [ -n "${running_id}" ]; then
    json_error "E_BUSY" "已有任务正在运行" "${running_id}"
    return 1
  fi

  local tasks_dir id dir
  tasks_dir="$(task_tasks_dir)"
  mkdir -p "${tasks_dir}" || {
    json_error "E_IO" "无法创建任务目录"
    return 1
  }
  task_cleanup
  id="$(date +%Y%m%d%H%M%S)-$$"
  dir="${tasks_dir}/${id}"
  mkdir -p "${dir}" || {
    json_error "E_IO" "无法创建任务目录"
    return 1
  }
  printf '%s\n' "${kind}" > "${dir}/kind"
  printf '%s\n' "${title}" > "${dir}/title"
  printf 'running\n' > "${dir}/state"
  printf '%s\n' "${cancelable}" > "${dir}/cancelable"
  printf '%s\n' "$(date '+%Y-%m-%dT%H:%M:%S%z')" > "${dir}/started_at"
  [ -n "${arg}" ] && printf '%s\n' "${arg}" > "${dir}/arg"

  local task_shell="/system/bin/sh"
  if [ ! -x "${task_shell}" ]; then
    if command -v bash >/dev/null 2>&1; then
      task_shell="$(command -v bash)"
    elif command -v mksh >/dev/null 2>&1; then
      task_shell="$(command -v mksh)"
    else
      task_shell="$(command -v sh)"
    fi
  fi
  if command -v nohup >/dev/null 2>&1; then
    nohup "${task_shell}" "${scripts_dir}/box.tool" task _run "${id}" > "${dir}/runner.log" 2>&1 &
  else
    "${task_shell}" "${scripts_dir}/box.tool" task _run "${id}" > "${dir}/runner.log" 2>&1 &
  fi
  printf '%s\n' "$!" > "${dir}/pid"
  json_ok "$(task_json_for_dir "${dir}")"
}

task_run_internal() {
  local id="$1"
  task_valid_id "${id}" || exit 1
  local dir
  dir="$(task_tasks_dir)/${id}"
  [ -d "${dir}" ] || exit 1

  trap 'printf "canceled\n" > "${dir}/state"; date "+%Y-%m-%dT%H:%M:%S%z" > "${dir}/finished_at"; exit 130' TERM INT
  exec >> "${dir}/output.log" 2>&1

  local kind arg rc
  kind="$(task_read_value "${dir}" "kind")"
  arg="$(task_read_value "${dir}" "arg")"
  case "${kind}" in
    sub-update) sub_update "${arg}" ;;
    upkernel) upkernel "${arg}" ;;
    upcnip) upcnip ;;
    geox) upgeox ;;
    xui) upxui ;;
    *)
      printf '未知任务类型: %s\n' "${kind}"
      false
      ;;
  esac
  rc=$?
  printf '%s\n' "${rc}" > "${dir}/exit_code"
  printf '%s\n' "$(date '+%Y-%m-%dT%H:%M:%S%z')" > "${dir}/finished_at"
  if [ "${rc}" -eq 0 ]; then
    printf 'success\n' > "${dir}/state"
  else
    printf 'failed\n' > "${dir}/state"
  fi
}

task_status_json() {
  local id="$1"
  if ! task_valid_id "${id}"; then
    json_error "E_USAGE" "非法任务 ID"
    return 1
  fi
  local dir="$(task_tasks_dir)/${id}"
  if [ ! -d "${dir}" ]; then
    json_error "E_NOT_FOUND" "任务不存在: ${id}"
    return 1
  fi
  task_update_interrupted "${dir}"
  json_ok "$(task_json_for_dir "${dir}")"
}

task_logs_json() {
  local id="$1"
  local count="${2:-200}"
  case "${count}" in ''|*[!0-9]*) count=200 ;; esac
  if ! task_valid_id "${id}"; then
    json_error "E_USAGE" "非法任务 ID"
    return 1
  fi
  local dir="$(task_tasks_dir)/${id}"
  if [ ! -d "${dir}" ]; then
    json_error "E_NOT_FOUND" "任务不存在: ${id}"
    return 1
  fi
  local file="${dir}/output.log"
  printf '{"ok":true,"data":{"id":%s,"file":%s,"lines":[' "$(json_quote "${id}")" "$(json_quote "${file}")"
  if [ -f "${file}" ]; then
    tail -n "${count}" "${file}" 2>/dev/null | busybox awk '
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
      END { print "]" }
    '
  else
    printf ']'
  fi
  printf '},"meta":{"version":1}}\n'
}

task_list_json() {
  local dir items first=1
  printf '{"ok":true,"data":{"items":['
  if [ -d "$(task_tasks_dir)" ]; then
    for dir in $(busybox ls -1dt "$(task_tasks_dir)"/* 2>/dev/null); do
      [ -d "${dir}" ] || continue
      task_update_interrupted "${dir}"
      [ "${first}" -eq 1 ] || printf ','
      task_json_for_dir "${dir}"
      first=0
    done
  fi
  printf ']},"meta":{"version":1}}\n'
}

task_cancel_json() {
  local id="$1"
  if ! task_valid_id "${id}"; then
    json_error "E_USAGE" "非法任务 ID"
    return 1
  fi
  local dir="$(task_tasks_dir)/${id}"
  if [ ! -d "${dir}" ]; then
    json_error "E_NOT_FOUND" "任务不存在: ${id}"
    return 1
  fi
  task_update_interrupted "${dir}"
  if [ "$(cat "${dir}/state" 2>/dev/null)" != "running" ]; then
    json_error "E_VALIDATION" "任务已结束，无法取消"
    return 1
  fi
  if [ "$(task_read_value "${dir}" "cancelable")" != "true" ]; then
    json_error "E_VALIDATION" "该任务不可取消"
    return 1
  fi
  local pid
  pid="$(cat "${dir}/pid" 2>/dev/null)"
  case "${pid}" in
    ''|*[!0-9]*)
      json_error "E_INTERRUPTED" "任务进程不存在"
      return 1
      ;;
  esac
  printf 'canceled\n' > "${dir}/cancel_requested"
  busybox pkill -TERM -P "${pid}" >/dev/null 2>&1 || true
  kill -TERM "${pid}" >/dev/null 2>&1 || true
  sleep 1
  task_update_interrupted "${dir}"
  if [ "$(cat "${dir}/state" 2>/dev/null)" = "running" ]; then
    printf 'canceled\n' > "${dir}/state"
    printf '%s\n' "$(date '+%Y-%m-%dT%H:%M:%S%z')" > "${dir}/finished_at"
  fi
  json_ok "$(task_json_for_dir "${dir}")"
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
      
      local smart_version_tag=$(github_get "${release_page_url}" | busybox grep -oE "smart-[a-f0-9]+" | head -1)

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
        latest_version=$(github_get "${api_url}" | grep "tag_name" | busybox grep -oE "v[0-9].*" | head -1 | cut -d'"' -f1)
      else
        log Debug "正在下载 ${core_to_update} 最新稳定版本"
        latest_version=$(github_get "${api_url}/latest" | grep "tag_name" | busybox grep -oE "v[0-9].*" | head -1 | cut -d'"' -f1)
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
        latest_version=$(github_get "${api_url}" | grep "tag_name" | busybox grep -oE "v[0-9].*" | head -1 | cut -d'"' -f1)
      else
        log Debug "下载 ${core_to_update} 最新稳定版"
        latest_version=$(github_get "${api_url}/latest" | grep "tag_name" | busybox grep -oE "v[0-9.]*" | head -1)
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
        latest_version=$(github_get "https://api.github.com/repos/MetaCubeX/mihomo/releases" | grep "tag_name" | busybox grep -oE "v[0-9.]*" | head -1)
        tag="$latest_version"
      else
        if [ "$use_ghproxy" == true ]; then
          download_link="${url_ghproxy}/${download_link}"
        fi
        tag="Prerelease-Alpha"
        latest_version=$(github_get "${download_link}/expanded_assets/${tag}" | busybox grep -oE "alpha-[0-9a-z]+" | head -1)
      fi

      if [ -z "$latest_version" ]; then
        log Error "获取 mihomo 最新 稳定版/预发行版 失败"
        return 1
      fi

      filename="mihomo-${platform}-${arch}-${latest_version}"
      log Debug "下载 ${download_link}/download/${tag}/${filename}.gz"
      upfile "${box_dir}/${file_kernel}.gz" "${download_link}/download/${tag}/${filename}.gz" && xkernel "$core_to_update" "" "" "" "$file_kernel"
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
  if [[ "${bin_name}" != @(mihomo|sing-box) ]]; then
    log Debug "${bin_name} 不支持面板"
    return 1
  fi

  local ui_path
  local ui_url
  if [ "${bin_name}" = "mihomo" ]; then
    # mihomo 面板固定为 zashboard，不再读取旧 external-ui-url，避免回落到 MetacubeXD。
    ui_path="./dashboard"
    ui_url="https://github.com/Zephyruso/zashboard/releases/latest/download/dist-no-fonts.zip"
  else
    ui_path=$(busybox awk -F '"' '/"external_ui"/ {print $4}' "${sing_config}" | head -n 1)
    ui_url=$(busybox awk -F '"' '/"external_ui_download_url"/ {print $4; exit}' "${sing_config}" 2>/dev/null)
    if [ -z "${ui_url}" ]; then
      ui_url="https://github.com/Zephyruso/zashboard/releases/latest/download/dist.zip"
    fi
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

  local file_dashboard="${box_dir}/${bin_name}_dashboard.zip"
  local temp_extract_dir="${box_dir}/${bin_name}_dashboard_temp.$$"
  local stage_dir="${dashboard_dir}.stage.$$"
  local backup_dir="${dashboard_dir}.backup.$$"

  if ! upfile "${file_dashboard}" "${ui_url}"; then
    log Error "下载 Dashboard 失败"
    return 1
  fi

  rm -rf "${temp_extract_dir}" "${stage_dir}" "${backup_dir}"
  mkdir -p "${temp_extract_dir}" "${stage_dir}"

  local unzip_command="unzip"
  if ! command -v unzip >/dev/null 2>&1; then
    unzip_command="busybox unzip"
  fi

  log Info "正在解压 Dashboard..."
  if ! "${unzip_command}" -oq "${file_dashboard}" -d "${temp_extract_dir}"; then
    log Error "解压 Dashboard 失败，保留当前面板"
    rm -f "${file_dashboard}"
    rm -rf "${temp_extract_dir}" "${stage_dir}"
    return 1
  fi

  local html_path
  html_path=$(busybox find "${temp_extract_dir}" -maxdepth 3 -type f -name index.html | head -n 1)
  if [ -z "${html_path}" ]; then
    log Error "Dashboard 中未找到 index.html，保留当前面板"
    rm -f "${file_dashboard}"
    rm -rf "${temp_extract_dir}" "${stage_dir}"
    return 1
  fi
  if [ "${bin_name}" = "mihomo" ] && ! grep -qi 'zashboard' "${html_path}"; then
    log Error "下载内容不是 zashboard，保留当前面板"
    rm -f "${file_dashboard}"
    rm -rf "${temp_extract_dir}" "${stage_dir}"
    return 1
  fi

  local html_dir
  html_dir="${html_path%/*}"
  if ! cp -a "${html_dir}/." "${stage_dir}/" || [ ! -f "${stage_dir}/index.html" ]; then
    log Error "准备 Dashboard 文件失败，保留当前面板"
    rm -f "${file_dashboard}"
    rm -rf "${temp_extract_dir}" "${stage_dir}"
    return 1
  fi

  if [ -d "${dashboard_dir}" ]; then
    mv "${dashboard_dir}" "${backup_dir}" || {
      log Error "备份当前 Dashboard 失败，保留原状态"
      rm -f "${file_dashboard}"
      rm -rf "${temp_extract_dir}" "${stage_dir}"
      return 1
    }
  fi
  if ! mv "${stage_dir}" "${dashboard_dir}"; then
    log Error "替换 Dashboard 失败，恢复当前面板"
    if [ -d "${backup_dir}" ]; then
      mv "${backup_dir}" "${dashboard_dir}"
    fi
    rm -f "${file_dashboard}"
    rm -rf "${temp_extract_dir}"
    return 1
  fi
  rm -rf "${backup_dir}" "${temp_extract_dir}"
  rm -f "${file_dashboard}"

  if [ "${bin_name}" = "mihomo" ] && [ -f "${mihomo_config}" ]; then
    if grep -qE '^[[:space:]]*external-ui:' "${mihomo_config}"; then
      sed -i -E 's|^[[:space:]]*external-ui:.*|external-ui: ./dashboard|' "${mihomo_config}"
    else
      printf '\nexternal-ui: ./dashboard\n' >> "${mihomo_config}"
    fi
    sed -i '/^[[:space:]]*external-ui-url:/d' "${mihomo_config}"
  fi

  log Info "Dashboard 更新成功 → ${dashboard_dir}"
  return 0
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
  echo -e '
  <!DOCTYPE html>
  <script>
      document.location = 'http://127.0.0.1:9090/ui/'
  </script>
  </html>
  ' > $path_webroot
  sed -i "s#document\.location =.*#document.location = 'http://$ip_port/ui/'#" $path_webroot
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
  service)
    if has_json_arg "$@"; then
      service_json "$2"
    else
      case "$2" in
        reload)
          reload
          ;;
        start|stop|restart)
          "${scripts_dir}/box.service" "$2"
          ;;
        *)
          log Info "用法: $0 service {start|stop|restart|reload} [--json]"
          ;;
      esac
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
      set)
        settings_set_json "$3" "$4"
        ;;
      *)
        log Info "用法: $0 settings {get|apply <json文件>|set <key> <value>}"
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
  core)
    case "$2" in
      get)
        core_get_json
        ;;
      apply)
        core_apply_json "$3"
        ;;
      *)
        json_error "E_USAGE" "用法: $0 core {get|apply <json文件>} --json"
        ;;
    esac
    ;;
  lists)
    case "$2" in
      get)
        lists_get_json
        ;;
      apply)
        lists_apply_json "$3"
        ;;
      *)
        json_error "E_USAGE" "用法: $0 lists {get|apply <json文件>} --json"
        ;;
    esac
    ;;
  rule-profile)
    case "$2" in
      get)
        rule_profile_get_json
        ;;
      set)
        rule_profile_set_json "$3"
        ;;
      clear)
        rule_profile_clear_json
        ;;
      url)
        rule_profile_url_json
        ;;
      fetch)
        rule_profile_fetch_json
        ;;
      apply)
        if [ "$3" = "--reload" ]; then
          rule_profile_apply_json true
        else
          rule_profile_apply_json false
        fi
        ;;
      *)
        json_error "E_USAGE" "用法: $0 rule-profile {get|set <url>|url|clear|fetch|apply [--reload]} --json"
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
  task)
    case "$2" in
      start)
        if [ "$4" = "--json" ]; then
          task_start_json "$3" ""
        else
          task_start_json "$3" "$4"
        fi
        ;;
      status)
        task_status_json "$3"
        ;;
      logs)
        if [ "$4" = "--json" ]; then
          task_logs_json "$3" ""
        else
          task_logs_json "$3" "$4"
        fi
        ;;
      list)
        task_list_json
        ;;
      cancel)
        task_cancel_json "$3"
        ;;
      _run)
        task_run_internal "$3"
        ;;
      *)
        json_error "E_USAGE" "用法: $0 task {start|status|logs|list|cancel} ... --json"
        ;;
    esac
    ;;
  sub)
    case "$2" in
      add)
        if has_json_arg "$@"; then
          if [ "$4" = "--json" ]; then
            sub_add_json "$3" ""
          else
            sub_add_json "$3" "$4"
          fi
        else
          sub_add "$3" "$4"
        fi
        ;;
      get)
        sub_get_json "$3"
        ;;
      set)
        if has_json_arg "$@"; then
          sub_set_json "$3" "$4" "$5"
        else
          log Info "用法: $0 sub set <旧名字> <新名字> <订阅地址> --json"
        fi
        ;;
      list|ls)
        if [ "$3" = "--json" ]; then
          sub_list_json
        else
          sub_list
        fi
        ;;
      rm|remove|del)
        if has_json_arg "$@"; then
          sub_rm_json "$3"
        else
          sub_rm "$3"
        fi
        ;;
      update|up)
        if has_json_arg "$@"; then
          if [ "$3" = "--json" ]; then
            sub_update_json ""
          else
            sub_update_json "$3"
          fi
        else
          sub_update "$3"
        fi
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
    upkernels sing-box mihomo
    for bin_name in "${bin_list[@]}"; do
      upsubs
      upxui
    done
    ;;
  *)
    log Error "$0 $1 未找到"
    log Info "用法: $0 {status|service|settings|config|rule-profile|core|lists|logs|task|reset|sub|subs|geox|geosub|upkernel|upkernels|upgeox_all|upxui|upyq|upcurl|upcnip|reload|webroot|check|memcg|blkio|bond0|bond1|all}"
    log Info "JSON 调用统一追加 --json，返回 {ok,data,error} 包络"
    log Info "upkernel 支持的核心: sing-box, mihomo, mihomo_smart, sing-box_ref1nd"
    ;;
esac
