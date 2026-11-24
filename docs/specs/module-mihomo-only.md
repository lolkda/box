# 2.0.0：仅保留 mihomo

## 1. 文档信息

- 文档标识：module-mihomo-only
- 规格状态：已确认
- 最后更新：2026-11-24
- 实现状态：已实现
- 验证状态：部分完成（本地 shell 163/163、dashboard 6/6、WebUI 35/35、类型检查与构建通过；未执行真机升级）
- 关联文档：[module-settings-json.md](module-settings-json.md)（设置数据层）
- 关联文档：[module-subscription-auto-provider.md](module-subscription-auto-provider.md)（mihomo 订阅链路）

## 2. 目标与范围

### 背景

模块长期同时维护 mihomo 与 sing-box 两套核心。近期节点订阅、provider、User-Agent、规则和 WebUI 能力都围绕 mihomo 开发，sing-box 已成为只保留基础启动和单订阅更新的兼容路径。用户确认 2.0.0 完全移除 sing-box。

### 目标

- 只支持 mihomo 核心。
- 移除核心选择、sing-box 设置、订阅、内核下载、配置目录和运行分支。
- 升级时直接删除 `/data/adb/box/sing-box` 与 `/data/adb/box/bin/sing-box`。
- 旧 settings.json 中的 sing-box 字段自动清理，mihomo 配置和订阅继续可用。

### 非目标

- 不迁移 sing-box 节点或配置。
- 不保留 sing-box 兼容模式。
- 不改变 mihomo 的订阅、provider、UA、规则和网络模式行为。

## 3. 参与深度与委托

- 参与深度：架构与设计
- 局部例外：用户明确选择升级时直接删除 sing-box 数据。
- 需要用户确认：移除 sing-box；旧数据直接删除；版本号 2.0.0。
- 模型自主决定：安装器迁移实现、WebUI 清理范围、测试调整和文档更新。
- 当前执行授权：允许修改模块脚本、设置、WebUI、测试、文档并发布 2.0.0。

## 4. 行为要求与约束

### R-001：核心固定为 mihomo

- 参与者或触发方：设置页、安装器、`box.tool`、`box.service`、`box.iptables`
- 前置条件与输入：模块启动或读取旧设置
- 必须满足的行为：
  - 不再提供 `bin_name` 设置
  - 运行时始终使用 mihomo
  - 旧 `bin_name` 值被清理或忽略
- 输出或状态变化：服务、配置和状态全部指向 mihomo
- 边界与失败行为：mihomo 不存在时按现有错误流程提示下载

### R-002：删除 sing-box 支持

- 参与者或触发方：安装器、WebUI、脚本
- 前置条件与输入：安装或升级 2.0.0
- 必须满足的行为：
  - 不下载或解压 sing-box
  - 不读取 sing-box 配置或订阅
  - 不提供 sing-box 设置、核心更新或危险操作入口
  - 删除 `box/sing-box` 配置目录和 `bin/sing-box`
- 输出或状态变化：运行路径中不存在 sing-box
- 边界与失败行为：删除失败时记录安装提示，不阻断 mihomo 安装

### R-003：旧设置迁移

- 参与者或触发方：安装器
- 前置条件与输入：旧 `/data/adb/box/settings.json` 含 `bin_name`、`singbox_stable`、`subscription_url_singbox`
- 必须满足的行为：
  - 备份旧设置后删除上述字段
  - 保留其余 mihomo 设置
  - 重新生成 settings runtime
- 输出或状态变化：升级后设置可正常读取
- 边界与失败行为：迁移失败时保留备份并提示，不静默覆盖其他字段

### R-004：mihomo 功能保持

- 参与者或触发方：mihomo 订阅、provider、UA、规则和网络模式
- 前置条件与输入：升级后的 mihomo 配置
- 必须满足的行为：
  - mihomo 配置、订阅清单、provider 缓存和 User-Agent 设置保持有效
  - 自动识别普通 provider 与完整配置聚合
  - 网络模式、规则和面板行为不变
- 输出或状态变化：无行为回归
- 边界与失败行为：沿用现有失败保留与错误处理

### 相关约束

- 2.0.0 是破坏性版本。
- 不删除 `mihomo`、`run`、`settings.json` 等用户数据。
- 日志和错误信息不包含订阅 token。

## 5. 关键决定

### D-001：完全移除而不是隐藏入口

- 状态：已确认
- 结论：删除 sing-box 代码路径、配置目录和二进制，不只隐藏 UI。
- 理由：用户明确只使用 mihomo，隐藏入口会继续留下维护和误用风险。
- 考虑过的备选：隐藏核心选择；保留基础兼容。
- 未采用原因：不符合“完全去掉”的目标。
- 影响范围：R-001、R-002、R-004

### D-002：升级直接删除旧数据

- 状态：已确认
- 结论：安装器直接删除 `/data/adb/box/sing-box` 和 `/data/adb/box/bin/sing-box`。
- 理由：用户明确选择直接删除，避免残留孤立配置。
- 考虑过的备选：保留但不使用；备份后删除。
- 未采用原因：用户选择彻底清理。
- 影响范围：R-002、R-003

## 6. 验收标准

### AC-001：旧设置迁移并固定 mihomo

- 对应规则：R-001、R-003
- 前置条件：旧 settings.json 含 `bin_name=sing-box` 和 sing-box 字段
- 操作或输入：执行升级并启动
- 预期结果：sing-box 字段被清理；服务使用 mihomo
- 验证状态：部分完成
- 验证证据：设置键一致性测试通过；安装器迁移未在真机执行

### AC-002：sing-box 数据清理

- 对应规则：R-002、D-002
- 前置条件：设备存在 sing-box 目录和二进制
- 操作或输入：安装 2.0.0
- 预期结果：两个路径均被删除；mihomo 仍可启动
- 验证状态：部分完成
- 验证证据：安装器脚本语法检查通过；未执行真机安装

### AC-003：mihomo 功能回归

- 对应规则：R-004
- 前置条件：升级后的 mihomo 配置
- 操作或输入：更新订阅、更新 provider、启动服务
- 预期结果：现有 mihomo 功能正常
- 验证状态：通过
- 验证证据：shell 163/163、dashboard 6/6、WebUI 35/35、类型检查与构建通过

## 7. 未决问题与假设

### Q-001：真机升级验证

- 类型：未决问题
- 内容：尚未在真实设备上执行 2.0.0 升级，无法确认旧 sing-box 进程、目录权限和 settings 迁移的实际结果。
- 影响范围：AC-001、AC-002
- 是否阻塞：否
- 决定或验证方式：发布后由用户在设备升级时验证，并保留 `settings.json.pre-2.0.0` 备份。
