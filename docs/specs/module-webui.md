# 模块管理页面（KernelSU WebUI）

## 1. 文档信息

- 文档标识：module-webui
- 规格状态：草稿
- 最后更新：2026-11-23
- 实现状态：已实现
- 验证状态：部分完成
- 关联文档：[module-subscription.md](module-subscription.md)

## 2. 目标与范围

### 背景

模块版没有 App 管理界面，MetaCubeXD / Zashboard 只能观察运行状态，订阅、配置、策略、规则都要命令行或手工改文件。KernelSU 的模块 WebUI 提供 `kernelsu` JavaScript API，页面可通过 `exec()` 以 root 执行 shell 命令，因此可以在 `webroot/index.html` 内实现完整管理页，无需额外服务。

### 目标

- 在 `webroot/` 提供 Preact 管理页，由 KernelSU 模块页入口打开。
- 页面通过 `kernelsu.exec()` 调用 `box.tool` 的 JSON 接口。
- 覆盖订阅、服务控制、状态、日志、常用设置、策略组、规则、规则集。
- 危险操作全部开放，但必须二次确认。
- 核心更新只使用模块内置 `upkernel` 的 GitHub 源。

### 非目标

- 不引入本地 HTTP 服务。
- 不修改 `clash-admin`，不与其共享数据。
- 不提供远程访问、多用户或权限体系。
- 不自动备份危险操作（按用户决定，仅确认框）。
- 不支持自定义核心下载地址。

## 3. 参与深度与委托

- 参与深度：架构与设计
- 局部例外：无
- 需要用户确认：页面宿主、下发方式、配置写入、危险操作保护、核心更新源、规则集范围、日志呈现均已确认
- 模型自主决定：页面结构、组件拆分、JSON 字段命名、校验规则、构建方式、测试方式
- 当前执行授权：允许创建本规格、修改 `box.tool`、新增 `webui/` 构建、替换 `webroot/`、新增测试；不包含真机部署、Git 提交或推送

## 4. 行为要求与约束

### R-001：页面入口

- 参与者或触发方：KernelSU 模块页
- 前置条件与输入：模块安装后存在 `webroot/index.html`
- 必须满足的行为：打开后显示管理页，不自动跳转
- 输出或状态变化：页内提供「打开面板」链接到 `http://127.0.0.1:9090/ui/`
- 边界与失败行为：`kernelsu` API 不可用时提示需从 KernelSU 打开

### R-002：数据访问

- 参与者或触发方：页面
- 前置条件与输入：`kernelsu.exec()` 可用
- 必须满足的行为：所有读写通过 `box.tool` 的 `--json` 接口，不直接写文件
- 输出或状态变化：页面只处理 JSON
- 边界与失败行为：命令非零退出时把 stderr 显示给用户

### R-003：订阅管理

- 参与者或触发方：页面订阅页
- 必须满足的行为：列出、添加、删除、更新订阅；显示节点文件状态
- 输出或状态变化：调用 `box.tool sub list --json`、`sub add`、`sub rm`、`sub update`
- 边界与失败行为：更新失败展示返回码与错误信息

### R-004：服务与状态

- 必须满足的行为：显示运行状态、PID、核心、模式；支持启动、停止、重启、重载
- 输出或状态变化：调用 `box.tool status --json` 与 `box.service`
- 边界与失败行为：操作中禁用按钮，失败显示原因

### R-005：日志

- 必须满足的行为：显示最近 200 行，自动刷新，可手动刷新
- 输出或状态变化：调用 `box.tool logs 200 --json`
- 边界与失败行为：日志不存在时显示空状态

### R-006：设置

- 必须满足的行为：读取并修改常用设置项，保存后提示是否需要重载/重启
- 输出或状态变化：`box.tool settings get --json`、`settings apply <file>`
- 边界与失败行为：未知键拒绝；写回前校验 settings.ini 可被 shell 解析

### R-007：策略组

- 必须满足的行为：增删改策略组，支持 `select` / `url-test` / `fallback`
- 输出或状态变化：`box.tool config get --json`、`config apply <file>`
- 边界与失败行为：被规则引用的策略组禁止删除

### R-008：规则与规则集

- 必须满足的行为：规则与规则集完整增删改；规则集字段包含 `type` / `behavior` / `format` / `url` / `path` / `interval`
- 输出或状态变化：同 R-007
- 边界与失败行为：规则引用不存在的策略组或规则集时拒绝保存

### R-009：危险操作

- 必须满足的行为：核心安装/更新、删除配置、清空日志、重置订阅均可在页面触发，且必须二次确认
- 输出或状态变化：调用对应 `box.tool` 命令
- 边界与失败行为：不自动备份；确认框需明确写出影响

### R-010：构建产物

- 必须满足的行为：`webui/` 构建输出到 `webroot/`，模块 zip 包含页面资源
- 输出或状态变化：`npm run build` 生成 `index.html` 与静态资源
- 边界与失败行为：`box.tool webroot` 不得覆盖管理页

### 相关约束

- 页面运行在 KernelSU WebView，必须兼容 Android WebView。
- 所有命令必须通过绝对路径调用 `/data/adb/box/scripts/box.tool`。
- 不得把订阅 URL、token 写入日志或页面标题。

## 5. 关键决定

### D-001：使用 KernelSU WebUI + kernelsu.exec

- 状态：已确认
- 结论：静态页面通过 `exec()` 执行 root 命令
- 理由：KernelSU 已内置入口与提权通道，无需自建服务
- 考虑过的备选：busybox httpd + CGI、专用二进制
- 未采用原因：额外服务增加攻击面与维护成本
- 影响范围：R-001、R-002

### D-002：统一通过 box.tool JSON 接口读写

- 状态：已确认
- 结论：新增 `status` / `logs` / `settings` / `config` 的 JSON 子命令
- 理由：校验、备份、原子替换集中在一处
- 考虑过的备选：页面直接写文件
- 未采用原因：引号、编码、并发都容易写坏配置
- 影响范围：R-002 至 R-008

### D-003：危险操作仅二次确认，不自动备份

- 状态：已确认
- 结论：按用户选择，只弹确认框
- 理由：用户要求全部开放
- 考虑过的备选：自动备份 + 输入确认词
- 未采用原因：用户明确选择仅确认框
- 影响范围：R-009

### D-004：核心更新只用内置源

- 状态：已确认
- 结论：页面调用 `box.tool upkernel`，不接受自定义 URL
- 理由：避免 root 级任意二进制入口
- 考虑过的备选：自定义 URL、文件上传
- 未采用原因：风险不成比例
- 影响范围：R-009

### D-005：规则集完整增删改

- 状态：已确认
- 结论：规则集与规则、策略组同页编辑
- 理由：用户要求完整增删改
- 考虑过的备选：只读 + 更新按钮
- 未采用原因：用户明确要求完整能力
- 影响范围：R-008

## 6. 验收标准

### AC-001：页面加载与状态

- 对应规则：R-001、R-004
- 前置条件：KernelSU 打开模块 WebUI
- 操作或输入：打开页面
- 预期结果：显示服务状态、核心、模式；无脚本错误
- 验证状态：部分完成
- 验证证据：页面构建通过；未在 KernelSU WebView 实测

### AC-002：订阅增删改查

- 对应规则：R-003
- 前置条件：`subscriptions.list` 为空
- 操作或输入：页面添加 `测试|https://example.com/sub`，再删除
- 预期结果：列表出现后消失；`box.tool sub list --json` 与页面一致
- 验证状态：通过
- 验证证据：`bash test/module-webui.test.sh` 通过

### AC-003：设置读写

- 对应规则：R-006
- 前置条件：默认 settings.ini
- 操作或输入：把 `network_mode` 改为 `tproxy` 并保存
- 预期结果：settings.ini 中该键变为 `tproxy`；其他键不变
- 验证状态：通过
- 验证证据：`bash test/module-webui.test.sh` 通过

### AC-004：策略与规则保存

- 对应规则：R-007、R-008
- 前置条件：config.yaml 有 1 个策略组、1 条规则
- 操作或输入：新增策略组和规则后保存
- 预期结果：config.yaml 对应段更新，其余段不变；YAML 可解析
- 验证状态：通过
- 验证证据：`bash test/module-webui.test.sh` 通过

### AC-005：非法配置拒绝

- 对应规则：R-007、R-008
- 前置条件：规则引用不存在的策略组
- 操作或输入：保存
- 预期结果：返回非零并提示原因；config.yaml 不变
- 验证状态：通过
- 验证证据：`bash test/module-webui.test.sh` 通过

### AC-006：日志接口

- 对应规则：R-005
- 前置条件：runs.log 有内容
- 操作或输入：`box.tool logs 200 --json`
- 预期结果：返回 JSON，包含行数组，长度不超过 200
- 验证状态：通过
- 验证证据：`bash test/module-webui.test.sh` 通过

### AC-007：危险操作确认

- 对应规则：R-009
- 前置条件：页面已打开
- 操作或输入：点击删除配置
- 预期结果：出现确认框并写明影响；取消后不执行
- 验证状态：部分完成
- 验证证据：页面确认交互已实现；未在设备实测

## 7. 未决问题与假设

### Q-001：busybox 是否带 httpd

- 类型：暂用假设
- 内容：本方案不依赖 httpd，此假设仅用于说明为何排除该方案
- 影响范围：无
- 是否阻塞：否
- 决定或验证方式：无需验证

## 8. 技术设计

### 模块职责

- `box/scripts/box.tool`：新增 `status`、`logs`、`settings`、`config` 子命令及 `sub list --json`
- `webui/`：Vite + Preact 源码，构建输出到 `webroot/`
- `webroot/index.html`：构建产物，KernelSU 模块页入口
- `test/module-webui.test.sh`：在模拟 `BOX_HOME` 中验证 JSON 接口

### 接口

#### box.tool status --json

- 调用方式：`box.tool status --json`
- 成功响应：`{"running":true,"pid":123,"core":"mihomo","mode":"tproxy","config":"config.yaml"}`
- 错误：返回非零并输出错误

#### box.tool logs [行数] --json

- 成功响应：`{"lines":["..."],"file":"/data/adb/box/run/runs.log"}`

#### box.tool sub list --json

- 成功响应：`{"items":[{"name":"香港","url":"https://...","provider":"已下载"}]}`

#### box.tool settings get --json

- 成功响应：`{"settings":{"network_mode":"tproxy","proxy_tcp":"true",...}}`

#### box.tool settings apply <file>

- 请求：JSON 对象，键必须在白名单内
- 成功响应：`{"ok":true,"changed":["network_mode"]}`
- 错误：未知键或写入失败返回非零

#### box.tool config get --json

- 成功响应：`{"proxy-groups":[...],"rule-providers":{...},"rules":["..."]}`

#### box.tool config apply <file>

- 请求：包含上述三个字段的 JSON
- 成功响应：`{"ok":true}`
- 错误：YAML 校验失败返回非零并保留原文件
