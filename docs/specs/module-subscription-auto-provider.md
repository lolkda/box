# 节点订阅自动识别与 provider 拆分

## 1. 文档信息

- 文档标识：module-subscription-auto-provider
- 规格状态：草稿
- 最后更新：2026-11-24
- 实现状态：已实现
- 验证状态：部分完成（本地 shell 138/138、前端 28/28、类型检查、WebUI 与模块包构建通过；安装器首次重置未执行真机验证）
- 关联文档：[module-subscription.md](module-subscription.md)（现有 provider 订阅链路历史规格）
- 关联文档：[module-settings-json.md](module-settings-json.md)（settings.json 数据层）

## 2. 目标与范围

### 背景

当前模块的订阅逻辑把每条订阅都当作 provider 文件，只读取 `.proxies`。用户提供的聚合 URL 返回的是完整 mihomo 配置：

- `proxies: []`
- `proxy-providers` 中包含 `xusx`、`imagic`、`bkm` 三个 provider
- 还包含 `proxy-groups` 和 `rules`

因此下载成功后会报 `订阅 proxies 为空`。当前用户没有已有节点和订阅，不需要兼容旧 `renew` 或做订阅迁移。

### 目标

- 把订阅能力限定为**节点订阅管理**：订阅只负责提供节点 provider，策略组和规则继续由模块 `config.yaml` 管理。
- 增加 `subscription_mode`，取值为 `auto` 和 `provider`，默认 `auto`。
- `auto` 自动识别普通 provider 与完整配置聚合，并把聚合中的 `proxy-providers` 拆成模块可下载的 provider 文件。
- 合并普通订阅与派生 provider，只更新 `config.yaml` 的 `proxy-providers` 段。
- 订阅页仍显示一条聚合订阅，并显示派生 provider 数量。
- 首次升级到本方案时重置旧设置和默认占位配置；之后升级保留用户设置。
- 移除 `renew` 与完整配置覆盖模式，不再支持用订阅覆盖主配置。

### 非目标

- 不导入聚合配置的 `proxy-groups`、`rules`、`rule-providers`。
- 不实现 `subscription_mode=full` 或任何完整配置覆盖主配置模式。
- 不兼容旧 `renew` 字段，不做旧订阅数据迁移。
- 不修改 sing-box、xray、v2fly、hysteria 的订阅逻辑。
- 不在本阶段实现 provider 列表收缩、删除聚合订阅后的自动清理；见 Q-001、Q-002。
- 不执行真机部署、开机自启验证、Git 提交或推送。

## 3. 参与深度与委托

- 参与深度：架构与设计
- 局部例外：无
- 需要用户确认：
  - 仅做节点订阅，不导入策略组和规则
  - 完整配置聚合自动拆 provider
  - `subscription_mode=auto/provider`，默认 `auto`
  - 不保留 full 覆盖模式，移除 renew
  - provider 使用原始名称，重名拒绝
  - provider 部分失败时成功几个更新几个
  - 订阅页显示派生 provider 数量
  - 首次升级重置设置和重置整个默认 config
- 模型自主决定：
  - 派生清单 `run/subscriptions.providers` 的字段细节
  - provider 文件名、临时文件和原子替换实现
  - `update_mihomo_providers` 的合并算法
  - 首次重置检测方式和安装脚本实现
  - 本地测试 fixture、桩命令和验证命令
- 当前执行授权：允许创建本规格，修改 `box.tool`、`settings.*`、`customize.sh`、`mihomo/config.yaml`、WebUI 和测试，执行本地测试与构建；不包含真机部署、Git 提交或推送。

## 4. 行为要求与约束

### R-001：subscription_mode 设置

- 参与者或触发方：设置页、`settings get`、`settings apply`、`box.tool upsubs`
- 前置条件与输入：`settings.json` 中存在 `subscription_mode`
- 必须满足的行为：
  - 只允许 `auto`、`provider`
  - 默认值为 `auto`
  - `renew` 不再是允许的 settings 字段
  - `subscription_mode=provider` 时只按普通 provider 解析
  - `subscription_mode=auto` 时自动识别普通 provider 与完整配置聚合
- 输出或状态变化：设置页显示“订阅模式”选择项；`settings.runtime.sh` 提供 `subscription_mode`
- 边界与失败行为：缺失时按 `auto`；非法值返回校验错误

### R-002：auto 模式识别

- 参与者或触发方：`box.tool upsubs`
- 前置条件与输入：已下载的订阅临时文件，`subscription_mode=auto`
- 必须满足的行为：
  - `proxies` 是非空数组时，按普通 provider 流程处理
  - `proxies` 为空或不存在，且 `proxy-providers` 是非空映射时，按完整配置聚合流程处理
  - 普通 provider 也可以是 base64 节点列表，沿用现有解码逻辑
  - 既无有效 proxies、也无有效 proxy-providers 时，返回明确的订阅格式错误
- 输出或状态变化：进入 provider 流程或聚合拆分流程
- 边界与失败行为：解析失败时保留旧 provider 和旧主配置，不修改配置

### R-003：provider 模式

- 参与者或触发方：`box.tool upsubs`
- 前置条件与输入：`subscription_mode=provider`
- 必须满足的行为：
  - 不尝试解析 `proxy-providers`
  - 完整配置型订阅因 `proxies` 为空而失败，并提示切换 `auto`
  - 普通 provider 和 base64 节点订阅保持当前行为
- 输出或状态变化：provider 文件按现有逻辑更新
- 边界与失败行为：失败时保留旧 provider 文件，返回失败或部分成功状态

### R-004：聚合 provider 拆分

- 参与者或触发方：`box.tool upsubs` 的 auto 聚合流程
- 前置条件与输入：完整配置中存在至少一个 `proxy-providers` 条目
- 必须满足的行为：
  - 读取每个 provider 的原始名称和 `url`
  - provider 名称必须非空，且不能包含 `|`、换行或控制字符
  - 只接受 `http://` 或 `https://` URL
  - 同一聚合内 provider 名称重复时拒绝该聚合更新
  - provider 名称与普通订阅或已有派生 provider 重名时拒绝该聚合更新，保留旧配置
  - provider 文件名使用现有安全文件名规则，不能互相覆盖
  - 对每个 provider URL 下载临时文件并校验 `proxies` 非空后原子替换
  - provider URL 返回完整配置但 `proxies` 非空时，提取 `.proxies` 保存
- 输出或状态变化：生成或更新本地 provider 文件
- 边界与失败行为：
  - 单个 provider 下载或解析失败时保留该 provider 的旧文件，继续处理其他 provider
  - 全部 provider 失败时不更新主配置
  - URL 日志必须掩码，不输出 token

### R-005：派生 provider 清单

- 参与者或触发方：`box.tool upsubs` 与 `update_mihomo_providers`
- 前置条件与输入：聚合流程至少成功生成一个 provider
- 必须满足的行为：
  - 生成 `${box_run}/subscriptions.providers`
  - 每行格式为 `source|provider|url|local_path`
  - `source` 是聚合订阅在 `subscriptions.list` 中的名字
  - `provider` 是原始 provider 名称
  - `local_path` 是本地 provider 文件路径
  - 普通订阅不写入该清单
  - 某个 provider 更新失败但旧文件存在时，保留旧清单条目
  - 全部 provider 失败时保留完整旧清单
  - 使用临时文件加原子替换，权限 `0600`
- 输出或状态变化：`update_mihomo_providers` 能同时读取普通订阅与派生清单
- 边界与失败行为：清单写入失败时保留旧清单，不更新主配置

### R-006：provider 文件更新与容错

- 参与者或触发方：`box.tool upsubs`
- 前置条件与输入：普通订阅或派生 provider
- 必须满足的行为：
  - provider 文件先写入 `.download` 临时文件，校验后原子替换
  - `subscription_keep_last_good=true` 时单个 provider 失败保留旧文件
  - 成功几个更新几个；全部失败才判定整体失败
  - 首次没有旧文件时，只导入成功的 provider
- 输出或状态变化：provider 文件更新，成功/失败计数准确
- 边界与失败行为：不得因单个 provider 失败删除其他 provider 的旧文件

### R-007：主配置 proxy-providers 生成

- 参与者或触发方：`update_mihomo_providers`
- 前置条件与输入：`subscriptions.list`、`subscriptions.providers`、`config.yaml`
- 必须满足的行为：
  - 普通订阅生成原有 provider 条目
  - 派生清单中的 provider 生成 provider 条目
  - 聚合订阅自身不作为 provider 条目写入主配置
  - provider 条目包含 `type: http`、原始远程 `url`、`path: ./proxy_provider/<安全文件名>.yaml`、`interval` 和健康检查
  - 保留 `proxy-groups`、`rules`、`rule-providers` 及其他段不变
  - 写入使用临时文件后原子替换
- 输出或状态变化：`config.yaml` 的 `proxy-providers` 与普通订阅 + 派生清单一致
- 边界与失败行为：没有 provider 时写入 `proxy-providers: {}`；不得删除或重写 groups/rules

### R-008：订阅页展示

- 参与者或触发方：WebUI 订阅页与 `box.tool sub list --json`
- 前置条件与输入：存在聚合订阅及派生清单
- 必须满足的行为：
  - 订阅列表仍只显示一条聚合订阅
  - 显示该聚合订阅的派生 provider 数量
  - 普通 provider 订阅仍显示原有节点文件状态
  - 已禁用订阅显示“已禁用”，操作菜单提供“启用订阅”；启用订阅提供“禁用订阅”
  - 聚合更新失败时显示明确错误，不显示为成功
- 输出或状态变化：订阅页可见聚合 provider 数量
- 边界与失败行为：派生清单不存在时显示 `0` 或 `未展开`，不崩溃

### R-009：首次升级重置

- 参与者或触发方：安装/升级脚本
- 前置条件与输入：安装或升级进入新方案
- 必须满足的行为：
  - 检测到旧 `settings.json` 含 `renew` 时，用默认 `settings.json` 覆盖
  - 同一个首次升级流程中，用新的默认 `config.yaml` 覆盖现有配置
  - 之后升级保留用户的 settings.json 和 config.yaml
  - 旧 `renew` 不写入新设置，不做值映射
- 输出或状态变化：首次升级得到干净的默认设置和默认 config
- 边界与失败行为：无法判断是否首次升级时，宁可保留现有文件并记录警告，不反复覆盖

### R-010：默认配置占位清理

- 参与者或触发方：`box/mihomo/config.yaml`、安装脚本
- 前置条件与输入：默认配置模板
- 必须满足的行为：
  - 默认配置使用 `proxy-providers: {}`
  - 不再包含 `proxy1` 占位 provider
  - 不再包含 `url: "填写订阅链接"`
  - 默认 `proxy-groups`、`rules` 仍可启动
- 输出或状态变化：没有订阅时 mihomo 不产生无效 provider URL 错误
- 边界与失败行为：不删除用户已有自定义 groups/rules（首次重置除外）

### R-011：启用/禁用节点订阅

- 参与者或触发方：订阅页操作菜单、`box.tool sub enable/disable --json`。
- 前置条件与输入：订阅存在；服务可能运行中。
- 必须满足的行为：
  - 禁用为软禁用：保留 URL 和本地 provider 文件，从 `proxy-providers` 移除该订阅（聚合订阅移除其全部派生 provider），停止自动更新。
  - 启用后重新加入 `proxy-providers`；本地文件存在时恢复节点，文件缺失时返回 `missing=true`，提示用户更新。
  - 修改后立即重建 `proxy-providers`；服务运行中立即 reload，重载失败保留已写入配置并返回 `E_RELOAD`。
  - 如果任何策略组的 `use` 显式引用待移除的 provider，拒绝禁用并返回 `E_VALIDATION`。
  - 状态写入 `subscriptions.list` 可选第三列，`0` 表示禁用；缺省视为启用。
- 输出或状态变化：`sub list/get --json` 返回 `enabled`；配置和运行状态更新。
- 边界与失败行为：`auto_modify_config=false` 时拒绝；更新 `proxy-providers` 失败返回 `E_IO`；不得删除订阅 URL 或 provider 文件。

### 相关约束

- 订阅和 provider URL 都可能包含 token，日志、错误详情和测试输出必须掩码。
- `subscriptions.list` 用户格式前两列保持 `名字|URL`，第三列可选 `enabled`；缺省为启用。
- 派生清单是内部运行文件，不改变用户订阅格式。
- 不修改 `proxy-groups`、`rules`、`rule-providers` 是硬约束。
- Q-001、Q-002 未决期间，不实现 provider 收缩或删除聚合订阅的自动清理。

## 5. 关键决定

### D-001：只做节点订阅

- 状态：已确认
- 结论：订阅只负责节点 provider，策略组和规则继续由模块 `config.yaml` 管理。
- 理由：用户当前目标是解决节点订阅错误，且聚合完整配置的策略规则不纳入本阶段。
- 考虑过的备选：完整配置管理；双模式。
- 未采用原因：完整配置会覆盖模块配置，双模式增加复杂度。
- 影响范围：R-001、R-002、R-007、AC-001。

### D-002：subscription_mode 仅 auto/provider

- 状态：已确认
- 结论：增加 `subscription_mode`，仅 `auto` 和 `provider`，默认 `auto`，移除 `renew` 和 full 模式。
- 理由：无节点、无订阅，不需要兼容完整配置覆盖；自动识别是默认，provider 用于强制旧行为。
- 考虑过的备选：固定 auto；保留 auto/provider/full。
- 未采用原因：固定 auto 无法处理特殊 provider；保留 full 与只做节点目标冲突。
- 影响范围：R-001、R-002、R-003、AC-002、AC-003。

### D-003：完整配置聚合自动拆 provider

- 状态：已确认
- 结论：auto 模式检测到 `proxies` 为空且 `proxy-providers` 非空时，自动提取并下载各 provider。
- 理由：用户聚合 URL 就是这种格式，拆分后可以继续使用模块的节点更新链路。
- 考虑过的备选：拒绝聚合 URL；让用户手动拆分。
- 未采用原因：拒绝无法解决当前问题；手动拆分维护成本高。
- 影响范围：R-002、R-004、R-005、AC-001。

### D-004：provider 原始名称，重名拒绝

- 状态：已确认
- 结论：保留 `proxy-providers` 原始名称；同名时拒绝该聚合更新并保留旧配置。
- 理由：名称可对应聚合配置，冲突显式失败比覆盖安全。
- 考虑过的备选：加聚合名前缀；数字后缀；覆盖。
- 未采用原因：前缀/后缀改变名称；覆盖可能误伤其他来源。
- 影响范围：R-004、AC-004。

### D-005：派生清单独立维护

- 状态：已确认
- 结论：`subscriptions.list` 只保留聚合 URL，派生 provider 写入 `run/subscriptions.providers`。
- 理由：用户只维护一个来源，派生数据不污染用户清单。
- 考虑过的备选：拆成多条订阅；不保存清单。
- 未采用原因：多订阅难以同步；无清单无法还原 provider URL。
- 影响范围：R-005、R-007、AC-001、AC-005。

### D-006：部分 provider 失败保留旧文件

- 状态：已确认
- 结论：成功几个更新几个，失败 provider 保留旧文件；全部失败才整体失败。
- 理由：尽量保持可用节点，符合现有容错习惯。
- 考虑过的备选：全有或全无；失败清空。
- 未采用原因：全有或全无可用性差；清空扩大故障。
- 影响范围：R-006、AC-005。

### D-007：订阅页显示 provider 数量

- 状态：已确认
- 结论：订阅页仍只有一条聚合订阅，显示派生 provider 数量。
- 理由：保持用户维护模型简单，同时能看到拆分结果。
- 考虑过的备选：展开多条派生条目；不显示数量。
- 未采用原因：展开暴露内部数据；不显示不利于排障。
- 影响范围：R-008、AC-006。

### D-008：首次升级重置设置和 config

- 状态：已确认
- 结论：首次新方案升级时用默认 settings.json 和默认 config.yaml 覆盖；之后升级保留用户文件。
- 理由：当前没有节点和订阅，不需要迁移，干净的默认状态最适合起点。
- 考虑过的备选：只删 renew；每次升级重置；只替换配置占位。
- 未采用原因：只删 renew 需要兼容逻辑；每次重置丢设置；占位替换不足以清空旧配置。
- 影响范围：R-009、R-010、AC-007、AC-008。

### D-009：软禁用而非删除

- 状态：已确认
- 结论：禁用只从 `proxy-providers` 移除并停止自动更新，保留 URL 和本地文件；策略组 `use` 显式引用时拒绝禁用；服务运行中立即重载。
- 理由：临时停用不应丢数据，也不应产生悬空 provider 引用。
- 考虑过的备选：只暂停更新；允许悬空引用；自动删除 `use` 引用。
- 未采用原因：只暂停更新节点仍可用；悬空引用会导致重载失败；自动改模板不可控。
- 影响范围：R-011、AC-010。

## 6. 验收标准

### AC-001：auto 识别完整配置并拆分 provider

- 对应规则：R-002、R-004、R-005、R-007
- 前置条件：`subscription_mode=auto`，fixture 中 `proxies` 为空、`proxy-providers` 有 3 项
- 操作或输入：执行 `box.tool sub update`
- 预期结果：生成 3 个本地 provider 文件；`run/subscriptions.providers` 有 3 条；`config.yaml` 的 `proxy-providers` 有 3 项；不生成聚合订阅自身的 provider 条目
- 验证状态：通过
- 验证证据：`test/module-webui.test.sh` 聚合 fixture 通过

### AC-002：auto 处理普通 provider

- 对应规则：R-002、R-006
- 前置条件：`subscription_mode=auto`，fixture 是含非空 `proxies` 的普通 provider
- 操作或输入：执行 `box.tool sub update`
- 预期结果：provider 文件更新成功；不生成派生清单；主配置 provider 条目正常更新
- 验证状态：通过
- 验证证据：`test/module-webui.test.sh` 普通 provider fixture 通过

### AC-003：provider 模式拒绝完整配置

- 对应规则：R-003
- 前置条件：`subscription_mode=provider`，fixture 是完整配置聚合
- 操作或输入：执行 `box.tool sub update`
- 预期结果：返回失败并提示切换 auto；不修改现有 provider 和主配置
- 验证状态：通过
- 验证证据：`test/module-webui.test.sh` provider 模式聚合拒绝测试通过

### AC-004：provider 重名拒绝并保留配置

- 对应规则：R-004、D-004
- 前置条件：聚合 provider 名称与已有 provider 或普通订阅重名
- 操作或输入：执行聚合更新
- 预期结果：该聚合更新失败；现有 provider、清单和主配置保持原样；错误信息指出冲突名称
- 验证状态：未执行
- 验证证据：尚未执行

### AC-005：单个 provider 失败保留旧文件

- 对应规则：R-006、D-006
- 前置条件：已有 3 个 provider，其中 1 个下载或解析失败
- 操作或输入：执行聚合更新
- 预期结果：成功的 provider 更新，失败的 provider 保留旧文件；主配置仍包含 3 个 provider；返回部分成功状态
- 验证状态：通过
- 验证证据：`test/module-webui.test.sh` 单 provider 失败 fixture 返回 partial

### AC-006：订阅页显示 provider 数量

- 对应规则：R-008、D-007
- 前置条件：聚合订阅已成功生成 3 个派生 provider
- 操作或输入：打开订阅页或调用 `box.tool sub list --json`
- 预期结果：仍只有一条聚合订阅；显示 3 个 provider；不展开成 3 条用户订阅
- 验证状态：通过
- 验证证据：`sub list --json` / `sub get --json` 断言通过，前端类型检查通过

### AC-007：首次升级重置设置

- 对应规则：R-009
- 前置条件：`settings.json` 含旧 `renew` 字段
- 操作或输入：执行安装/升级
- 预期结果：`settings.json` 变为默认新设置；`renew` 不存在；`subscription_mode=auto`
- 验证状态：未执行
- 验证证据：安装器逻辑已实现，真机首次升级未执行

### AC-008：首次升级重置 config

- 对应规则：R-009、R-010
- 前置条件：`mihomo/config.yaml` 含默认 `proxy1` 占位
- 操作或输入：执行安装/升级
- 预期结果：config 替换为默认新模板；`proxy-providers: {}`；无 `填写订阅链接`
- 验证状态：未执行
- 验证证据：安装器逻辑已实现，真机首次升级未执行

### AC-009：不修改 groups/rules

- 对应规则：R-007、D-003
- 前置条件：主配置含现有 `proxy-groups`、`rules`、`rule-providers`
- 操作或输入：执行聚合更新
- 预期结果：只有 `proxy-providers` 段变化；其他段完全保持
- 验证状态：通过
- 验证证据：`test/module-webui.test.sh` 聚合前后 groups/rules 哈希比较通过

### AC-010：启用/禁用节点订阅

- 对应规则：R-011
- 前置条件：订阅已添加；服务未运行或运行中。
- 操作或输入：禁用订阅；检查 `proxy-providers`；启用；检查配置；让策略组 `use` 引用 provider 后再次禁用。
- 预期结果：禁用后 provider 条目移除，URL 和文件保留；启用后恢复；`enabled` 在列表/详情中正确；显式 `use` 引用时返回 `E_VALIDATION`。
- 验证状态：通过
- 验证证据：`test/module-webui.test.sh` 覆盖普通订阅禁用/启用、聚合订阅派生 provider 移除/恢复、`use` 引用阻止；前端测试覆盖禁用菜单和“已禁用”徽标；截图：[禁用节点订阅](../design/node-subscription-disabled.png)。

## 7. 未决问题与假设

### Q-001：聚合源 provider 列表收缩时的清理

- 类型：未决问题
- 内容：聚合 URL 下次更新时 provider 列表减少，旧 provider 文件、派生清单条目和主配置条目是否自动删除？
- 影响范围：R-004、R-005、R-007；AC-001、AC-005
- 是否阻塞：否，但阻塞 provider 收缩清理的完整验收
- 决定或验证方式：后续确认清理策略并补充测试

### Q-002：删除或重命名聚合订阅时的派生数据清理

- 类型：未决问题
- 内容：`sub rm` 或重命名聚合订阅时，是否自动清理其派生 provider 文件、清单项和主配置项？
- 影响范围：R-005、R-007；AC-006
- 是否阻塞：否，但阻塞删除/重命名场景的完整验收
- 决定或验证方式：后续确认清理策略并补充测试

### Q-003：模式切换时的派生数据

- 类型：未决问题
- 内容：`auto` 与 `provider` 模式切换时，已有派生清单和 provider 文件是否保留、清理或下次更新重新生成？
- 影响范围：R-001、R-002、R-003
- 是否阻塞：否，第一阶段覆盖首次识别和更新路径
- 决定或验证方式：后续在 `sub update` 状态处理中确认

### Q-004：provider URL 的非标准响应

- 类型：未决问题
- 内容：provider URL 返回嵌套 `proxy-providers`、base64 节点列表或没有 HTTP URL 时，应跳过、递归解析还是整体失败？
- 影响范围：R-004、R-006；AC-001、AC-005
- 是否阻塞：否，第一阶段只支持含非空 `proxies` 的 YAML provider
- 决定或验证方式：后续根据实际 provider 格式补充规则和 fixture

## 8. 技术设计

### 8.1 模块职责

| 模块 | 职责 |
|---|---|
| `box.tool upsubs` | 读取 `subscription_mode`；识别普通 provider / 聚合配置；下载 provider 文件；写派生清单 |
| `update_mihomo_providers` | 合并普通订阅与派生清单，重写主配置的 `proxy-providers` 段 |
| `settings.base.sh` / `settings-gen.awk` | 提供并校验 `subscription_mode` |
| `customize.sh` | 首次升级重置旧 settings.json 和 config.yaml |
| WebUI 设置页 | 显示“订阅模式”（auto/provider） |
| WebUI 订阅页 | 显示派生 provider 数量 |
| `box/mihomo/config.yaml` | 默认 `proxy-providers: {}`，不包含 `proxy1` 占位 |

### 8.2 更新流程

```text
box.tool sub update
  → 读取 subscription_mode
  → provider: 只走普通 provider / base64 解析
  → auto:
      → 下载临时文件
      → 非空 proxies: 普通 provider 流程
      → 空 proxies + 非空 proxy-providers:
          → 提取 provider 名称与 URL
          → 逐 provider 下载、校验 proxies、原子替换
          → 写 run/subscriptions.providers
          → update_mihomo_providers
```

### 8.3 派生清单格式

```text
source|provider|url|local_path
```

| 字段 | 类型 | 必填/出现条件 | 可为 null | 默认行为 | 含义与约束 |
|---|---|---|---|---|---|
| `source` | string | 聚合流程 | 否 | 无 | `subscriptions.list` 中的聚合订阅名 |
| `provider` | string | 聚合流程 | 否 | 无 | 聚合配置中的原始 provider 名 |
| `url` | string | 聚合流程 | 否 | 无 | provider 的 http/https URL；日志必须掩码 |
| `local_path` | string | 聚合流程 | 否 | 无 | 本地 provider 文件路径；文件内容只包含 `proxies` |

清单位于 `${box_run}/subscriptions.providers`，权限 `0600`，每次成功更新后原子替换。

### 8.4 主配置合并

`update_mihomo_providers` 读取：

1. `subscriptions.list` 中的普通订阅
2. `run/subscriptions.providers` 中的派生 provider

聚合订阅名出现在派生清单的 `source` 中时，不把该聚合订阅自身写成 provider 条目。普通 provider 条目和派生 provider 条目都使用：

```yaml
type: http
url: <远程 URL>
path: ./proxy_provider/<安全文件名>.yaml
interval: 86400
health-check:
  enable: true
  url: https://cp.cloudflare.com
  interval: 300
  timeout: 1000
  tolerance: 100
```

只替换 `proxy-providers` 段；`proxy-groups`、`rules`、`rule-providers` 和主配置其他内容保持不变。

### 8.5 首次升级检测

安装脚本在 `settings_load` 之前执行：

```text
if settings.json 含 "renew":
    settings.json = settings.default.json
    config.yaml = box/mihomo/config.yaml 默认模板
else:
    保留 settings.json 和 config.yaml
```

不使用 `renew` 的值做映射；这是一个一次性重置，不迁移旧值。迁移后 `subscription_mode` 取默认 `auto`。

### 8.6 UI 变更

- `webui/src/lib/settings.ts` 增加“订阅模式”选择项，取值为 `auto`、`provider`。
- `renew` 从设置字段列表移除。
- 订阅页在聚合订阅行显示派生 provider 数量，数量来自 `box.tool sub list --json` 的扩展字段。
- 订阅卡片操作菜单增加“禁用订阅/启用订阅”，禁用状态显示“已禁用”徽标；禁用/启用前显示确认对话框。
- 不新增独立页面，不展开派生 provider 为用户条目。

### 8.7 测试设计

- shell fixture：完整配置含空 `proxies` 和 3 个 `proxy-providers`；普通 provider fixture 含非空 `proxies`。
- 覆盖 `auto` 拆分、`auto` 普通 provider、`provider` 拒绝聚合、重名拒绝、单个 provider 失败保留、首次重置。
- 比较 `config.yaml` 的 `proxy-groups`/`rules` 哈希，验证不被改写。
- 运行现有 shell/front-end 测试，并构建 WebUI 与模块包。
- 不执行真机部署。
