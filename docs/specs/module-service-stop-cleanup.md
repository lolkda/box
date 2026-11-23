# 服务停止时清理透明代理规则

## 1. 文档信息

- 文档标识：module-service-stop-cleanup
- 规格状态：已确认
- 最后更新：2026-11-24
- 实现状态：已实现
- 验证状态：部分完成（本地 shell 144 项通过，当前设备已清理并恢复 DNS；新停止代码未真机安装验证）
- 关联文档：[module-webui-refactor.md](module-webui-refactor.md)（服务控制与订阅页规格）
- 关联文档：[module-webui-performance.md](module-webui-performance.md)（状态刷新与轮询）

## 2. 目标与范围

### 背景

用户停止 Box 服务后，WiFi 显示已连接但无法上网。ADB 只读检查确认：

- `box.tool status --json` 返回 `running=false`
- 9898 / 1053 没有进程监听
- `BOX_EXTERNAL`、`BOX_LOCAL` 仍然挂在 iptables 中
- TCP/UDP 仍 TPROXY 到 9898，DNS 仍被重定向到 1053
- 网关和公网 IP 可以 ping 通，但域名解析失败

根因是 `box.service stop` 只执行 `stop_box` 停止核心和 cron，没有调用 `box.iptables disable`。`restart` 路径有清理，`start` 会重建规则，因此问题只在用户直接停止服务时暴露。

### 目标

- 用户点击“停止服务”时，先清理透明代理规则，再停止核心。
- 清理失败时取消停止，核心继续运行，向界面返回错误。
- 增加可重复验证停止成功和清理失败两条路径的测试。
- 保持 `start`、`restart`、启动失败清理的现有行为不变。

### 非目标

- 不增加 mihomo 异常退出 watchdog。
- 不修改 `box.iptables` 的规则生成或清理算法。
- 不修改 WebUI 页面、按钮或状态轮询。
- 不处理服务进程被 SIGKILL 后无法执行清理的情况。
- 不执行 Git 提交、推送或发布。

## 3. 参与深度与委托

- 参与深度：架构与设计
- 局部例外：无
- 需要用户确认：只修显式停止；先清理规则再停止核心；清理失败取消停止。
- 模型自主决定：错误日志文案、`box.service` 内部实现结构、测试 fixture 和命令。
- 当前执行授权：用户选择“新建独立规格，保存并开始开发”；允许写入本规格、`box/scripts/box.service` 和 shell 测试；不包含 Git 提交、推送、发布或真机安装。

## 4. 行为要求与约束

### R-001：显式停止先清理规则

- 参与者或触发方：WebUI“停止服务”、`box.tool service stop`、`box.service stop`。
- 前置条件与输入：服务可能运行中；`network_mode` 已经加载。
- 必须满足的行为：
  - 在执行 `stop_box` 之前调用 `box.iptables disable`。
  - 清理成功后才停止核心和 cron。
  - 清理顺序与 `restart` 的现有顺序一致。
  - 不因为核心已停止而跳过清理。
- 输出或状态变化：iptables、DNS 重定向和 `ip rule` 清理完成，核心停止。
- 边界与失败行为：清理失败时不得继续停止核心，返回非零退出码。

### R-002：清理失败取消停止

- 参与者或触发方：`box.service stop`、`box.tool service stop --json`、WebUI 服务操作。
- 前置条件与输入：`box.iptables disable` 返回非零。
- 必须满足的行为：
  - 记录明确错误日志。
  - 不调用 `stop_box`，核心保持运行。
  - `box.service` 返回非零，`service_json` 返回 `E_COMMAND`。
  - 用户可再次点击停止重试。
- 输出或状态变化：核心和规则保持原状，网络不被裸停。
- 边界与失败行为：不能把清理失败伪装成停止成功。

### R-003：其他服务路径不变

- 参与者或触发方：`box.service start`、`box.service restart`、`box_bin_alive`。
- 前置条件与输入：现有启动和重启流程。
- 必须满足的行为：
  - `start` 继续在启动前停止旧核心，并按现有逻辑重建规则。
  - `restart` 继续先清理再停止、再启动。
  - 启动失败时 `box_bin_alive` 继续清理规则。
- 输出或状态变化：现有服务生命周期行为不变。
- 边界与失败行为：不新增重复的规则清理调用。

### R-004：可测试性和日志

- 参与者或触发方：本地 shell 测试。
- 前置条件与输入：测试环境提供可替换的 `box.iptables`。
- 必须满足的行为：
  - 停止成功时测试能观察到 `box.iptables disable` 被调用，且核心停止逻辑执行。
  - 清理失败时测试能观察到非零退出，且核心停止逻辑未执行。
  - 不依赖真实 iptables 或真实 mihomo 进程。
- 输出或状态变化：停止路径有回归保护。
- 边界与失败行为：测试不得修改真实设备网络。

### 相关约束

- 清理必须使用 Box 自带的 `box.iptables disable`，不手工拼接 iptables 命令。
- 不改变 `box.iptables disable` 的参数和清理语义。
- 错误信息不得包含订阅 URL、节点凭据或配置密钥。
- 本次只覆盖显式停止；异常退出清理见 Q-001。

## 5. 关键决定

### D-001：只修显式停止

- 状态：已确认
- 结论：只在 `box.service stop` 路径增加清理，不增加异常退出监控。
- 理由：当前故障由显式停止触发；watchdog 需要额外的守护进程和生命周期设计，范围明显更大。
- 考虑过的备选：所有 Exit 路径加 trap；启动前清理；后台 watchdog。
- 未采用原因：trap 无法覆盖 SIGKILL；启动前清理不能解决停止后断网；watchdog 超出本次范围。
- 影响范围：R-001、R-003、AC-001、AC-003。

### D-002：清理在停止之前

- 状态：已确认
- 结论：先执行 `box.iptables disable`，成功后才停止核心。
- 理由：如果先停核心再清理，清理失败会把设备留在“规则存在但核心已死”的黑hole；先清理失败则核心仍可用。
- 考虑过的备选：先停止核心再清理；清理和停止并行。
- 未采用原因：都会扩大断网窗口，且失败时不可恢复。
- 影响范围：R-001、R-002、AC-001、AC-002。

### D-003：清理失败取消停止

- 状态：已确认
- 结论：清理失败返回错误并保留核心，不执行 `stop_box`。
- 理由：停止动作不能主动把网络弄断；用户可以在修复权限或重试后再次停止。
- 考虑过的备选：停止后报错；停止后后台重试。
- 未采用原因：都会让用户处于无网络状态，且界面无法说明当前是否仍在代理。
- 影响范围：R-002、AC-002。

## 6. 验收标准

### AC-001：停止成功清理规则

- 对应规则：R-001、R-004
- 前置条件：服务运行中；测试用 `box.iptables` stub 返回成功。
- 操作或输入：执行 `box.service stop` 或 WebUI 停止服务。
- 预期结果：stub 记录 `disable` 被调用；随后核心停止；命令返回成功。
- 验证状态：通过
- 验证证据：`bash test/module-webui.test.sh` 的 service stop 用例确认先调用 `box.iptables disable`，成功后输出核心停止。

### AC-002：清理失败保留核心

- 对应规则：R-002、R-004
- 前置条件：测试用 `box.iptables` stub 返回非零；核心停止函数可被观察。
- 操作或输入：执行停止服务。
- 预期结果：返回非零；核心停止函数未执行；错误信息说明清理失败。
- 验证状态：通过
- 验证证据：同一测试用 `IPTABLES_STUB_EXIT=1` 确认返回非零、输出“已取消停止服务”，且没有核心停止日志。

### AC-003：启动和重启不回归

- 对应规则：R-003
- 前置条件：现有 shell 测试和真实启动/重启路径。
- 操作或输入：运行完整 shell 测试；检查 `start`、`restart` 调用顺序。
- 预期结果：现有测试通过；`restart` 仍先清理再停止；`start` 仍能启动。
- 验证状态：通过
- 验证证据：完整 shell 测试 144 项通过；只修改 `stop)` 分支，`start`/`restart`/`box_bin_alive` 代码未改。

## 7. 未决问题与假设

### Q-001：核心异常退出后的清理

- 类型：未决问题
- 内容：mihomo 在服务未执行 stop 的情况下崩溃或被杀，规则仍可能残留；是否需要 watchdog 或 App 层监控？
- 影响范围：R-001、R-003
- 是否阻塞：否，不阻塞本次显式停止修复；但属于后续可靠性问题。
- 决定或验证方式：先完成显式停止修复，再评估独立 watchdog 设计。

## 8. 技术设计

### 8.1 修改点

`box/scripts/box.service` 的 `stop)` 分支：

```sh
stop)
  if ! "${scripts_dir}/box.iptables" disable; then
    log Error "清理透明代理规则失败，已取消停止服务"
    exit 1
  fi
  stop_box
  ;;
```

- 不修改 `stop_box`，避免影响 `start` 中的“停止旧核心”流程和 `restart` 的既有清理。
- `box.iptables disable` 自身的输出保留，便于日志定位。
- `service_json` 已有的非零返回处理会把错误转成 `E_COMMAND`。

### 8.2 测试设计

在 `test/module-webui.test.sh` 中增加停止路径测试：

- 使用一个临时 `box.iptables` stub 记录参数并返回 0 或非零。
- 使用一个可观察的 `stop_box` 替身或检查 stub 日志，确认清理失败时没有进入核心停止。
- 不启动真实 mihomo，不修改宿主 iptables。

### 8.3 手动恢复

当前设备修复前可通过 Box 自带命令清理：

```sh
BOX_HOME=/data/adb/box /data/adb/box/scripts/box.iptables disable
```

清理后重新测试 DNS 和 HTTP；如需继续代理，再启动服务。
