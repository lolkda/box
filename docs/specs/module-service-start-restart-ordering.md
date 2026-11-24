# 服务启动与重启的规则应用顺序

## 1. 文档信息

- 文档标识：module-service-start-restart-ordering
- 规格状态：已确认
- 最后更新：2026-11-24
- 实现状态：已实现
- 验证状态：部分完成（设备实测启动/重启/黑名单模式通过；待发布包安装复验）
- 关联文档：[module-service-stop-cleanup.md](module-service-stop-cleanup.md)

## 2. 目标与范围

### 背景

用户从核心模式切换到黑名单模式，回到概览点击重启后，服务显示运行中，但应用无法联网。

代码检查发现两条独立问题：

1. WebUI 的启动入口 `box.tool service start` 只调用 `box.service start`；后者只启动核心，不应用 iptables。启动后可能没有透明代理规则。
2. `box.service restart` 并行执行 `start_box` 和 `box.iptables renew`。`start_box` 内部才解析黑名单/白名单 UID，`renew` 可能读到旧 UID 列表，或在核心未就绪时下发规则。

### 目标

- WebUI 启动服务时，核心启动后必须重建透明代理规则。
- 重启必须按“清理规则 → 停止核心 → 启动核心/解析 UID → 应用规则”的顺序串行执行。
- 任一步骤失败时停止服务并清理规则，向界面返回错误。
- 保持开机启动、快捷操作和网络变化触发的现有入口兼容。

### 非目标

- 不修改黑名单/白名单的语义。
- 不增加 mihomo 异常退出 watchdog。
- 不修改 WebUI 页面布局。
- 不处理多核心同时运行的场景。

## 3. 参与深度与委托

- 参与深度：架构与设计
- 用户确认：重启链路需要修复；启动入口也不能只启动核心。
- 模型自主决定：`box.tool` 与 `box.service` 的职责划分、错误文案、测试 fixture、回滚顺序。
- 执行授权：允许修改 `box/scripts/box.service`、`box/scripts/box.tool`、测试和规格；包含发布验证，不包含未经确认的 Git 推送。

## 4. 行为要求与约束

### R-001：WebUI 启动应用规则

- 触发方：WebUI 概览的“启动”、`box.tool service start --json`。
- 必须满足：
  - 调用 `box.service start` 启动核心。
  - 启动成功后调用 `box.iptables renew` 重建规则。
  - 规则失败时返回 `E_COMMAND`，不能只显示服务运行。
- 输出：服务运行且透明代理规则存在。

### R-002：重启串行执行

- 触发方：WebUI 概览的“重启”、`box.tool service restart --json`。
- 必须满足：
  - 先执行 `box.iptables disable`。
  - 再执行 `stop_box`。
  - 然后执行 `start_box`，完成 UID 列表解析和核心启动。
  - 最后执行 `box.iptables renew`。
  - 不使用后台并发启动 `renew`。
- 输出：规则与当前 `proxy_mode` 一致。

### R-003：失败回滚

- 清理失败：取消重启，保留当前服务与规则。
- 核心启动失败：清理规则并返回失败。
- 规则应用失败：停止核心、清理规则并返回失败。
- 不得把失败状态显示为“服务正常”。

### R-004：兼容现有入口

- 开机脚本仍可在 `box.service start` 后调用 `box.iptables enable`。
- `box.tool service start` 不应重复改开机脚本的逻辑。
- `action.sh`、`net.inotify` 现有调用保持可用。

## 5. 关键决定

### D-001：启动规则放在 box.tool

- 结论：只在 `box.tool service_json` 的 `start` 分支追加 `box.iptables renew`。
- 理由：开机脚本 `start.sh`、`action.sh`、`net.inotify` 已经在 `box.service start` 后调用 `box.iptables enable`；放进 `box.service start` 会导致这些入口重复应用规则。
- 影响：WebUI 启动修复，开机入口不重复。

### D-002：重启改为串行

- 结论：`restart` 按 disable → stop → start → renew 顺序执行。
- 理由：消除 `renew` 读取旧 UID 列表和核心未就绪的竞态。
- 备选：保留并发并等待端口；未采用，因为 UID 列表时序仍不确定。

### D-003：规则应用失败即停止

- 结论：`renew` 失败时停止核心并清理规则。
- 理由：避免“服务运行但流量未接管”的假正常状态。

## 6. 验收标准

- AC-001：`box.tool service start --json` 成功后存在 BOX_LOCAL/BOX_EXTERNAL/NAT_DNS_FORWARD 规则；黑名单 UID 规则存在。
- AC-002：`box.tool service restart --json` 后 mihomo API 响应，规则存在，应用可解析并访问 `www.baidu.com`。
- AC-003：规则应用失败时 `service start` 返回 `E_COMMAND`，不显示成功。
- AC-004：停止仍清理规则，`box.service stop` 行为不回退。
- AC-005：完整 shell 测试通过。

## 7. 未决问题与假设

### Q-001：异常退出清理

- 仍不在本次范围；独立 watchdog 后续处理。

## 8. 技术设计

### 8.1 `box.tool service start`

在 `service_json` 的 `start` 分支，`box.service start` 返回成功后执行：

```sh
ipt_output="$("${scripts_dir}/box.iptables" renew 2>&1)"
ipt_rc=$?
if [ "${ipt_rc}" -ne 0 ]; then
  json_error "E_COMMAND" "透明代理规则应用失败: ${action}" "${ipt_output}"
  return 1
fi
```

### 8.2 `box.service restart`

改为串行：

```sh
restart)
  box.iptables disable || exit 1
  stop_box
  start_box || { box.iptables disable; exit 1; }
  box.iptables renew || { stop_box; box.iptables disable; exit 1; }
  box.tool webroot
  ;;
```

### 8.3 测试

- `test/module-webui.test.sh` 增加 service start 调用 renew 的成功/失败断言。
- 设备实测：停止 → 启动（规则重建、Chrome 正常）；重启（API、规则、app 200）。
