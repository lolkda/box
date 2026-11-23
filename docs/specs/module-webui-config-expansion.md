# 管理页配置扩展与核心精简

## 1. 文档信息

- 文档标识：module-webui-config-expansion
- 规格状态：已确认
- 最后更新：2026-11-23
- 实现状态：已实现
- 验证状态：部分完成（本地 shell/前端测试与构建通过；真机安装升级未执行）
- 关联文档：[module-webui-refactor.md](module-webui-refactor.md)（已完成的管理页重构规格；本规格记录其后的配置扩展与核心精简）
- 关联文档：[module-webui.md](module-webui.md)（旧 WebUI 历史规格，仅作历史参考）
- 关联文档：[module-subscription.md](module-subscription.md)
- 关联文档：[module-dashboard-zashboard.md](module-dashboard-zashboard.md)（mihomo 面板固定为 zashboard 的后续规格）
- 关联文档：[module-settings-json.md](module-settings-json.md)（后续 settings.json 设置数据层重构规格）

## 2. 目标与范围

### 背景

当前管理页已完成 React + TypeScript 重构，设置页覆盖 `settings.ini` 中的 45 个标量和 6 个数组，但仍有以下缺口：

- `cn_ip_url`、`cn_ipv6_url`、`subscription_url_singbox` 未暴露。
- `package.list.cfg`、`gid.list.cfg`、`ap.list.cfg` 只能手改；`proxy_mode` 的黑名单/白名单因此不完整。
- mihomo 的常用运行配置只能直接编辑 `config.yaml`；缺少结构化入口和安全提示。
- sing-box 只有切换核心和 `singbox_stable` 设置，`subscription_url_singbox` 没有管理页入口。
- 模块支持 `mihomo / sing-box / xray / v2fly / hysteria` 五种核心，但管理页的规则、策略组、规则集只实现 mihomo；其他核心没有对应配置模型。
- 经协议覆盖评估，mihomo 已覆盖主流客户端协议；继续为其他核心维护结构化配置的收益低，且会产生“当前核心与管理页面不一致”的误导。

### 目标

- 将模块运行时核心精简为 **mihomo + sing-box**；xray / v2fly / hysteria 从安装器、更新器、运行分支、设置项和文档中移除。
- 保留 mihomo 作为主管理核心；为 mihomo 增加独立「核心配置」页，包含安全运行层结构化表单与高级 YAML 编辑。
- sing-box 只提供基础能力：运行状态与 `subscription_url_singbox` 设置，不提供结构化规则/出站/核心配置页面。
- 补齐 `cn_ip_url`、`cn_ipv6_url`、`subscription_url_singbox` 设置。
- 新增独立「应用与网络」页，管理 `package.list.cfg`、`gid.list.cfg`、`ap.list.cfg`。
- 保持现有单用户、本地 KernelSU WebView 架构；所有写入通过 `box.tool --json`。

### 非目标

- 不实现 sing-box 的结构化规则、规则集、出站、DNS 或 raw JSON 编辑。
- 不实现 `crontab.cfg` 自定义任务编辑。
- 不开放 `box_dir`、`settings_file`、`bin_list`、各核心配置路径、`uid_list` 等内部路径或常量。
- 不保留 xray / v2fly / hysteria 的运行、更新、配置兼容；升级时清理旧二进制与配置目录。
- 不自动备份核心配置或清单文件；仅做语法校验和原子替换。
- 不实现多用户权限、远程访问、HTTP 服务或后台守护进程。
- 不包含真机写入、Git 提交、推送或发布。

## 3. 参与深度与委托

- 参与深度：架构与设计（沿用 `module-webui-refactor` 的项目约定）
- 局部例外：核心精简是产品级 breaking change，用户已明确选择“只保留 mihomo + sing-box”和“升级时直接清理”；配置字段、接口命名、页面拆分由模型按本规格实现。
- 需要用户确认：核心范围、sing-box 基础能力、mihomo 结构化字段范围、清单文件三类、配置写入安全策略、文档处理方式均已确认。
- 模型自主决定：具体 API 命名、列表文件解析细节、组件拆分、表单布局、错误码、测试用例拆分、删除旧代码的执行顺序。
- 当前执行授权：允许创建本规格、修改 `box/scripts/box.tool`、`box/scripts/box.service`、`box/settings.ini`、`customize.sh`、`module.prop`、`README.md`、`CHANGELOG.md`、`webui/`、`webroot/`、`test/`；允许本地构建与测试。不包含真机部署、Git 提交或推送。

## 4. 行为要求与约束

### R-001：核心精简

- 参与者或触发方：安装器、升级流程、`box.service`、`box.tool`、WebUI 设置页。
- 前置条件与输入：模块启动、安装或升级。
- 必须满足的行为：
  - `bin_list` 只包含 `mihomo` 和 `sing-box`。
  - `bin_name` 只接受 `mihomo` 或 `sing-box`；非法值在设置校验、启动和 WebUI 中均被拒绝或回退。
  - `box.tool` / `box.service` 不再包含 xray、v2fly、hysteria 的启动、停止、配置修改、版本更新分支。
  - `customize.sh` 不提供 xray / v2fly / hysteria 下载选项，一键安装列表只含 mihomo、sing-box、GeoX、utils。
  - 升级时直接删除旧核心目录和旧二进制：`/data/adb/box/xray`、`/data/adb/box/v2fly`、`/data/adb/box/hysteria`、对应 `bin/` 文件。
  - 旧 `bin_name` 为旧核心时，升级后迁移为 `mihomo`，并在安装日志中提示。
- 输出或状态变化：服务只运行 mihomo 或 sing-box；旧核心文件不存在；设置和 UI 选项一致。
- 边界与失败行为：清理失败不阻断模块安装，但必须记录错误；核心切换失败时服务给出明确错误。

### R-002：settings.ini 缺口补齐

- 参与者或触发方：WebUI 设置页、`box.tool settings`。
- 前置条件与输入：设置查询、保存。
- 必须满足的行为：
  - `settings_scalar_keys` 包含 `cn_ip_url`、`cn_ipv6_url`、`subscription_url_singbox`。
  - 三个字段在 `settings get` 中返回，在 `settings apply` 中可写。
  - `cn_ip_url`、`cn_ipv6_url` 只接受空值或 `http(s)://` URL。
  - `subscription_url_singbox` 只接受空值或 `http(s)://` URL。
  - 设置页在 `bin_name = sing-box` 时显示 `subscription_url_singbox`，其他核心时隐藏；`cn_ip_url` / `cn_ipv6_url` 在代理或更新设置组中显示。
  - `bin_name` 选项只显示 mihomo、sing-box。
- 输出或状态变化：`settings.ini` 写入对应键，WebUI 重新读取后值一致。
- 边界与失败行为：非法 URL 返回 `E_VALIDATION`，不修改文件。

### R-003：mihomo 核心配置结构化

- 参与者或触发方：WebUI「核心配置」页、`box.tool core`。
- 前置条件与输入：`bin_name = mihomo` 且 `mihomo/config.yaml` 存在。
- 必须满足的行为：
  - 提供 `core get --json`，返回 `core`、`file`、`structured`、`raw`。
  - `structured` 只包含安全运行层字段：`mode`、`log-level`、`allow-lan`、`external-controller`、`external-ui`、`profile.store-selected`、`profile.store-fake-ip`、`unified-delay`、`tcp-concurrent`、`find-process-mode`。
  - 提供 `core apply <json文件> --json`，支持 `{"structured": {...}}` 与 `{"raw": "..."}` 两种模式。
  - 结构化保存只修改提供的字段；不删除未知顶层字段和 `proxy-groups` / `rule-providers` / `rules`。
  - raw 保存使用完整 YAML 文件内容；保存前必须通过 YAML 语法解析，语义错误不保证被拒绝。
  - 不自动创建备份；失败时不替换原文件。
- 输出或状态变化：`config.yaml` 更新；WebUI 重新读取后结构化值与 raw 内容一致。
- 边界与失败行为：非 mihomo、文件不存在、YAML 解析失败、字段类型错误均返回明确错误，不修改文件。

### R-004：应用与网络清单

- 参与者或触发方：WebUI「应用与网络」页、`box.tool lists`。
- 前置条件与输入：目标文件存在或可创建。
- 必须满足的行为：
  - 提供 `lists get --json` 和 `lists apply <json文件> --json`。
  - `package.list.cfg` 支持 `UID:包名` 与纯包名；默认 UID 为 `0`，包名必须符合 Android 包名格式。
  - `gid.list.cfg` 每行一个数字 GID；空行和注释保留或忽略。
  - `ap.list.cfg` 支持 `allow <接口>` 与 `ignore <接口>` 两类条目，接口名允许字母、数字、`+`、`_`、`.`、`:`、`-`。
  - 三类文件均通过临时文件写入，失败不覆盖原文件。
  - 不自动备份；保存前做格式校验。
- 输出或状态变化：三类文件内容更新；重启服务后 package/GID/接口规则生效。
- 边界与失败行为：非法包名、UID、GID、接口名返回 `E_VALIDATION`；文件写入失败返回 `E_IO`。

### R-005：导航与页面

- 参与者或触发方：WebUI 底部导航、「更多」页。
- 前置条件与输入：用户打开管理页。
- 必须满足的行为：
  - 「更多」页新增「核心配置」和「应用与网络」入口。
  - 核心配置页仅在 `bin_name = mihomo` 时可用；其他核心显示明确提示。
  - 应用与网络页对 mihomo/sing-box 均可用。
  - 现有策略组、规则、规则集页面仅在 mihomo 下可用；sing-box 下显示“当前核心不支持该页面”的提示，不能调用 mihomo config 接口。
- 输出或状态变化：导航项与核心状态一致。
- 边界与失败行为：核心状态未知时显示加载态，不默认按 mihomo 处理。

### R-006：sing-box 基础能力

- 参与者或触发方：设置页、概览页。
- 前置条件与输入：`bin_name = sing-box`。
- 必须满足的行为：
  - 设置页显示 `subscription_url_singbox`。
  - 概览页的状态、核心名、运行状态正确。
  - 不渲染 sing-box 的规则、规则集、出站或核心配置编辑。
- 输出或状态变化：`subscription_url_singbox` 写入 `settings.ini`；sing-box 订阅更新仍由现有 `upsubs` 路径处理。
- 边界与失败行为：sing-box 配置缺失时状态显示配置缺失，不静默显示 mihomo 配置文件。

### R-007：写入安全

- 参与者或触发方：`core apply`、`lists apply`、`settings apply`。
- 前置条件与输入：用户提交配置。
- 必须满足的行为：
  - 所有写入先写临时文件；验证通过后再替换目标文件。
  - 不做自动备份；不保留时间戳副本。
  - YAML/JSON/清单格式错误时拒绝写入。
  - 结构化配置写入只合并允许字段。
- 输出或状态变化：原文件在失败时保持不变。
- 边界与失败行为：进程中断可能留下临时文件；下次请求应清理同前缀临时文件。

### R-008：旧核心启动保护

- 参与者或触发方：`box.service`、`box.tool status`、WebUI。
- 前置条件与输入：`bin_name` 为旧核心或非法值。
- 必须满足的行为：
  - 启动前校验 `bin_name`，非法值必须拒绝启动并输出明确错误。
  - 状态接口返回实际 `bin_name` 或错误；WebUI 不把非法值显示为 mihomo。
  - 安装升级迁移逻辑将旧值改为 mihomo。
- 输出或状态变化：服务不会以不存在的核心启动。
- 边界与失败行为：迁移失败时保留设置并阻止启动，要求用户手动修正。

### 相关约束

- `box.tool` 必须继续兼容 POSIX/mksh，不能依赖 bash-only 语法。
- JSON 接口继续使用 `{ok, data, error, meta}` 包络。
- WebUI 仍通过 `kernelsu.exec()` 调用 `box.tool`。
- 配置文件路径和现有 mihomo 结构由 `settings.ini` 决定，不在 UI 中暴露。
- 不自动备份是用户明确选择；规格和 UI 必须提示失败风险，但不添加隐式备份。

## 5. 关键决定

### D-001：核心范围收敛为 mihomo + sing-box

- 状态：已确认
- 结论：只保留 mihomo 与 sing-box。
- 理由：mihomo 覆盖主流客户端协议；为 xray / v2fly / hysteria 维护独立配置模型投入产出比低。
- 考虑过的备选：保留全部核心、仅管理页收敛、只保留 mihomo + sing-box。
- 未采用原因：全部保留会产生多套 schema；只收敛 UI 仍保留无人维护的运行分支。
- 影响范围：R-001、R-006、R-008、AC-001、AC-007。

### D-002：旧核心升级直接清理

- 状态：已确认
- 结论：升级时删除旧二进制和旧配置目录，不归档。
- 理由：用户明确选择“升级时直接清理”，希望彻底移除旧核心。
- 考虑过的备选：保留文件并报错、归档后删除、只删代码。
- 未采用原因：用户选择彻底清理；本规格必须记录其不可逆风险。
- 影响范围：R-001、AC-001。

### D-003：sing-box 只做基础能力

- 状态：已确认
- 结论：仅状态 + `subscription_url_singbox`；不做结构化规则/出站/核心配置。
- 理由：sing-box 作为基础运行核心保留，避免第二套配置 schema。
- 考虑过的备选：sing-box 规则结构化、规则+出站结构化、仅 raw JSON。
- 未采用原因：与“基础 sing-box”及核心精简目标冲突。
- 影响范围：R-006、AC-007。

### D-004：mihomo 核心配置采用安全运行层 + raw

- 状态：已确认
- 结论：结构化字段仅安全运行层；完整 YAML 通过高级 raw 编辑。
- 理由：避开 `box.service` 自动改写的端口、TUN、DNS 字段。
- 考虑过的备选：运行层 + DNS/TUN、只 raw、最小字段。
- 未采用原因：DNS/TUN 会被服务逻辑覆盖；只 raw 对普通用户不友好；最小字段覆盖不足。
- 影响范围：R-003、AC-003、AC-004。

### D-005：应用与网络清单三类都做

- 状态：已确认
- 结论：`package.list.cfg`、`gid.list.cfg`、`ap.list.cfg` 均提供可视化编辑。
- 理由：三类都是运行时匹配和 `proxy_mode` 实际生效的依赖。
- 考虑过的备选：只做 package、package + GID、全部 raw。
- 未采用原因：GID 和接口清单同样常用，且结构简单。
- 影响范围：R-004、AC-005、AC-006。

### D-006：package.list.cfg 支持 UID:包名与纯包名

- 状态：已确认
- 结论：默认 UID 0，包选择器写入 `0:package`，允许手动改 UID。
- 理由：兼顾普通用户与工作资料/多用户。
- 考虑过的备选：只纯包名、全部强制 UID、只 raw。
- 未采用原因：纯包名覆盖不了工作资料；强制 UID 增加学习成本。
- 影响范围：R-004、AC-005。

### D-007：配置写入仅语法校验，不自动备份

- 状态：已确认
- 结论：写入前校验格式，失败不覆盖；不创建自动备份。
- 理由：用户明确选择“仅语法校验”。
- 考虑过的备选：自动备份、手动备份、直接覆盖。
- 未采用原因：用户未选择备份；直接覆盖被排除。
- 影响范围：R-003、R-004、R-007、AC-009。

### D-008：新增独立规格

- 状态：已确认
- 结论：新建 `docs/specs/module-webui-config-expansion.md`，不覆盖已完成的 `module-webui-refactor.md`。
- 理由：保留旧规格的实现/验证记录，新范围独立验收。
- 考虑过的备选：更新旧重构规格、更新历史规格。
- 未采用原因：会混淆已完成状态和新增未实现范围；历史规格不应修改。
- 影响范围：文档。

## 6. 验收标准

### AC-001：核心精简与升级清理

- 对应规则：R-001、R-008
- 前置条件：安装包包含新版本，旧设备存在 xray/v2fly/hysteria 目录或 `bin_name=xray`。
- 操作或输入：安装/升级模块并启动服务。
- 预期结果：旧核心目录和二进制被删除；`bin_list` 只有 mihomo/sing-box；`bin_name` 迁移为 mihomo；服务以 mihomo 启动；旧值不再出现在设置选项。
- 验证状态：未执行
- 验证证据：尚未在真机安装升级流程执行；仅完成 customize.sh 语法检查

### AC-002：settings 缺口字段

- 对应规则：R-002
- 前置条件：管理页可读取 settings。
- 操作或输入：切换核心为 sing-box，修改 `subscription_url_singbox`、`cn_ip_url`、`cn_ipv6_url` 并保存。
- 预期结果：`settings get` 返回修改后的值；`settings.ini` 写入正确；非法 URL 被拒绝且文件不变。
- 验证状态：通过
- 验证证据：本地自动验证通过：test/module-webui.test.sh 60/60，webui npm test 14/14，npm run typecheck/build 通过

### AC-003：mihomo 结构化配置

- 对应规则：R-003
- 前置条件：`bin_name=mihomo`，`box.tool core get --json` 可用。
- 操作或输入：修改 `mode`、`log-level`、`allow-lan`、`profile.store-selected` 并保存。
- 预期结果：`config.yaml` 只改变对应字段；未知字段和 groups/rules/providers 保持；重新读取值一致。
- 验证状态：通过
- 验证证据：本地自动验证通过：test/module-webui.test.sh 60/60，webui npm test 14/14，npm run typecheck/build 通过

### AC-004：mihomo raw YAML

- 对应规则：R-003、R-007
- 前置条件：核心配置页可用。
- 操作或输入：提交合法 YAML 和非法 YAML。
- 预期结果：合法 YAML 替换文件；非法 YAML 返回错误且原文件不变；没有自动备份文件生成。
- 验证状态：通过
- 验证证据：本地自动验证通过：test/module-webui.test.sh 60/60，webui npm test 14/14，npm run typecheck/build 通过

### AC-005：应用包清单

- 对应规则：R-004、D-006
- 前置条件：应用与网络页可用，`pm` API 可用或手动输入。
- 操作或输入：添加 `0:com.example.app`、纯包名 `com.example.other`、带 UID `10:com.example.app`，保存后重新读取。
- 预期结果：`package.list.cfg` 内容正确；非法包名/UID 被拒绝；列表保留顺序；服务可读取 `packages_list`。
- 验证状态：通过
- 验证证据：本地自动验证通过：test/module-webui.test.sh 60/60，webui npm test 14/14，npm run typecheck/build 通过

### AC-006：GID 与接口清单

- 对应规则：R-004
- 前置条件：应用与网络页可用。
- 操作或输入：添加 GID `1000`、allow `wlan+`、ignore `wlan0` 并保存。
- 预期结果：`gid.list.cfg` 为数字行；`ap.list.cfg` 为 `allow`/`ignore` 行；非法值被拒绝。
- 验证状态：通过
- 验证证据：本地自动验证通过：test/module-webui.test.sh 60/60，webui npm test 14/14，npm run typecheck/build 通过

### AC-007：sing-box 基础与 mihomo 页面保护

- 对应规则：R-005、R-006、R-008
- 前置条件：`bin_name=sing-box`。
- 操作或输入：打开概览、设置、策略组、规则、规则集、核心配置。
- 预期结果：概览显示 sing-box；设置显示并保存 `subscription_url_singbox`；策略组/规则/规则集/核心配置显示“仅支持 mihomo”提示，不调用 mihomo config 接口。
- 验证状态：通过
- 验证证据：本地自动验证通过：test/module-webui.test.sh 60/60，webui npm test 14/14，npm run typecheck/build 通过

### AC-008：构建与回归

- 对应规则：R-001～R-008
- 前置条件：代码完成。
- 操作或输入：运行 shell 测试、前端测试、TypeScript 检查、Vite 构建和 Chromium 页面检查。
- 预期结果：全部通过；WebUI 无横向溢出；核心配置和清单页面可打开；无控制台运行时错误。
- 验证状态：通过
- 验证证据：本地自动验证通过：test/module-webui.test.sh 60/60，webui npm test 14/14，npm run typecheck/build 通过

### AC-009：写入失败保护

- 对应规则：R-003、R-004、R-007
- 前置条件：mihomo 配置和三类清单存在。
- 操作或输入：提交非法 YAML、非法包名、非法 GID、非法接口名。
- 预期结果：对应命令返回 `E_VALIDATION`；原文件哈希不变；不生成自动备份。
- 验证状态：通过
- 验证证据：本地自动验证通过：test/module-webui.test.sh 60/60，webui npm test 14/14，npm run typecheck/build 通过

## 7. 未决问题与假设

### Q-001：raw YAML 与结构化页面的并发写入

- 类型：暂用假设
- 内容：单用户 WebView 场景下，raw 保存和结构化保存按最后一次写入生效；不实现文件锁或合并冲突。
- 影响范围：R-003、AC-004
- 是否阻塞：否；本规格按单用户顺序操作处理。
- 决定或验证方式：在实现中统一使用临时文件 + 原子替换；不做并发冲突提示。

### Q-002：清单文件注释保留

- 类型：暂用假设
- 内容：`package.list.cfg`、`gid.list.cfg`、`ap.list.cfg` 允许重写为标准头部注释和条目；不保证保留用户手工添加的注释。
- 影响范围：R-004、AC-005、AC-006
- 是否阻塞：否；用户选择可视化编辑，预期由页面管理条目。
- 决定或验证方式：实现时保留固定头部说明，写入时重建条目区。

### Q-003：旧核心清理失败

- 类型：暂用假设
- 内容：升级时旧核心文件删除失败不阻断模块安装，但日志必须提示，旧文件可能残留。
- 影响范围：R-001、AC-001
- 是否阻塞：否；服务只允许 mihomo/sing-box，残留旧文件不影响新核心启动。
- 决定或验证方式：清理函数逐项执行，`rm -f` 失败记录 Warning。

## 8. 技术设计

### 架构与模块职责

- `box/scripts/box.tool`：新增 `lists` 与 `core` JSON 子命令；扩展 settings 键和核心状态读取；移除旧核心更新/检查分支。
- `box/scripts/box.service`：只保留 mihomo/sing-box 启动分支；拒绝非法 `bin_name`。
- `box/settings.ini`：`bin_list` 收缩为两项，移除旧核心配置路径；新增 settings 缺口键。
- `customize.sh`：移除旧核心下载选项；升级时清理旧目录/二进制；迁移旧 `bin_name`。
- `webui/src/features/core-config/`：mihomo 核心配置页（结构化 + raw）。
- `webui/src/features/apps-network/`：应用包/GID/接口清单页。
- `webui/src/api/`：新增 `core`、`lists` API 与查询 hooks。
- `webui/src/app/router.tsx`、`more-page.tsx`、`shell.tsx`：新页面路由、入口和核心保护。

### JSON 接口

#### `core get`

- 调用：`box.tool core get --json`
- 成功：
  ```json
  {
    "ok": true,
    "data": {
      "core": "mihomo",
      "file": "/data/adb/box/mihomo/config.yaml",
      "structured": {
        "mode": "rule",
        "log-level": "error",
        "allow-lan": true,
        "external-controller": "0.0.0.0:9090",
        "external-ui": "./dashboard",
        "profile.store-selected": true,
        "profile.store-fake-ip": true,
        "unified-delay": false,
        "tcp-concurrent": true,
        "find-process-mode": "always"
      },
      "raw": "...完整 YAML..."
    },
    "meta": { "version": 1 }
  }
  ```
- 错误：`E_NOT_FOUND`（非 mihomo 或配置不存在）、`E_IO`。

#### `core apply`

- 调用：`box.tool core apply <json文件> --json`
- 请求二选一：
  ```json
  { "structured": { "mode": "rule", "log-level": "info" } }
  ```
  ```json
  { "raw": "mode: rule\n..." }
  ```
- 成功：`{"ok":true,"data":{"changed":["mihomo"],"mode":"structured"},"meta":{"version":1}}`
- 错误：`E_VALIDATION`（字段类型/枚举/YAML 语法）、`E_IO`、`E_NOT_FOUND`。

#### `lists get`

- 调用：`box.tool lists get --json`
- 成功：
  ```json
  {
    "ok": true,
    "data": {
      "package": [
        { "uid": "0", "package": "com.example.app" },
        { "package": "com.example.other" }
      ],
      "gid": ["1000"],
      "interfaces": {
        "allow": ["ap+", "wlan+"],
        "ignore": []
      },
      "files": {
        "package": "/data/adb/box/package.list.cfg",
        "gid": "/data/adb/box/gid.list.cfg",
        "interfaces": "/data/adb/box/ap.list.cfg"
      }
    },
    "meta": { "version": 1 }
  }
  ```

#### `lists apply`

- 调用：`box.tool lists apply <json文件> --json`
- 请求：
  ```json
  {
    "package": [
      { "uid": "0", "package": "com.example.app" },
      { "package": "com.example.other" }
    ],
    "gid": ["1000"],
    "interfaces": { "allow": ["wlan+"], "ignore": ["wlan0"] }
  }
  ```
- 成功：`{"ok":true,"data":{"changed":["package","gid","interfaces"]},"meta":{"version":1}}`
- 错误：`E_VALIDATION`、`E_IO`。

### 数据格式

- `package.list.cfg`：
  - 头部固定注释，条目行 `UID:包名` 或 `包名`；默认 UID `0`。
  - 包名正则：`^[A-Za-z0-9_]+(\.[A-Za-z0-9_]+)+$`。
- `gid.list.cfg`：每行一个 `^[0-9]+$`。
- `ap.list.cfg`：`allow <interface>` / `ignore <interface>`，接口名正则 `^[A-Za-z0-9_.:+-]+$`。
- 写入顺序：校验 → 临时文件 → `mv`；不创建 `.bak`。

### WebUI 结构

- 新增 `/core-config`：
  - 头部：mihomo 核心、文件路径、说明。
  - 两个标签：常用项（表单）、高级 YAML（Textarea）。
  - 常用项字段：mode、log-level、allow-lan、external-controller、external-ui、profile.store-selected、profile.store-fake-ip、unified-delay、tcp-concurrent、find-process-mode。
  - 保存成功 toast；非法 YAML 显示错误详情。
- 新增 `/apps-network`：
  - 应用包 tab：包选择器 + UID 输入 + 列表；支持手动包名。
  - GID tab：数字 TagInput。
  - 接口 tab：allow / ignore 两组 TagInput。
  - 保存按钮和 dirty 状态。
- `AppShell` 对 `/groups`、`/rules`、`/providers`、`/core-config` 做 mihomo-only 保护；`/apps-network` 不受核心限制。
- `MorePage` 新增「核心配置」「应用与网络」入口。

### 测试

- Shell：核心清理、`settings` 新键、`core get/apply`、`lists get/apply`、非法值、旧核心清理、status 核心切换。
- Frontend：设置字段、核心配置页、应用与网络页、sing-box 保护提示。
- Build：`npm run typecheck`、`npm test`、`npm run build`。
