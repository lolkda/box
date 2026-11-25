#!/system/bin/sh

# Ordinary module uninstall preserves persistent Box data.
# Destructive deletion is only available through `boxctl purge.plan/purge.apply`
# with an explicit token and confirmation.
box_data_dir="/data/adb/box"
boxctl_bin="${box_data_dir}/bin/boxctl"
boxctl_socket="${box_data_dir}/run/boxctl/control.sock"
boxctl_temp_pid=""

mkdir -p "${box_data_dir}/run/boxctl" 2>/dev/null || true
: > "${box_data_dir}/run/boxctl/daemon.stop" 2>/dev/null || true

if [ -x "${boxctl_bin}" ]; then
  if [ ! -S "${boxctl_socket}" ]; then
    mkdir -p "${box_data_dir}/run" 2>/dev/null || true
    "${boxctl_bin}" --root "${box_data_dir}" daemon >> "${box_data_dir}/run/boxctl-uninstall.log" 2>&1 &
    boxctl_temp_pid=$!
    attempt=0
    while [ "${attempt}" -lt 10 ]; do
      [ -S "${boxctl_socket}" ] && break
      sleep 1
      attempt=$((attempt + 1))
    done
  fi
  if [ -S "${boxctl_socket}" ]; then
    "${boxctl_bin}" --root "${box_data_dir}" service stop --json >/dev/null 2>&1 || true
  fi
  if [ -n "${boxctl_temp_pid}" ]; then
    kill "${boxctl_temp_pid}" >/dev/null 2>&1 || true
    wait "${boxctl_temp_pid}" 2>/dev/null || true
  fi
fi

# Stop a pre-existing management daemon so no removed binary keeps running.
daemon_pid_file="${box_data_dir}/run/boxctl/daemon.pid"
if [ -f "${daemon_pid_file}" ]; then
  daemon_pid=$(cat "${daemon_pid_file}" 2>/dev/null | tr -cd '0-9')
  if [ -n "${daemon_pid}" ] && [ "${daemon_pid}" -gt 1 ] 2>/dev/null; then
    daemon_cmdline=$(tr '\0' ' ' < "/proc/${daemon_pid}/cmdline" 2>/dev/null || true)
    case "${daemon_cmdline}" in
      *boxctl*daemon*)
        kill -TERM "${daemon_pid}" >/dev/null 2>&1 || true
        attempt=0
        while [ "${attempt}" -lt 5 ] && kill -0 "${daemon_pid}" >/dev/null 2>&1; do
          sleep 1
          attempt=$((attempt + 1))
        done
        ;;
      *)
        : # stale or foreign pid; leave it alone
        ;;
    esac
  fi
fi

# Remove startup hooks; the Root manager removes the module directory itself.
rm -f "/data/adb/ksu/service.d/box_service.sh" 2>/dev/null || true
rm -f "/data/adb/service.d/box_service.sh" 2>/dev/null || true

# Drop only transient runtime state. Settings, token, config, subscriptions,
# rule profiles, lists, logs and the core binary are intentionally kept.
rm -rf "${box_data_dir}/run/boxctl" 2>/dev/null || true
rm -f "${box_data_dir}/run/box.pid" 2>/dev/null || true

echo "Box 模块已卸载；持久数据保留在 ${box_data_dir}，如需彻底删除请使用 boxctl purge。"
exit 0
