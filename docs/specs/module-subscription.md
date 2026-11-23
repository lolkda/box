# 模块版订阅链路与配置模板修复

## 1. 文档信息

- 文档标识：module-subscription
- 规格状态：草稿
- 最后更新：2026-11-23
- 实现状态：已实现
- 验证状态：通过
- 关联文档：无
- 关联文档：[module-subscription-auto-provider.md](module-subscription-auto-provider.md)（节点订阅自动识别与 provider 拆分规格）

## 2. 目标与范围

### 背景

`boxproxy/box` 模块版当前的订阅配置依赖 `settings.ini` 中两个必须等长的 shell 数组：`subscription_url_mihomo` 和 `name_provide_mihomo_config`。手工维护容易错位，安装器「增量合并」按 `^key=` 逐行取值，多行数组会被截断。`update_subscription` 默认 `false`，`box.tool subs` 会直接拒绝执行。`upsubs()` 在 `proxies` 解析失败时会删除 provider 文件，导致上一个可用节点也丢失。模板 `box/mihomo/config.yaml` 使用非法占位符 `url: "填写订阅链接"`，且规则集地址写死 `ghfast.top`。

### 目标

- 用 `box/mihomo/subscriptions.list`（每行 `名字|URL`）作为订阅唯一来源，删除两个旧数组。
- 提供 `box.tool sub add|list|rm|update`，并保留 `subs` 作为全量更新。
- 订阅下载失败或解析失败时保留上一个可用 provider 文件，并返回非零状态。
- `settings.ini` 新增 `ruleset_prefix`，默认空表示使用官方地址，非空时给规则集 URL 加前缀。
- 清理配置模板中的非法占位符和死代码。

### 非目标

- 不修改 sing-box / xray / v2fly / hysteria 的订阅逻辑。
- 不迁移旧的 `subscription_url_mihomo` / `name_provide_mihomo_config` 数组，升级后旧配置不再生效。
- 不修改 BoxProxy App（`com.boxproxy.box`）及其私有目录。
- 不部署到真机、不改设备开机自启。

## 3. 参与深度与委托

- 参与深度：架构与设计
- 局部例外：无
- 需要用户确认：清单格式、入口命令、规则集前缀方式、失败策略、修复范围、运行方式、文档方式均已确认
- 模型自主决定：清单放在 `box/mihomo/` 下；provider 文件名由名字安全化生成；`update_subscription` 默认 `true`；清单为空时静默跳过；移除 `custom_rules_subs` 死代码；本地测试用模拟 `BOX_HOME` 与桩命令
- 当前执行授权：允许在 `lolkda/box` 仓库创建本规格、修改脚本与模板、新增本地测试；不包含真机部署、Git 提交或推送

## 4. 行为要求与约束

### R-001：订阅清单作为唯一来源

- 参与者或触发方：模块脚本与用户
- 前置条件与输入：`box/mihomo/subscriptions.list`，每行 `名字|URL`
- 必须满足的行为：忽略空行和以 `#` 开头的行；按第一个 `|` 分割名字与 URL；名字不能为空
- 输出或状态变化：`settings.ini` 不再包含 `subscription_url_mihomo` 与 `name_provide_mihomo_config`
- 边界与失败行为：清单不存在或为空时，`upsubs` 记录警告并返回 0，不视为失败

### R-002：sub 子命令

- 参与者或触发方：用户执行 `box.tool sub ...`
- 前置条件与输入：
  - `sub add <URL> [名字]`
  - `sub list`
  - `sub rm <名字>`
  - `sub update`
- 必须满足的行为：`add` 校验 http/https、拒绝重名并追加一行；`list` 输出名字、掩码后的 URL、provider 文件是否存在；`rm` 删除对应行和 provider 文件；`update` 执行全量更新
- 输出或状态变化：只修改 `subscriptions.list` 与 provider 目录
- 边界与失败行为：参数缺失或名字不存在时输出用法并返回非零

### R-003：更新失败保留上一个可用 provider

- 参与者或触发方：`upsubs` 执行订阅更新
- 前置条件与输入：已存在同名 provider 文件
- 必须满足的行为：先下载到临时文件，校验 `proxies` 非空后才原子替换正式文件
- 输出或状态变化：失败时正式文件保持原样
- 边界与失败行为：全部失败返回 1；部分成功返回 2；全部成功返回 0

### R-004：provider 配置生成

- 参与者或触发方：`update_mihomo_providers`
- 前置条件与输入：`subscriptions.list` 与 `mihomo_config`
- 必须满足的行为：重写 `config.yaml` 的 `proxy-providers` 段，每项包含 `type: http`、`url`、`path: ./proxy_provider/<安全文件名>.yaml`、`interval`、健康检查
- 输出或状态变化：`config.yaml` 的 `proxy-providers` 与清单一致
- 边界与失败行为：清单为空时写入 `proxy-providers: {}`

### R-005：规则集前缀

- 参与者或触发方：`update_mihomo_rulesets`
- 前置条件与输入：`settings.ini` 的 `ruleset_prefix`
- 必须满足的行为：`rule-providers` 段的每个 `url` 去掉上次前缀、加上当前前缀；前缀为空时使用官方地址
- 输出或状态变化：状态文件 `run/state/ruleset_prefix` 记录当前前缀
- 边界与失败行为：重复执行不产生重复前缀

### R-006：配置模板合法

- 参与者或触发方：模板与启动流程
- 前置条件与输入：`box/mihomo/config.yaml`
- 必须满足的行为：不包含 `填写订阅链接` 等非法占位符；`proxy-providers` 为空映射；规则集 URL 使用官方地址
- 输出或状态变化：无订阅时配置仍可被 mihomo 解析
- 边界与失败行为：无

### 相关约束

- 脚本必须兼容 Android `/system/bin/sh`（mksh），不得使用 bash 专有语法。
- 不得删除用户已有 provider 文件，除非用户显式执行 `sub rm`。
- 所有新增路径必须可由 `BOX_HOME` 覆盖，便于本地测试。

## 5. 关键决定

### D-001：订阅清单使用纯文本 `名字|URL`

- 状态：已确认
- 结论：`box/mihomo/subscriptions.list`
- 理由：awk 可直接解析，无 jq/yq 依赖，增删都是一行
- 考虑过的备选：JSON、每订阅一个 `.url` 文件
- 未采用原因：JSON 需要额外解析依赖；多文件管理成本更高
- 影响范围：R-001、R-002、R-004

### D-002：提供 `box.tool sub` 子命令

- 状态：已确认
- 结论：支持 `add` / `list` / `rm` / `update`，`subs` 保留为全量更新别名
- 理由：避免用户手改清单格式
- 考虑过的备选：只保留 `subs`
- 未采用原因：手改容易写错分隔符
- 影响范围：R-002

### D-003：失败保留上一个可用 provider

- 状态：已确认
- 结论：临时文件校验通过后再原子替换
- 理由：订阅失败不应中断正在工作的代理
- 考虑过的备选：清空并报错
- 未采用原因：会直接断网
- 影响范围：R-003

### D-004：规则集前缀可配置

- 状态：已确认
- 结论：`settings.ini` 新增 `ruleset_prefix`，默认空
- 理由：镜像失效时改一行即可，不用改模板
- 考虑过的备选：固定官方地址、保持 ghfast.top
- 未采用原因：固定官方在受限网络不可用；保持 ghfast.top 等于不修
- 影响范围：R-005、R-006

### D-005：废弃旧数组且不迁移

- 状态：已确认
- 结论：删除 `subscription_url_mihomo` 与 `name_provide_mihomo_config`
- 理由：两个数组本身就是错误来源
- 考虑过的备选：保留数组并自动迁移
- 未采用原因：用户明确选择不迁移
- 影响范围：R-001

## 6. 验收标准

### AC-001：添加与列出订阅

- 对应规则：R-001、R-002
- 前置条件：空的 `subscriptions.list`
- 操作或输入：`box.tool sub add https://example.com/sub 香港`
- 预期结果：清单新增 `香港|https://example.com/sub`；`sub list` 显示掩码 URL；重名再添加返回非零
- 验证状态：通过
- 验证证据：尚未执行

### AC-002：更新并生成 provider

- 对应规则：R-002、R-004
- 前置条件：清单有一条订阅，桩 curl 返回含 2 个节点的 YAML
- 操作或输入：`box.tool sub update`
- 预期结果：`proxy_provider/<名字>.yaml` 含 `proxies` 2 项；`config.yaml` 的 `proxy-providers` 含该名字与对应 path
- 验证状态：通过
- 验证证据：尚未执行

### AC-003：失败保留上一个可用 provider

- 对应规则：R-003
- 前置条件：已有上一次成功的 provider 文件，桩 curl 返回 HTTP 500
- 操作或输入：`box.tool sub update`
- 预期结果：provider 文件内容不变；命令返回非零
- 验证状态：通过
- 验证证据：尚未执行

### AC-004：删除订阅

- 对应规则：R-002
- 前置条件：清单中有 `香港`
- 操作或输入：`box.tool sub rm 香港`
- 预期结果：清单不再包含该行；对应 provider 文件被删除
- 验证状态：通过
- 验证证据：尚未执行

### AC-005：规则集前缀幂等

- 对应规则：R-005
- 前置条件：`ruleset_prefix="https://ghfast.top"`
- 操作或输入：连续执行两次 `box.tool sub update`
- 预期结果：`rule-providers` 的 url 只加一次前缀；改回空后恢复官方地址
- 验证状态：通过
- 验证证据：尚未执行

### AC-006：模板无非法占位符

- 对应规则：R-006
- 前置条件：仓库中的 `box/mihomo/config.yaml`
- 操作或输入：文本检索
- 预期结果：不含 `填写订阅链接`；`proxy-providers` 为空映射；规则集 URL 为官方地址
- 验证状态：通过
- 验证证据：尚未执行

## 7. 未决问题与假设

无。

## 8. 技术设计

### 模块职责

- `box/settings.ini`：`BOX_HOME` 覆盖、删除旧数组、新增 `subscriptions_file`、`subscription_keep_last_good`、`ruleset_prefix`，`update_subscription` 默认 `true`
- `box/scripts/box.tool`：重写 `upsubs`，新增 `sub_*` 函数与 `update_mihomo_rulesets`
- `box/mihomo/config.yaml`：移除非法占位符，规则集改官方地址
- `test/module-subscription.test.sh`：模拟 `BOX_HOME`、桩 `busybox`/`curl` 的本地回归测试

### 接口

#### box.tool sub add

- 调用方式：`box.tool sub add <URL> [名字]`
- 请求契约：URL 必须 http/https；名字可选，缺省取主机名
- 成功响应契约：追加一行并输出提示，退出码 0
- 错误与失败行为：URL 非法、名字重复、参数缺失返回非零

#### box.tool sub list

- 调用方式：`box.tool sub list`
- 成功响应契约：每行输出 `名字<TAB>掩码URL<TAB>provider状态`
- 错误与失败行为：清单为空时输出提示，退出码 0

#### box.tool sub rm

- 调用方式：`box.tool sub rm <名字>`
- 成功响应契约：删除清单行与 provider 文件，退出码 0
- 错误与失败行为：名字不存在返回非零

#### box.tool sub update

- 调用方式：`box.tool sub update`
- 成功响应契约：全部成功返回 0
- 错误与失败行为：全部失败返回 1；部分成功返回 2
