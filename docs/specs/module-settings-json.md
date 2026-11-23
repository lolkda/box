# 设置数据层重构：settings.json

## 1. 文档信息

- 文档标识：module-settings-json
- 规格状态：已确认
- 最后更新：2026-11-23
- 实现状态：已实现
- 验证状态：部分完成（本地 shell 68/68、前端 15/15、类型检查与构建通过；真机安装升级与 box.service 自动修正未执行）
- 关联文档：[module-webui-refactor.md](module-webui-refactor.md)（管理页重构历史规格）
- 关联文档：[module-webui-config-expansion.md](module-webui-config-expansion.md)（设置扩展历史规格）
- 关联文档：[module-subscription.md](module-subscription.md)（订阅设置相关历史规格）
- 关联文档：[module-subscription-auto-provider.md](module-subscription-auto-provider.md)（节点订阅自动识别与 provider 拆分规格）

## 2. 目标与范围

### 背景

当前 `settings.ini` 同时承担用户配置、内部路径常量、运行时派生变量和 shell 函数。`settings get --json` 通过 `settings_get_json` 逐项读取约 48 个标量键和 6 个数组键，每个值还会经过命令替换、`sed` 和 `awk` 转义。本地基准约为 **736ms/次**，设置页首次打开明显等待；shell 读取路径也不适合继续扩展。

用户已确认接受破坏性升级：不保留旧 `settings.ini` 兼容，不导入旧配置，升级直接使用新的默认设置。

### 目标

- 将 `settings.json` 作为唯一用户配置源。
- 用 `settings.schema.json` 定义字段类型、枚举、默认值边界和未知键策略。
- 用静态 `settings.base.sh` 保存内部路径、函数和运行时常量。
- 由 `settings.json` 自动生成 `settings.runtime.sh`，供现有 shell 脚本 source。
- `settings get --json` 直接返回扁平设置 map，token 放同级 `sensitive` 字段。
- `settings apply` 接收扁平 map，使用两阶段原子替换更新 JSON 与 runtime。
- 删除旧 `settings.ini` 的用户配置角色；安装升级直接重置为默认设置。
- 前端设置页适配扁平设置接口，保留现有字段分组和表单交互。
- 用本地基准记录改造前后的 `settings get` 耗时。

### 非目标

- 不实现自适应轮询、设置页整表重渲染优化或概览页性能优化；这些另开任务。
- 不修复 KernelSU `ksu.pm` / `ksu.listPackages` 应用选择器兼容；另开任务。
- 不兼容旧 `settings.ini`，不自动导入旧值。
- 不引入常驻 daemon、HTTP 服务或 SQLite。
- 不改变 mihomo / sing-box 的运行配置、订阅、规则和任务语义。
- 不执行真机 ADB 安装升级或 Git 推送。

## 3. 参与深度与委托

- 参与深度：架构与设计（沿用本任务前序讨论）
- 局部例外：允许破坏旧配置兼容；用户明确选择直接重置。
- 需要用户确认：D 的范围、旧配置处理、JSON 结构、运行时文件、接口、默认值策略、失败行为和性能验收口径均已确认。
- 模型自主决定：具体文件拆分、JSON 生成器实现、shell 解析细节、错误消息、测试用例拆分、前端类型命名和组件内部调整。
- 当前执行授权：允许创建本规格；允许修改 `box/scripts/*`、`box/settings.json`、`box/settings.schema.json`、`box/settings.base.sh`、删除/替换 `box/settings.ini`、`customize.sh`、`build.sh`、`module.prop`、`README.md`、`CHANGELOG.md`、`webui/`、`webroot/`、`test/`；允许本地构建与测试。不包含真机部署、Git 提交或推送。

## 4. 行为要求与约束

### R-001：settings.json 成为唯一用户配置源

- 参与者或触发方：安装器、设置页、`box.tool settings`。
- 前置条件与输入：模块安装、读取设置或保存设置。
- 必须满足的行为：
  - 用户设置存储在 `/data/adb/box/settings.json`，采用扁平键 map。
  - 标量值以 JSON 字符串表示，数组值以 JSON 字符串数组表示。
  - `githubtoken` 是敏感值，单独存储在 `/data/adb/box/githubtoken`，不写入 `settings.json`。
  - `settings.json` 只包含用户可编辑设置，不包含 `box_dir`、`box_run`、`settings_file`、`bin_list`、核心配置路径等内部常量。
  - 默认值由 `box/settings.json` 提供，字段约束由 `box/settings.schema.json` 提供。
  - 缺失键在读取时补默认值；未知键在读取或写入时返回 `E_VALIDATION`。
  - `settings.json` 仅通过 WebUI 管理，手工编辑不属于支持范围；加载时仍进行 JSON 语法和 schema 校验。
- 输出或状态变化：设置页和 `settings get` 使用同一份 `settings.json` 数据。
- 边界与失败行为：JSON 无法解析、不是对象、字段类型错误或未知键时返回明确错误；损坏时不得静默覆盖用户文件。

### R-002：settings.runtime.sh 自动生成

- 参与者或触发方：安装器、启动脚本、`box.tool settings apply`、`settings_set`。
- 前置条件与输入：`settings.json` 存在且通过校验。
- 必须满足的行为：
  - 静态内部内容放在 `box/settings.base.sh`，所有 shell 脚本 source 它。
  - 用户值由生成器写入 `/data/adb/box/run/settings.runtime.sh`。
  - shell 脚本不再 source `settings.ini`，改为 source `settings.base.sh` 和 `settings.runtime.sh`。
  - 安装时生成 runtime；启动时 runtime 缺失则按 `settings.json` 重新生成；保存成功后立即重新生成。
  - 生成文件包含所有标量赋值和数组赋值，数组必须保留空格和单引号。
  - 生成文件必须可被 `/system/bin/sh` 语法解析。
- 输出或状态变化：Shell 变量与 `settings.json` 一致；文件被删除后可在下一次启动或保存时恢复。
- 边界与失败行为：生成失败不得替换旧 runtime；错误写入日志并返回失败。

### R-003：settings get 契约

- 参与者或触发方：WebUI 设置页、`box.tool settings get --json`。
- 前置条件与输入：`settings.json` 存在。
- 必须满足的行为：
  - 成功响应 `data` 是扁平 map：标量键为 string，数组键为 string[]。
  - `githubtoken` 不出现在扁平 map 中；`data.sensitive.githubtoken` 返回 `{set,preview}`。
  - token 从 `/data/adb/box/githubtoken` 读取，文件仅 root 可读写。
  - 缺失键使用默认值补齐后返回。
  - 返回的数组与 JSON 中顺序一致。
- 输出或状态变化：不修改设置文件。
- 边界与失败行为：JSON 损坏返回 `E_VALIDATION` 或 `E_IO`，不返回部分设置；旧 runtime 保留。

### R-004：settings apply 契约

- 参与者或触发方：WebUI 设置页、`box.tool settings apply <json文件> --json`。
- 前置条件与输入：请求体为扁平 map；`githubtoken` 可选。
- 必须满足的行为：
  - 未知键拒绝；字段类型和值必须通过 schema/校验规则。
  - 缺失键使用当前值或默认值补齐；请求不要求包含所有键。
  - `githubtoken` 省略表示保持当前 token，空字符串表示清除，非空字符串表示设置新 token；token 写入独立文件 `/data/adb/box/githubtoken`。
  - 写入使用两阶段原子替换：先生成并校验临时 JSON 与临时 runtime，再一起替换正式文件。
  - 成功后 runtime 与 JSON 一致。
- 输出或状态变化：`data.changed` 返回发生变化的键；token 值不回显。
- 边界与失败行为：JSON 非法、字段类型错误、校验失败或生成失败时，旧 JSON 和旧 runtime 都保持不变；返回 `E_VALIDATION` 或 `E_IO`。

### R-005：旧 settings.ini 直接重置

- 参与者或触发方：`customize.sh`、升级流程。
- 前置条件与输入：安装或升级时存在旧 `/data/adb/box/settings.ini`。
- 必须满足的行为：
  - 不读取、不合并、不导入旧 `settings.ini` 的值。
  - 安装新默认 `settings.json`；旧文件由安装器移除，或明确忽略且不再被任何脚本读取。
  - 安装日志明确提示旧设置不会迁移。
  - `mihomo/`、`sing-box/` 等核心配置目录是否保留按现有安装器规则处理，不受本规则影响。
- 输出或状态变化：升级后使用默认设置；旧 `settings.ini` 不参与运行。
- 边界与失败行为：旧文件删除失败不阻断安装，但不得继续被脚本 source；记录警告。

### R-006：损坏 settings.json 的恢复边界

- 参与者或触发方：启动脚本、`box.tool`、`box.service`。
- 前置条件与输入：`settings.json` 缺失、损坏或 schema 不通过；最后一次有效 `settings.runtime.sh` 存在。
- 必须满足的行为：
  - 服务启动优先使用最后一次有效的 runtime。
  - `settings get` / `settings apply` 返回明确错误，指出 JSON 损坏或校验失败。
  - 不自动用默认值覆盖损坏的 `settings.json`。
  - 修复后下一次保存成功即恢复 JSON 与 runtime 一致。
- 输出或状态变化：服务不因设置读取失败而静默切换代理行为。
- 边界与失败行为：没有有效 runtime 且 JSON 无效时，启动失败并记录明确错误。

### R-007：cgroup 自动修正统一走 settings_set

- 参与者或触发方：`box.service` 的 cgroup 自动修正逻辑。
- 前置条件与输入：cgroup 启用失败，需要把对应用户设置改为 false。
- 必须满足的行为：
  - 提供 `box.tool settings set <key> <value> --json`（或等价内部命令），复用 schema 校验和两阶段写入。
  - 不再使用 `sed` 直接修改配置文件。
  - `box_user_group` 作为 `settings.base.sh` 的静态内部值，不再由 service 写回用户设置。
  - 自动修正失败时记录错误，不破坏旧 JSON/runtime。
- 输出或状态变化：修正值持久化到 `settings.json` 并同步 runtime。
- 边界与失败行为：非法 key/value 返回 `E_VALIDATION`；写入失败返回 `E_IO`。

### R-008：前端设置页适配扁平接口

- 参与者或触发方：WebUI 设置页、`webui/src/api`。
- 前置条件与输入：`settings get` 返回扁平 map + sensitive。
- 必须满足的行为：
  - 前端类型和 `buildValues` 能区分标量、数组和 token。
  - 设置页继续按现有分组和字段展示，不改变用户可见字段含义。
  - 保存请求发送扁平 map，可选 `githubtoken`。
  - 缺省值、未知键和错误由后端处理，前端展示后端错误。
- 输出或状态变化：设置页读写与新契约一致。
- 边界与失败行为：后端错误显示 `ErrorState` 或 toast，不显示空表单冒充成功。

### R-009：性能与验证

- 参与者或触发方：`settings get`、本地测试。
- 前置条件与输入：默认 `settings.json` 和有效 runtime。
- 必须满足的行为：
  - `settings get` 不再逐键执行 `settings_value` 命令替换，不再逐值调用 `json_quote`。
  - 本地基准记录改造前后耗时；改造后 `settings get` 应显著低于当前约 736ms，具体数字以实际环境记录为准。
  - 功能测试覆盖读取、写入、数组转义、缺失键、未知键、损坏 JSON、原子失败和 runtime 生成。
- 输出或状态变化：测试与基准结果写入验证证据。
- 边界与失败行为：性能测试不设 CI 绝对阈值；功能失败必须修复。

### 相关约束

- shell 必须兼容 POSIX/mksh，不依赖 bash-only 语法。
- 保留 `box.tool` 现有 `{ok,data,error,meta}` 包络和已有命令契约。
- `settings.json` 与 `settings.runtime.sh` 权限仅 root 可读写。
- 不改变订阅、mihomo 配置、规则、任务等其他 JSON 接口。
- 不把 token 明文返回给 WebUI。

## 5. 关键决定

### D-001：采用 D-strict，不考虑旧配置兼容

- 状态：已确认
- 结论：旧 `settings.ini` 直接重置，不做迁移和双格式兼容。
- 理由：用户明确接受破坏更新；避免长期维护旧解析和合并逻辑。
- 考虑过的备选：保留兼容层、一次性导入旧值、双写过渡。
- 未采用原因：用户明确选择直接重置；兼容层会抵消本次清洁化收益。
- 影响范围：R-001、R-005、AC-004。

### D-002：settings.json 使用扁平用户键 map

- 状态：已确认
- 结论：所有用户设置使用现有键名平铺；数组值直接使用 JSON 数组；内部常量不进入 JSON。
- 理由：与现有设置键和 UI 字段一一对应，避免嵌套映射。
- 考虑过的备选：分组嵌套、settings/arrays 两层结构。
- 未采用原因：增加前端和后端映射；扁平结构更适合当前字段模型。
- 影响范围：R-001、R-003、R-008。

### D-003：默认文件与独立 schema

- 状态：已确认
- 结论：默认值来自 `box/settings.json`，类型、枚举、必填边界来自 `box/settings.schema.json`。
- 理由：默认值和约束职责分离，安装器和测试可复用。
- 考虑过的备选：schema 内嵌 defaults、前端 settings.ts + 后端 allowed list。
- 未采用原因：单文件 schema 读取复杂；双份 allowed list 容易漂移。
- 影响范围：R-001、R-004、AC-001。

### D-004：静态 base + 生成 runtime

- 状态：已确认
- 结论：`settings.base.sh` 保存内部路径/函数/派生数组；`settings.runtime.sh` 只保存用户值。
- 理由：避免生成器覆盖内部常量，同时保持 shell 兼容。
- 考虑过的备选：继续生成 settings.ini、生成完整 box.env。
- 未采用原因：前者误导用户，后者生成器复杂且风险高。
- 影响范围：R-002、AC-003。

### D-005：settings get 直接返回扁平 map

- 状态：已确认
- 结论：`data` 直接是用户设置键 map；token 放 `data.sensitive.githubtoken`。
- 理由：接口与文件模型一致，减少前端转换。
- 考虑过的备选：保持旧 settings/arrays 结构、增加 v2 接口。
- 未采用原因：用户选择直接扁平；旧结构会继续复制分层模型。
- 影响范围：R-003、R-008。

### D-006：settings apply 接收扁平 map，token 可选

- 状态：已确认
- 结论：与读取对称；省略 token 保持，空字符串清除。
- 理由：降低前端保存转换和接口歧义。
- 考虑过的备选：保留旧 settings/arrays/token、独立 token 写入接口。
- 未采用原因：读写不对称；独立接口增加 IPC 和原子性复杂度。
- 影响范围：R-004、AC-002。

### D-007：缺失补默认、未知拒绝

- 状态：已确认
- 结论：缺失键补默认值；未知键拒绝。
- 理由：支持后续新增键，同时避免拼写错误静默生效。
- 考虑过的备选：全部保留、缺失报错。
- 未采用原因：保留未知键污染 runtime；缺失报错阻碍升级。
- 影响范围：R-001、R-003、R-004。

### D-008：两阶段原子替换

- 状态：已确认
- 结论：临时 JSON 与临时 runtime 全部生成并校验后一起替换；失败保留旧版本。
- 理由：避免 JSON 与 shell 运行时不一致。
- 考虑过的备选：先 JSON 后 runtime、先 runtime 后 JSON。
- 未采用原因：任一步失败会造成配置源与运行配置不一致。
- 影响范围：R-004、AC-005。

### D-009：损坏时保留最后一次有效 runtime

- 状态：已确认
- 结论：JSON 损坏不自动重置，服务按最后有效 runtime 运行，设置接口报错。
- 理由：避免静默改变代理行为，同时保留修复入口。
- 考虑过的备选：自动恢复默认、阻止启动。
- 未采用原因：自动恢复会丢配置；阻止启动降低可用性。
- 影响范围：R-006、AC-006。

### D-010：性能验收采用功能 + 本地基准

- 状态：已确认
- 结论：不设 CI 绝对耗时阈值，记录改造前后本地 `settings get` 基准。
- 理由：Android 设备差异大，本地绝对阈值容易误报。
- 考虑过的备选：硬性性能门禁、只做功能验收。
- 未采用原因：硬门禁不稳定；只做功能无法确认收益。
- 影响范围：R-009、AC-009。

### D-011：内部自动修正通过 settings_set 回写

- 状态：已确认
- 结论：`box.service` 的 cgroup 自动修正统一调用 `settings_set`，不再 sed 改配置。
- 理由：保持单一配置源和 schema 校验。
- 考虑过的备选：只改 runtime、移除自动修正。
- 未采用原因：只改 runtime 会重启失效；移除自动修正改变现有行为。
- 影响范围：R-007、AC-007。

## 6. 验收标准

### AC-001：首次读取扁平设置

- 对应规则：R-001、R-003
- 前置条件：默认 `settings.json` 存在，runtime 可生成。
- 操作或输入：执行 `box.tool settings get --json`。
- 预期结果：`data` 为扁平 map；包含全部用户标量和 6 个数组；`data.sensitive.githubtoken` 存在；`githubtoken` 不在扁平 map 中。
- 验证状态：通过
- 验证证据：`test/module-webui.test.sh` 的 settings get 断言通过（68 项全通过）。

### AC-002：扁平写入与 token 语义

- 对应规则：R-004
- 前置条件：默认设置可读。
- 操作或输入：提交 `{"network_mode":"tproxy","wifi_ssids_list":["My Home WiFi","Cafe 'Test'"],"githubtoken":"secret"}`。
- 预期结果：写入成功；`network_mode` 和数组值正确；`data.sensitive.githubtoken.set == true`；再次读取不返回明文。
- 验证状态：通过
- 验证证据：`test/module-webui.test.sh` 的 settings apply、扁平读取和 token 脱敏断言通过。

### AC-003：runtime 生成与转义

- 对应规则：R-002
- 前置条件：设置写入成功。
- 操作或输入：source `settings.base.sh` 和 `settings.runtime.sh`。
- 预期结果：布尔、字符串、空字符串、数组均正确；数组元素保留空格和单引号；`sh -n` 通过。
- 验证状态：通过
- 验证证据：`test/module-webui.test.sh` 的数组转义和 runtime 语法断言通过。

### AC-004：旧 settings.ini 升级重置

- 对应规则：R-005
- 前置条件：安装目录存在旧 `settings.ini`。
- 操作或输入：执行安装/升级流程。
- 预期结果：旧文件不再被 source；新的默认 `settings.json` 生效；安装日志提示旧设置不迁移。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-005：保存失败原子性

- 对应规则：R-004、R-008
- 前置条件：已有有效 settings.json 和 runtime。
- 操作或输入：提交非法 JSON、非法值或模拟 runtime 生成失败。
- 预期结果：返回错误；旧 settings.json 与旧 runtime 均保持不变。
- 验证状态：通过
- 验证证据：`test/module-webui.test.sh` 对非法枚举同时比较 settings.json 与 runtime 哈希，均未变化。

### AC-006：损坏 JSON 保留旧 runtime

- 对应规则：R-006
- 前置条件：有效 runtime 存在，settings.json 被破坏。
- 操作或输入：启动服务并调用 `settings get --json`。
- 预期结果：服务使用旧 runtime 继续可用；`settings get` 返回明确错误；settings.json 未被自动覆盖。
- 验证状态：通过
- 验证证据：`test/module-webui.test.sh` 删除 runtime 后恢复、损坏 settings.json 后 status 仍成功且 settings get 返回 E_VALIDATION。

### AC-007：内部自动修正持久化

- 对应规则：R-007
- 前置条件：cgroup 启用失败，需要自动关闭 `cgroup_memcg` 或 `cgroup_blkio`。
- 操作或输入：触发 `box.service` 的 cgroup 自动修正。
- 预期结果：settings.json 更新为修正值；runtime 同步；不再直接 sed 配置文件。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-008：前端设置页适配

- 对应规则：R-003、R-004、R-008
- 前置条件：WebUI 使用 mock 或 KernelSU。
- 操作或输入：打开设置页、修改标量和数组、保存。
- 预期结果：表单正确展示扁平响应；保存成功后读取一致；错误显示 toast/错误页。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-009：本地性能基准

- 对应规则：R-009
- 前置条件：默认配置、无外部网络。
- 操作或输入：连续执行 `settings get --json` 5 次，记录总耗时。
- 预期结果：改造后显著低于改造前约 736ms/次；结果记录在验证证据中。
- 验证状态：通过
- 验证证据：本地基准 5 次合计约 327ms（平均约 65ms/次），改造前约 736ms/次；约 11 倍改善。环境为本地 shell 模拟，不代表真机绝对值。

## 7. 未决问题与假设

### Q-001：真机性能与升级行为

- 类型：未决问题
- 内容：本规格只做本地 shell/前端/构建验证，不执行真机 ADB 升级；真机 `settings get` 延迟和安装器行为需后续设备验证。
- 影响范围：AC-004、AC-009
- 是否阻塞：否，不阻塞代码实现；作为验证缺口记录。
- 决定或验证方式：后续在真机执行安装升级与基准测试。

### Q-002：settings.json 与 runtime 的权限

- 类型：暂用假设
- 内容：按现有模块文件权限，settings.json 和 runtime 使用 root 可读写，不对普通应用开放。
- 影响范围：R-001、R-002
- 是否阻塞：否
- 决定或验证方式：实现时按模块现有权限策略设置，settings.json、runtime 和 githubtoken 均限制 root 读写。

## 8. 技术设计

### 8.1 文件职责

| 路径 | 职责 |
|---|---|
| `box/settings.json` | 默认用户设置，扁平 map，安装时复制为 `/data/adb/box/settings.json` |
| `box/settings.default.json` | 安装时复制为 `/data/adb/box/settings.default.json`，用于缺失键回退和 runtime 恢复 |
| `box/settings.schema.json` | 设置字段 schema：类型、枚举、数组元素约束、允许键 |
| `box/settings.base.sh` | 静态内部路径、函数、运行时派生数组；所有 shell 脚本 source |
| `/data/adb/box/githubtoken` | 敏感 token，root only；由 settings apply 单独原子替换 |
| `/data/adb/box/run/settings.runtime.sh` | 由 settings.json 生成的 shell 用户值；不手工编辑 |
| `/data/adb/box/scripts/box.tool` | settings get/apply/set、runtime 生成、校验 |
| `customize.sh` | 安装默认 JSON、删除旧 settings.ini、生成 runtime |

### 8.2 settings.json 数据模型

- 顶层为对象。
- 标量字段值为字符串；数组字段值为字符串数组。
- `githubtoken` 不在 `settings.json` 中，单独存放于 token 文件；`settings get` 只返回 `sensitive` 状态。
- 内部常量、路径、核心配置路径、`bin_list`、运行时派生数组不进入 settings.json。

### 8.3 生成流程

```text
settings.json + settings.default.json
        │
        ├─ schema/未知键/类型校验
        ├─ 生成临时 settings.runtime.sh
        ├─ `sh -n` 语法校验
        └─ 原子替换 settings.json + settings.runtime.sh
```

### 8.4 读取流程

```text
box.tool settings get --json
  → 读取 settings.json
  → 缺失键补默认值（内存中）
  → 不读取 githubtoken 明文到普通 map
  → 输出扁平 map + sensitive
```

不执行逐键 `settings_value`，不执行逐字段 `json_quote`。

### 8.5 写入流程

- `settings apply` 接收扁平 map。
- 校验未知键、类型、枚举和数组元素。
- 合并当前值/默认值，保留省略的字段。
- 生成临时 JSON 和临时 runtime。
- `sh -n` 校验 runtime。
- 用 `mv` 原子替换；任一步失败清理临时文件并返回错误。

### 8.6 运行时加载

所有 shell 脚本按顺序 source：

```sh
. "${box_dir}/settings.base.sh"
. "${box_dir}/run/settings.runtime.sh"
```

`settings.runtime.sh` 缺失时由启动脚本调用生成器恢复；不得回退到旧 `settings.ini`。

### 8.7 前端适配

- `SettingsData` 改为扁平 map 加 `sensitive`。
- `SettingsPage` 的 `buildValues` 按键类型拆分：
  - 数组字段 → `arrays`
  - 其他字段 → `settings`
  - `sensitive.githubtoken` → token 状态
- `api.applySettings` 组装扁平 map，token 可选。
- mock 的 `settings get` 返回扁平结构。

### 8.8 兼容与清理

- 移除 `box/settings.ini` 的用户配置内容。
- 安装器删除旧 `settings.ini` 或不再引用；安装日志提示不迁移。
- 文档中旧的 `settings.ini` 引用更新为 `settings.json` / `settings.runtime.sh`，历史规格保留历史说明。

