#!/system/bin/sh

(
  until [ "$(getprop init.svc.bootanim)" = "stopped" ]; do
    sleep 10
  done

  box_data_dir="/data/adb/box"
  boxctl_bin="${box_data_dir}/bin/boxctl"
  stop_file="${box_data_dir}/run/boxctl/daemon.stop"
  mkdir -p "${box_data_dir}/run" 2>/dev/null || true
  rm -f "${stop_file}" 2>/dev/null || true

  if [ -x "${boxctl_bin}" ]; then
    # Restart with bounded backoff: uninstall writes daemon.stop so the supervisor
    # exits before its binary/hook is removed.
    delay=3
    failures=0
    failed_file="${box_data_dir}/run/boxctl/daemon.failed"
    rm -f "${failed_file}" 2>/dev/null || true
    daemon_pid_file="${box_data_dir}/run/boxctl/daemon.pid"
    while [ -x "${boxctl_bin}" ] && [ ! -f "${stop_file}" ]; do
      if [ -f "${daemon_pid_file}" ]; then
        existing_pid=$(cat "${daemon_pid_file}" 2>/dev/null | tr -cd '0-9')
        if [ -n "${existing_pid}" ] && [ -e "/proc/${existing_pid}/exe" ]; then
          # Do not fight an externally started daemon for the flock.
          sleep 30
          continue
        fi
      fi
      started_at=$(date +%s 2>/dev/null || echo 0)
      export BOXCTL_BOOT_AUTOSTART=1
      "${boxctl_bin}" --root "${box_data_dir}" daemon >> "${box_data_dir}/run/boxctl.log" 2>&1
      [ -f "${stop_file}" ] && break
      ended_at=$(date +%s 2>/dev/null || echo 0)
      if [ "${started_at}" -gt 0 ] && [ "${ended_at}" -gt 0 ] && [ $((ended_at - started_at)) -ge 60 ]; then
        delay=3
        failures=0
      fi
      failures=$((failures + 1))
      if [ "${failures}" -ge 10 ]; then
        echo "boxctl daemon exited ${failures} times; supervision stopped" > "${failed_file}"
        break
      fi
      sleep "${delay}"
      if [ "${delay}" -lt 60 ]; then
        delay=$((delay * 2))
        [ "${delay}" -gt 60 ] && delay=60
      fi
    done
  else
    echo "未找到 boxctl: ${boxctl_bin}"
  fi
)&
