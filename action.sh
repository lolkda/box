#!/system/bin/sh

box_dir="/data/adb/box"
box_pid="${box_dir}/run/box.pid"
boxctl="${box_dir}/bin/boxctl"

if [ -x "${boxctl}" ]; then
  if [ -f "${box_pid}" ] && kill -0 "$(cat "${box_pid}" 2>/dev/null)" 2>/dev/null; then
    echo "服务正在关闭..."
    "${boxctl}" --root "${box_dir}" service stop --json
  else
    echo "服务正在启动，请稍候..."
    "${boxctl}" --root "${box_dir}" service start --json
  fi
  exit $?
fi

echo "boxctl 不存在，无法执行模块操作"
exit 1
