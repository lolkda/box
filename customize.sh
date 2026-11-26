#!/system/bin/sh

SKIPUNZIP=1
SKIPMOUNT=false
PROPFILE=true
POSTFSDATA=false
LATESTARTSERVICE=true

if [ "$BOOTMODE" != true ]; then
  abort "-----------------------------------------------------------"
  ui_print "! 请在 Magisk/KernelSU/APatch Manager 中安装本模块"
  ui_print "! 不支持从 Recovery 安装"
  abort "-----------------------------------------------------------"
elif [ "$KSU" = true ] && [ "$KSU_VER_CODE" -lt 10670 ]; then
  abort "-----------------------------------------------------------"
  ui_print "! 请升级您的 KernelSU 及其管理器"
  abort "-----------------------------------------------------------"
fi

service_dir="/data/adb/service.d"
if [ "$KSU" = "true" ]; then
  ui_print "- 检测到 KernelSU 版本: $KSU_VER ($KSU_VER_CODE)"
  [ "$KSU_VER_CODE" -lt 10683 ] && service_dir="/data/adb/ksu/service.d"
elif [ "$APATCH" = "true" ]; then
  APATCH_VER=$(cat "/data/adb/ap/version")
  ui_print "- 检测到 APatch 版本: $APATCH_VER"
else
  ui_print "- 检测到 Magisk 版本: $MAGISK_VER ($MAGISK_VER_CODE)"
fi

mkdir -p "${service_dir}"

BOXCTL_BIN="/data/adb/box/bin/boxctl"
BOXCTL_SOCKET="/data/adb/box/run/boxctl/control.sock"
BOXCTL_TEMP_PID=""

boxctl_install_daemon_start() {
  [ -x "${BOXCTL_BIN}" ] || return 1
  if [ -S "${BOXCTL_SOCKET}" ]; then
    return 0
  fi
  mkdir -p /data/adb/box/run
  "${BOXCTL_BIN}" --root /data/adb/box daemon >> /data/adb/box/run/boxctl-install.log 2>&1 &
  BOXCTL_TEMP_PID=$!
  local attempt=0
  while [ "${attempt}" -lt 10 ]; do
    [ -S "${BOXCTL_SOCKET}" ] && return 0
    sleep 1
    attempt=$((attempt + 1))
  done
  return 1
}

boxctl_install_daemon_stop() {
  if [ -n "${BOXCTL_TEMP_PID}" ]; then
    kill "${BOXCTL_TEMP_PID}" >/dev/null 2>&1 || true
    wait "${BOXCTL_TEMP_PID}" 2>/dev/null || true
    BOXCTL_TEMP_PID=""
  fi
}

boxctl_install_stop_existing() {
  [ -x "${BOXCTL_BIN}" ] || return 0
  if [ -f "/data/adb/box/run/boxctl/daemon.pid" ]; then
    existing_pid=$(cat "/data/adb/box/run/boxctl/daemon.pid" 2>/dev/null | tr -cd '0-9')
    if [ -n "${existing_pid}" ] && [ "${existing_pid}" -gt 1 ] 2>/dev/null; then
      : > "/data/adb/box/run/boxctl/daemon.stop" 2>/dev/null || true
      kill -TERM "${existing_pid}" >/dev/null 2>&1 || true
      attempt=0
      while [ "${attempt}" -lt 5 ] && kill -0 "${existing_pid}" >/dev/null 2>&1; do
        sleep 1
        attempt=$((attempt + 1))
      done
    fi
  fi
}
if [ -d "/data/adb/modules/box_for_magisk" ]; then
  rm -rf "/data/adb/modules/box_for_magisk"
  ui_print "- 已删除旧模块。"
fi

ui_print "- 正在安装 Box for Magisk/KernelSU/APatch"
unzip -o "$ZIPFILE" -x 'META-INF/*' -x 'webroot/*' -d "$MODPATH" >&2

# 先把构建时内置的 zashboard 暂存到安装临时目录，避免恢复旧 mihomo 数据时被覆盖。
if [ ! -d "$MODPATH/box/mihomo/dashboard" ] || [ ! -f "$MODPATH/box/mihomo/dashboard/index.html" ]; then
  ui_print "! 模块包缺少内置 zashboard，安装中止"
  abort "-----------------------------------------------------------"
fi
ZASHBOARD_STAGE="$MODPATH/zashboard"
mkdir -p "$ZASHBOARD_STAGE"
cp -a "$MODPATH/box/mihomo/dashboard/." "$ZASHBOARD_STAGE/"

# 保存新方案的默认 mihomo 配置，供旧 renew 设置首次升级时重置使用。
DEFAULT_CONFIG_STAGE="$MODPATH/default-config.yaml"
cp -f "$MODPATH/box/mihomo/config.yaml" "$DEFAULT_CONFIG_STAGE" 2>/dev/null || true

boxctl_install_stop_existing

legacy_settings="false"
if [ -d "/data/adb/box" ]; then
  ui_print "- 备份现有 box 数据"
  temp_bak=$(mktemp -d -p "/data/adb/box" box.XXXXXXXXXX)
  temp_dir="${temp_bak}"
  mv /data/adb/box/* "${temp_dir}/"
  mv "$MODPATH/box/"* /data/adb/box/
  backup_box="true"

  if [ -f "${temp_dir}/settings.json" ] && busybox grep -q '"renew"' "${temp_dir}/settings.json" 2>/dev/null; then
    legacy_settings="true"
    ui_print "- 检测到旧 renew 设置，首次升级将重置 settings.json 和 mihomo/config.yaml"
  elif [ -f "${temp_dir}/settings.json" ]; then
    cp -f "${temp_dir}/settings.json" /data/adb/box/settings.json 2>/dev/null || true
  else
    legacy_settings="true"
  fi
else
  mv "$MODPATH/box" /data/adb/
fi

ui_print "- 创建目录"
mkdir -p /data/adb/box/ /data/adb/box/run/ /data/adb/box/bin/
[ -f /data/adb/box/bin/boxctl ] && chmod 0755 /data/adb/box/bin/boxctl

ui_print "- 提取 uninstall.sh 和 box_service.sh"
unzip -j -o "$ZIPFILE" 'uninstall.sh' -d "$MODPATH" >&2
unzip -j -o "$ZIPFILE" 'box_service.sh' -d "${service_dir}" >&2

ui_print "- 设置权限"
set_perm_recursive $MODPATH 0 0 0755 0644
set_perm_recursive /data/adb/box/ 0 3005 0755 0644
set_perm /data/adb/box/bin/boxctl 0 0 0755
[ -f /data/adb/box/bin/yq ] && set_perm /data/adb/box/bin/yq 0 0 0755
[ -f /data/adb/box/bin/curl ] && set_perm /data/adb/box/bin/curl 0 0 0755
[ -f /data/adb/box/settings.json ] && set_perm /data/adb/box/settings.json 0 0 0600
[ -f /data/adb/box/githubtoken ] && set_perm /data/adb/box/githubtoken 0 0 0600
[ -f /data/adb/box/run/settings.runtime.sh ] && set_perm /data/adb/box/run/settings.runtime.sh 0 0 0600
set_perm ${service_dir}/box_service.sh 0 0 0755
set_perm $MODPATH/uninstall.sh 0 0 0755
chmod ugo+x ${service_dir}/box_service.sh $MODPATH/uninstall.sh
# 旧 shell 已封存到仓库 legacy/，升级时清理设备上的活动副本。
rm -rf /data/adb/box/scripts /data/adb/box/settings.base.sh /data/adb/box/run/settings.runtime.sh

KEY_LISTENER_PID=""
KEY_FIFO=""

start_key_listener() {
    if [ -n "$KEY_LISTENER_PID" ] && kill -0 "$KEY_LISTENER_PID" 2>/dev/null; then
        return
    fi
    KEY_FIFO=$(mktemp -u -p /dev/tmp)
    mkfifo "$KEY_FIFO" || exit 1
    getevent -ql > "$KEY_FIFO" &
    KEY_LISTENER_PID=$!
}

stop_key_listener() {
    if [ -n "$KEY_LISTENER_PID" ]; then
        kill "$KEY_LISTENER_PID" >/dev/null 2>&1
        KEY_LISTENER_PID=""
    fi
    if [ -n "$KEY_FIFO" ]; then
        rm -f "$KEY_FIFO"
        KEY_FIFO=""
    fi
}

volume_key_detection() {
    local timeout_seconds="${1:-0}"
    local detection_result_file=$(mktemp -u -p /dev/tmp)
    
    (
        while read -r line; do
            if echo "$line" | grep -Eiq "(KEY_)?VOLUME ?UP|KEYCODE_VOLUME_UP" && echo "$line" | grep -Eiq "DOWN|PRESS"; then
                echo "0" > "$detection_result_file"
                exit 0
            elif echo "$line" | grep -Eiq "(KEY_)?VOLUME ?DOWN|KEYCODE_VOLUME_DOWN" && echo "$line" | grep -Eiq "DOWN|PRESS"; then
                echo "1" > "$detection_result_file"
                exit 0
            fi
        done < "$KEY_FIFO"
    ) &
    local detection_pid=$!
    
    if [ "$timeout_seconds" -gt 0 ]; then
        (
            sleep "$timeout_seconds"
            if kill -0 "$detection_pid" 2>/dev/null; then
                kill "$detection_pid" 2>/dev/null
                echo "2" > "$detection_result_file"
            fi
        ) &
        local timeout_pid=$!
        
        wait "$detection_pid" 2>/dev/null
        kill "$timeout_pid" 2>/dev/null
        wait "$timeout_pid" 2>/dev/null
    else
        wait "$detection_pid" 2>/dev/null
    fi
    
    if [ -f "$detection_result_file" ]; then
        local result=$(cat "$detection_result_file")
        rm -f "$detection_result_file"
        return "$result"
    fi
    
    rm -f "$detection_result_file"
    return 2
}

handle_choice() {
    local question="$1"
    if [ -f "/data/adb/box_install_noninteractive" ]; then
        ui_print " "
        ui_print "- 非交互安装：跳过可选下载/确认"
        return 1
    fi
    local choice_yes="${2:-是}"
    local choice_no="${3:-否}"
    local timeout_seconds="${4:-10}"

    ui_print " "
    ui_print "-----------------------------------------------------------"
    ui_print "- ${question}"
    ui_print "- [ 音量加(+) ]: ${choice_yes}"
    ui_print "- [ 音量减(-) ]: ${choice_no}"
    ui_print "- [ ${timeout_seconds}秒内未选择将默认选择: ${choice_yes} ]"

    timeout 0.1 getevent -c 1 >/dev/null 2>&1

    start_key_listener
    volume_key_detection "$timeout_seconds"
    local result=$?
    stop_key_listener
    
    if [ "$result" -eq 0 ]; then
        ui_print "  => 您选择了: ${choice_yes}"
        return 0
    elif [ "$result" -eq 1 ]; then
        ui_print "  => 您选择了: ${choice_no}"
        return 1
    else
        ui_print "  => 超时未选择，默认选择: ${choice_yes}"
        return 0
    fi
}

ui_print " "
ui_print "==========================================================="
ui_print "==         Box for Magisk/KernelSU/APatch 安装程序         =="
ui_print "==========================================================="


if handle_choice "是否需要下载内核或数据文件？" "是，进行下载" "否，全部跳过"; then

    if handle_choice "是否使用镜像加速接下来的下载？" "使用加速" "直接下载"; then
        ui_print "- 已启用镜像加速。"
        if [ -x "/data/adb/box/bin/boxctl" ]; then
            /data/adb/box/bin/boxctl --root /data/adb/box settings set use_ghproxy true --json >/dev/null 2>&1 || true
        else
            ui_print "  ! legacy box.tool is sealed; use boxctl"
        fi
    else
        ui_print "- 已禁用镜像加速。"
        if [ -x "/data/adb/box/bin/boxctl" ]; then
            /data/adb/box/bin/boxctl --root /data/adb/box settings set use_ghproxy false --json >/dev/null 2>&1 || true
        else
            ui_print "  ! legacy box.tool is sealed; use boxctl"
        fi
    fi

    COMPONENTS_TO_DOWNLOAD=""

    if handle_choice "是否需要自定义下载内容？" "自定义" "一键下载所有组件"; then
        ui_print "- 进入自定义下载..."
        if handle_choice "是否下载 GeoX 数据文件 (geoip/geosite)？" "下载" "跳过"; then
            COMPONENTS_TO_DOWNLOAD="$COMPONENTS_TO_DOWNLOAD geox"
        fi
        if handle_choice "是否下载实用工具 (yq, curl)？" "下载" "跳过"; then
            COMPONENTS_TO_DOWNLOAD="$COMPONENTS_TO_DOWNLOAD utils"
        fi
        
        ui_print " "
        ui_print "-----------------------------------------------------------"
        ui_print "- 请选择您需要下载的内核:"
        if handle_choice "  - 下载 mihomo 内核？" "下载" "跳过"; then
            COMPONENTS_TO_DOWNLOAD="$COMPONENTS_TO_DOWNLOAD mihomo"
        fi
        if handle_choice "  - 下载 mihomo_smart (带Smart策略组) 内核？（与mihomo冲突，请勿同时下载）" "下载" "跳过"; then
            COMPONENTS_TO_DOWNLOAD="$COMPONENTS_TO_DOWNLOAD mihomo_smart"
        fi
    else
        ui_print "- 已选择一键下载所有组件。"
        COMPONENTS_TO_DOWNLOAD="geox utils mihomo"
    fi

    ui_print " "
    ui_print "==========================================================="
    ui_print "- 下载任务预览"
    ui_print "-----------------------------------------------------------"
    
    if [ -z "$COMPONENTS_TO_DOWNLOAD" ]; then
        ui_print "  - 无任何下载任务。"
    else
        COMPONENTS_TO_DOWNLOAD=$(echo "$COMPONENTS_TO_DOWNLOAD" | sed 's/^ *//')
        ui_print "  - 将要下载: ${COMPONENTS_TO_DOWNLOAD}"
    fi
    ui_print "==========================================================="

    if [ -n "$COMPONENTS_TO_DOWNLOAD" ]; then
        if handle_choice "是否开始执行以上下载任务？" "开始下载" "取消全部"; then
            ui_print "- 开始执行下载..."
            for component in $COMPONENTS_TO_DOWNLOAD; do
              case "$component" in
                geox)
                  ui_print "  -> 正在下载 GeoX..."
                  if boxctl_install_daemon_start; then
                    if ! "${BOXCTL_BIN}" --root /data/adb/box task start geox --wait --json >/dev/null 2>&1; then
                      ui_print "  ! Rust GeoX 更新失败，回退旧脚本"
                      ui_print "  ! legacy box.tool is sealed; use boxctl"
                    fi
                  else
                    ui_print "  ! legacy box.tool is sealed; use boxctl"
                  fi
                  ;;
                utils)
                  ui_print "  -> 正在下载 yq..."
                  if boxctl_install_daemon_start; then
                    if ! "${BOXCTL_BIN}" --root /data/adb/box task start upyq --wait --json >/dev/null 2>&1; then
                      ui_print "  ! Rust yq 更新失败，回退旧脚本"
                      ui_print "  ! legacy box.tool is sealed; use boxctl"
                    fi
                  else
                    ui_print "  ! legacy box.tool is sealed; use boxctl"
                  fi
                  ui_print "  -> 正在下载 curl..."
                  if boxctl_install_daemon_start; then
                    if ! "${BOXCTL_BIN}" --root /data/adb/box task start upcurl --wait --json >/dev/null 2>&1; then
                      ui_print "  ! Rust curl 更新失败，回退旧脚本"
                      ui_print "  ! legacy box.tool is sealed; use boxctl"
                    fi
                  else
                    ui_print "  ! legacy box.tool is sealed; use boxctl"
                  fi
                  ;;
                *)
                  ui_print "  -> 正在下载内核: $component..."
                  if boxctl_install_daemon_start; then
                    if ! "${BOXCTL_BIN}" --root /data/adb/box task start upkernel "$component" --wait --json >/dev/null 2>&1; then
                      ui_print "  ! Rust 内核更新失败，回退旧脚本"
                      ui_print "  ! legacy box.tool is sealed; use boxctl"
                    fi
                  else
                    ui_print "  ! legacy box.tool is sealed; use boxctl"
                  fi
                  ;;
              esac
            done
            ui_print "- 所有下载任务已完成！"
        else
            ui_print "- 已取消所有下载任务。"
        fi
    fi
else
    ui_print "- 已跳过所有下载步骤。"
fi


if [ "${backup_box}" = "true" ]; then
  ui_print " "
  ui_print "- 正在恢复用户配置和数据..."

  if [ -f "${temp_dir}/settings.ini" ] || [ -f "/data/adb/box/settings.ini" ]; then
    ui_print "  - 旧 settings.ini 不迁移，升级后使用默认 settings.json"
    rm -f "/data/adb/box/settings.ini" 2>/dev/null || true
  fi

  restore_config_dir() {
    config_dir="$1"
    if [ -d "${temp_dir}/${config_dir}" ]; then
        ui_print "  - 恢复 ${config_dir} 目录配置"
        cp -af "${temp_dir}/${config_dir}/." "/data/adb/box/${config_dir}/"
    fi
  }
  restore_config_dir mihomo

  if [ "${legacy_settings}" = "true" ] && [ -f "${DEFAULT_CONFIG_STAGE}" ]; then
    ui_print "  - 首次升级：重置 mihomo/config.yaml 为默认配置"
    cp -f "${DEFAULT_CONFIG_STAGE}" "/data/adb/box/mihomo/config.yaml" 2>/dev/null || true
  fi

  ui_print "  - 恢复根目录配置文件"
  for conf_file in ap.list.cfg package.list.cfg gid.list.cfg crontab.cfg; do
    if [ -f "${temp_dir}/${conf_file}" ]; then
      cp -f "${temp_dir}/${conf_file}" "/data/adb/box/${conf_file}"
    fi
  done

  restore_binary() {
    local bin_path_fragment="$1"
    local target_path="/data/adb/box/bin/${bin_path_fragment}"
    local backup_path="${temp_dir}/bin/${bin_path_fragment}"

    if [ ! -f "${target_path}" ] && [ -f "${backup_path}" ]; then
      ui_print "  - 恢复二进制文件: ${bin_path_fragment}"
      mkdir -p "$(dirname "${target_path}")"
      cp -f "${backup_path}" "${target_path}"
      chmod 755 "${target_path}"
    fi
  }
  for bin_item in curl yq mihomo; do
    restore_binary "$bin_item"
  done

  if [ -d "${temp_dir}/run" ]; then
    ui_print "  - 恢复日志、pid等运行时文件"
    cp -af "${temp_dir}/run/." "/data/adb/box/run/"
    rm -f "/data/adb/box/run/settings.runtime.sh" 2>/dev/null || true
  fi

  if [ -d "${temp_dir}/state" ]; then
    ui_print "  - 恢复持久状态目录"
    mkdir -p "/data/adb/box/state"
    cp -af "${temp_dir}/state/." "/data/adb/box/state/"
  fi

  if [ -f "${temp_dir}/githubtoken" ]; then
    ui_print "  - 恢复 GitHub Token"
    cp -f "${temp_dir}/githubtoken" "/data/adb/box/githubtoken"
    chmod 600 "/data/adb/box/githubtoken" 2>/dev/null || true
  fi
fi

# 旧 settings/renew 迁移交由 Rust 准备步骤：先备份，再重置并写升级确认门槛。
if [ "${legacy_settings}" = "true" ] && [ -x "/data/adb/box/bin/boxctl" ]; then
  ui_print "- 正在执行 Rust 升级准备（备份 settings/token/旧名单）"
  if /data/adb/box/bin/boxctl --root /data/adb/box internal upgrade-prepare --json >/dev/null 2>&1; then
    ui_print "  - 备份与重置完成；启动前需在 WebUI 确认升级"
  else
    ui_print "  ! Rust 升级准备失败，保留现有设置"
  fi
fi

# 所有安装路径都强制使用模块内置 zashboard，并清理旧面板路径/下载地址。
ui_print "- 部署内置 zashboard"
rm -rf "/data/adb/box/mihomo/dashboard"
mkdir -p "/data/adb/box/mihomo/dashboard"
cp -a "${ZASHBOARD_STAGE}/." "/data/adb/box/mihomo/dashboard/"
if [ -f "/data/adb/box/mihomo/config.yaml" ]; then
  if grep -qE '^[[:space:]]*external-ui:' "/data/adb/box/mihomo/config.yaml"; then
    sed -i -E 's|^[[:space:]]*external-ui:.*|external-ui: ./dashboard|' "/data/adb/box/mihomo/config.yaml"
  else
    printf '\nexternal-ui: ./dashboard\n' >> "/data/adb/box/mihomo/config.yaml"
  fi
  sed -i '/^[[:space:]]*external-ui-url:/d' "/data/adb/box/mihomo/config.yaml"
fi

# 2.0.0：完全移除 sing-box 支持，升级时按确认直接清理旧核心与数据
ui_print "- 清理已移除的 sing-box 核心与数据"
busybox pkill -15 -f "/data/adb/box/bin/sing-box" >/dev/null 2>&1 || true
rm -rf "/data/adb/box/sing-box" 2>/dev/null || ui_print "  ! 无法删除 /data/adb/box/sing-box"
rm -f "/data/adb/box/bin/sing-box" "/data/adb/box/bin/backup/sing-box.bak" 2>/dev/null || true
if [ -n "${temp_dir}" ] && [ -d "${temp_dir}" ]; then
  rm -rf "${temp_dir}/sing-box" 2>/dev/null || true
  rm -f "${temp_dir}/bin/sing-box" "${temp_dir}/bin/backup/sing-box.bak" 2>/dev/null || true
fi

# 迁移旧 settings.json：删除已移除的 sing-box 设置键
if [ -f "/data/adb/box/settings.json" ]; then
  cp -f "/data/adb/box/settings.json" "/data/adb/box/settings.json.pre-2.0.0" 2>/dev/null || true
  if command -v jq >/dev/null 2>&1; then
    jq 'del(.bin_name, .singbox_stable, .subscription_url_singbox)' "/data/adb/box/settings.json" > "/data/adb/box/settings.json.tmp" 2>/dev/null && mv -f "/data/adb/box/settings.json.tmp" "/data/adb/box/settings.json" || rm -f "/data/adb/box/settings.json.tmp"
  elif [ -x "/data/adb/box/bin/yq" ]; then
    /data/adb/box/bin/yq -o=json -i 'del(.bin_name, .singbox_stable, .subscription_url_singbox)' "/data/adb/box/settings.json" 2>/dev/null || true
  else
    busybox sed -i -E '/^[[:space:]]*"(bin_name|singbox_stable|subscription_url_singbox)"[[:space:]]*:/d' "/data/adb/box/settings.json" 2>/dev/null || true
  fi
fi

# 旧核心已移除：清理旧二进制与配置目录，并迁移非法 bin_name
ui_print "- 清理已移除的 xray / v2fly / hysteria 核心"
for legacy_bin in xray v2fly hysteria; do
  busybox pkill -15 -f "/data/adb/box/bin/${legacy_bin}" >/dev/null 2>&1 || true
done
for legacy_dir in xray v2fly hysteria; do
  rm -rf "/data/adb/box/${legacy_dir}" 2>/dev/null || ui_print "  ! 无法删除 /data/adb/box/${legacy_dir}"
done
for legacy_bin in xray v2fly hysteria; do
  rm -f "/data/adb/box/bin/${legacy_bin}" 2>/dev/null || true
  rm -f "/data/adb/box/bin/backup/${legacy_bin}.bak" 2>/dev/null || true
done
if [ -n "${temp_dir}" ] && [ -d "${temp_dir}" ]; then
  for legacy_dir in xray v2fly hysteria; do
    rm -rf "${temp_dir}/${legacy_dir}" 2>/dev/null || true
  done
  for legacy_bin in xray v2fly hysteria; do
    rm -f "${temp_dir}/bin/${legacy_bin}" 2>/dev/null || true
  done
fi
# 安装/升级后确保默认 JSON 与运行时文件存在。
if [ ! -f "/data/adb/box/settings.default.json" ] && [ -f "/data/adb/box/settings.json" ]; then
  cp -f "/data/adb/box/settings.json" "/data/adb/box/settings.default.json" 2>/dev/null || true
fi
boxctl_install_daemon_start >/dev/null 2>&1 || true
if [ -x "/data/adb/box/bin/boxctl" ]; then
  /data/adb/box/bin/boxctl --root /data/adb/box settings get --json >/dev/null 2>&1 || ui_print "  ! 设置运行时初始化失败，请打开设置页检查"
else
  ui_print "  ! legacy box.tool is sealed; use boxctl"
fi

[ -z "$(find /data/adb/box/bin -type f -name '*' ! -name '*.bak')" ] && sed -Ei 's/^description=(\[.*][[:space:]]*)?/description=[ 😱 模块已安装但需手动下载内核 ] /g' $MODPATH/module.prop

if [ "$KSU" = "true" ]; then
  sed -i "s/name=.*/name=Box for KernelSU/g" $MODPATH/module.prop
elif [ "$APATCH" = "true" ]; then
  sed -i "s/name=.*/name=Box for APatch/g" $MODPATH/module.prop
else
  sed -i "s/name=.*/name=Box for Magisk/g" $MODPATH/module.prop
fi
unzip -o "$ZIPFILE" 'webroot/*' -d "$MODPATH" >&2

ui_print "- 清理残留文件"
rm -rf /data/adb/box/bin/.bin $MODPATH/box $MODPATH/box_service.sh "$MODPATH/zashboard" "$MODPATH/default-config.yaml"

if [ "$backup_box" = "true" ] && [ -n "$temp_dir" ] && [ -d "$temp_dir" ]; then
  ui_print " "
  if handle_choice "检测到更新残留备份文件，是否删除？" "删除备份" "保留备份"; then
    ui_print "- 正在删除备份: ${temp_dir}"
    rm -rf "${temp_dir}"
    ui_print "- 备份已删除"
  else
    ui_print "- 备份已保留在: ${temp_dir}"
  fi
fi

boxctl_install_daemon_stop
ui_print "- 安装完成，请重启设备。"
