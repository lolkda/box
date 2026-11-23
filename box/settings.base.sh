#!/system/bin/sh
# Box for Root - static shell environment and settings runtime loader.
# User settings live in settings.json; settings.runtime.sh is generated.

# Keep the local test override; on device it is /data/adb/box.
if ! command -v busybox >/dev/null 2>&1; then
  export PATH="/data/adb/magisk:/data/adb/ksu/bin:/data/adb/ap/bin:${PATH}:/system/bin"
fi

box_dir="${BOX_HOME:-/data/adb/box}"
settings_file="${box_dir}/settings.json"
settings_json_file="${settings_file}"
settings_default_file="${box_dir}/settings.default.json"
settings_schema_file="${box_dir}/settings.schema.json"
settings_runtime_file="${box_dir}/run/settings.runtime.sh"
settings_token_file="${box_dir}/githubtoken"
settings_shell_bin="/system/bin/sh"
if [ ! -x "${settings_shell_bin}" ]; then
  if command -v bash >/dev/null 2>&1; then
    settings_shell_bin="bash"
  elif command -v mksh >/dev/null 2>&1; then
    settings_shell_bin="mksh"
  else
    settings_shell_bin="${SH:-sh}"
  fi
fi

box_run="${box_dir}/run"
box_run_state="${box_run}/state"
box_run_locks="${box_run}/locks"

mkdir -p "${box_run_state}" "${box_run_locks}" >/dev/null 2>&1 || true

box_log="${box_run}/runs.log"
box_pid="${box_run}/box.pid"
bin_dir="${box_dir}/bin"
system_packages_file="/data/system/packages.list"
uid_list="${box_run_state}/appuid.list"

# Internal constants. These are deliberately not user settings.
box_user_group="root:net_admin"
bin_list=("mihomo" "sing-box")
name_mihomo_config="config.yaml"
name_sing_config="config.json"
subscriptions_file="${box_dir}/mihomo/subscriptions.list"
mihomo_provide_path="${box_dir}/mihomo/proxy_provider"
cn_ip_file="${box_run_state}/cn_ip.txt"
cn_ipv6_file="${box_run_state}/cn_ipv6.txt"
write_listap="${box_dir}/ap.list.cfg"
pkg_config="${box_dir}/package.list.cfg"
gid_config="${box_dir}/gid.list.cfg"

settings_scalar_keys() {
  cat <<'KEYS'
auto_modify_config boot_auto_start bin_name network_mode tproxy_port redir_port ipv6 performance_mode quic mihomo_dns_forward proxy_tcp proxy_udp dns_hijack_tcp dns_hijack_udp dns_hijack_mode bypass_cn_ip bypass_cn_ip_v4 bypass_cn_ip_v6 cnip_mode cn_ip_url cn_ipv6_url proxy_mode update_subscription update_geo use_ghproxy url_ghproxy mihomo_stable singbox_stable subscription_url_singbox run_crontab interva_update cgroup_memcg memcg_limit taskset_cpu allow_cpu cgroup_blkio weight mac_filter mac_mode ruleset_prefix subscription_keep_last_good enable_network_service_control use_module_on_wifi use_module_on_wifi_disconnect use_ssid_matching use_wifi_list_mode inotify_log_enabled subscription_mode
KEYS
}

settings_array_keys() {
  cat <<'KEYS'
cnip_force_proxy_apps force_proxy_cidrs force_proxy_cidrs6 macs_list wifi_ssids_list wifi_bssids_list
KEYS
}

settings_allowed_keys() {
  settings_scalar_keys
  settings_array_keys
  printf 'githubtoken\n'
}

settings_apply_derived() {
  bin_path="${bin_dir}/${bin_name}"
  bin_log="${box_run}/${bin_name}.log"
  mihomo_config="${box_dir}/mihomo/${name_mihomo_config}"
  sing_config="${box_dir}/sing-box/${name_sing_config}"

  [ ! -f "${write_listap}" ] && touch "${write_listap}"
  [ ! -f "${pkg_config}" ] && touch "${pkg_config}"
  [ ! -f "${gid_config}" ] && touch "${gid_config}"

  prefixed_list=($(sed -n '/^[0-9][0-9]*:/p' "${pkg_config}"))
  unprefixed_list_converted=($(sed -n '/\./p' "${pkg_config}" | grep -v -E '^[0-9]+:|^[0-9]+$|^[[:space:]]*#' | sed 's/^/0:/'))
  packages_list=("${prefixed_list[@]}" "${unprefixed_list_converted[@]}")
  gid_list=($(busybox awk '!/^#/ && /^[0-9]+$/ {print $1}' "${gid_config}"))

  ap_list=($(busybox awk '/^allow / {print $2}' "${write_listap}"))
  ignore_ap_list=($(busybox awk '/^ignore / {print $2}' "${write_listap}"))
}

settings_shell_quote() {
  local escaped
  escaped="$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
  printf "'%s'" "${escaped}"
}

settings_render_runtime_file() {
  local json_file="$1"
  local output="$2"
  local token_file="$3"
  local default_file="$4"
  local allowed token

  [ -f "${default_file}" ] || default_file="${settings_json_file}"
  [ -f "${default_file}" ] || {
    printf 'settings: default settings.json not found\n' >&2
    return 1
  }
  [ -f "${json_file}" ] || {
    printf 'settings: settings.json not found\n' >&2
    return 1
  }
  [ -f "${box_dir}/scripts/settings-gen.awk" ] || {
    printf 'settings: settings-gen.awk not found\n' >&2
    return 1
  }

  allowed=" $(settings_allowed_keys | tr '\n' ' ') "
  if ! busybox awk -v allowed="${allowed}" -f "${box_dir}/scripts/settings-gen.awk" \
      "${default_file}" "${json_file}" > "${output}" 2>/dev/null; then
    rm -f "${output}"
    printf 'settings: settings.json is invalid\n' >&2
    return 1
  fi

  token=""
  if [ -n "${token_file}" ] && [ -f "${token_file}" ] && [ -s "${token_file}" ]; then
    token="$(cat "${token_file}")"
  fi
  printf 'githubtoken=%s\n' "$(settings_shell_quote "${token}")" >> "${output}" || {
    rm -f "${output}"
    return 1
  }

  if ! "${settings_shell_bin}" -n "${output}" >/dev/null 2>&1; then
    rm -f "${output}"
    printf 'settings: generated runtime has invalid shell syntax\n' >&2
    return 1
  fi
}

settings_generate_runtime() {
  local generated="${settings_runtime_file}.tmp.$$"
  local default_file="${settings_default_file}"

  [ -f "${default_file}" ] || default_file="${settings_json_file}"
  mkdir -p "${settings_runtime_file%/*}" >/dev/null 2>&1 || true
  settings_render_runtime_file "${settings_json_file}" "${generated}" "${settings_token_file}" "${default_file}" || return 1
  mv -f "${generated}" "${settings_runtime_file}" || {
    rm -f "${generated}"
    return 1
  }
  chmod 0600 "${settings_runtime_file}" >/dev/null 2>&1 || true
}

settings_ensure_runtime() {
  if [ -s "${settings_runtime_file}" ] && "${settings_shell_bin}" -n "${settings_runtime_file}" >/dev/null 2>&1; then
    return 0
  fi
  settings_generate_runtime
}

settings_load() {
  settings_ensure_runtime || return 1
  . "${settings_runtime_file}" || return 1
  settings_apply_derived
}

append_box_log_line() {
  local target_file="$1"
  local target_dir
  shift
  [ -n "${target_file}" ] || return 0
  target_dir="${target_file%/*}"
  if [ "${target_dir}" != "${target_file}" ]; then
    mkdir -p "${target_dir}" >/dev/null 2>&1 || true
  fi
  printf '%s\n' "$*" >> "${target_file}" 2>/dev/null || true
}

log() {
  local normal="\033[0m"
  local red="\033[1;31m"
  local green="\033[1;32m"
  local yellow="\033[1;33m"
  local blue="\033[1;34m"
  local color
  local level="$1"
  shift
  local timestamp
  local raw_message
  local log_line
  local message

  timestamp="$(date '+%Y-%m-%d %H:%M:%S')"
  raw_message="$*"

  case "${level}" in
    Info) color="${blue}" ;;
    Error) color="${red}" ;;
    Warn|Warning) level="Warn"; color="${yellow}" ;;
    *) color="${green}" ;;
  esac

  while IFS= read -r log_line || [ -n "${log_line}" ]; do
    message="${timestamp} [${level}]: ${log_line}"
    append_box_log_line "${box_log}" "${message}"

    if [ "${BOX_JSON_MODE:-0}" = "1" ]; then
      printf '%s\n' "${message}" >&2
    elif [ -t 1 ]; then
      printf '%b\n' "${color}${message}${normal}"
    else
      printf '%s\n' "${message}"
    fi
  done <<EOF2
${raw_message}
EOF2

  if [ -n "${TOAST:-}" ]; then
    true
  fi
}
