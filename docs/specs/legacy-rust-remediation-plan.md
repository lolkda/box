# Legacy Shell → Rust 行为修复计划

## 1. 文档信息

- 文档标识：legacy-rust-remediation-plan
- 规格状态：草稿
- 最后更新：2026-11-26
- 实现状态：进行中（P0 代码已部分落地；设备验收未完成）
- 验证状态：部分通过（Rust 单测/集成测试通过；subscription oracle normal/aggregate 对照；Magisk .33 五模式启动/停止/curl 已通过；KSU 最终门禁按用户要求未执行）
- 关联文档：
  - [legacy-rust-behavior-audit.md](legacy-rust-behavior-audit.md)
  - [legacy-parity.md](legacy-parity.md)
  - [rust-script-migration.md](rust-script-migration.md)

## 2. 目标与范围

### 背景

深度审计确认，当前 Rust `boxctl` 能完成生命周期、订阅下载和节点加载，但透明代理、DNS、本机 OUTPUT、UID 分流、TUN/mixed/enhance 与 legacy shell 存在结构性差异。已有“接口成功、启停成功、节点出现”的验收不足以证明用户流量行为正确。

### 目标

- 五种网络模式 `tun / tproxy / redirect / mixed / enhance` 的关键代理行为达到 legacy 等价；
- 修复审计中全部订阅/规则数据完整性问题；
- 使用 Rust 事务化规则计划器，保持幂等、顺序、清理和失败回滚；
- 以 legacy 夹具作为可执行 oracle，建立 Rust golden 与真机流量矩阵；
- Magisk `192.168.1.33` 先验收，KSU `192.168.1.12` 作为最终发布门禁；
- 分阶段提交，全部 P0 通过后才生成最终发布包。

### 非目标

- nft 不作为本次 P0；当前两台设备均 `nft=false`，无法做真实验收；
- 不恢复 eBPF；
- 不复制 legacy 的性能链、缓存格式、日志文本和所有历史边缘行为；
- 不在本计划内进行无关 WebUI 重构或新增产品功能。

## 3. 参与深度与委托

- 参与深度：架构与设计
- 局部例外：无
- 需要用户确认：P0 范围、兼容级别、规则架构、设备门禁、交付节奏，均已在本计划第 5 节确认。
- 模型自主决定：事务化计划器的内部数据结构、链名、规则顺序实现、legacy 夹具目录布局、测试命令和分阶段提交拆分。
- 当前执行授权：用户已明确要求完成测试基线后直接按本计划落地 P0 代码，并在 Magisk .33 验收、KSU .12 最终门禁；不提交 Git，不推送。

## 4. 行为要求与约束

### R-001：五种模式的行为门禁

- 参与者或触发方：用户切换 `network_mode` 并启动服务。
- 前置条件与输入：`network_mode` 为 `tun`、`tproxy`、`redirect`、`mixed`、`enhance` 之一；服务可启动；iptables 可用。
- 必须满足的行为：每种模式按 legacy 的流量路径接管；未实现的模式不得静默降级为其他模式。
- 输出或状态变化：服务状态、规则计划、DNS 处理、代理范围和清理结果与模式语义一致。
- 边界与失败行为：能力不足时返回明确警告或失败，不生成看似成功但流量不走的计划。

### R-002：mixed 必须是 TUN + redirect

- 参与者或触发方：`network_mode=mixed`。
- 前置条件与输入：TUN 可用，iptables 可用。
- 必须满足的行为：候选配置保留并启用 TUN；应用 redirect 规则；不得按 tproxy+redirect 组合实现。
- 输出或状态变化：mihomo TUN 与 redirect 规则同时存在；非 TUN 启动路径不关闭 TUN。
- 边界与失败行为：TUN 不可用时明确失败或按审计决定降级并记录，不静默忽略。

### R-003：所有非 TUN 模式建立 DNS 劫持

- 参与者或触发方：`tproxy / redirect / mixed / enhance`。
- 前置条件与输入：DNS 劫持开关和 mihomo DNS 端口可用。
- 必须满足的行为：按 legacy 在 nat/mangle 的合适链建立 UDP/TCP DNS 劫持；不得只对 TUN 建立。
- 输出或状态变化：非 TUN 模式的 DNS 查询进入 mihomo DNS，不直接泄漏到上游。
- 边界与失败行为：ip6tables 表不可用时记录降级，不声称 IPv6 DNS 已生效。

### R-004：本机 OUTPUT 与 proxy_mode 分流

- 参与者或触发方：本机应用产生 TCP/UDP 流量；`proxy_mode` 为 `core`、`blacklist` 或 `whitelist`。
- 前置条件与输入：应用 UID/package 清单可读；iptables owner/match 能力可用。
- 必须满足的行为：普通本机应用按 proxy_mode 进入代理或绕过；不能只处理 `cnip_force_proxy_apps`。
- 输出或状态变化：OUTPUT 链、UID 规则和本地防回环规则与 legacy 语义一致。
- 边界与失败行为：无法解析 UID 时明确报错或记录，不默认代理所有应用。

### R-005：TUN、enhance 与绕过规则

- 参与者或触发方：`tun` 或 `enhance` 模式启动。
- 前置条件与输入：TUN 设备、iptables forward/filter 能力。
- 必须满足的行为：TUN 建立 forward/bypass、CNIP、fake-ip/loopback 所需规则；enhance 按 legacy 避免 redirect 与 tproxy 双重接管。
- 输出或状态变化：TUN bypass 链、enhance 去重规则和清理结果正确。
- 边界与失败行为：规则部分失败时回滚，不留下半套链。

### R-006：事务化规则计划器

- 参与者或触发方：任何网络模式启动、重载或停止。
- 前置条件与输入：settings、能力探测、应用清单。
- 必须满足的行为：规则计划按阶段生成，包含链创建、跳转、规则、策略路由、DNS、清理；apply 失败回滚，cleanup 幂等。
- 输出或状态变化：规则顺序稳定；重复 apply/cleanup 不产生重复链或残留跳转。
- 边界与失败行为：任一阶段失败时返回具体阶段和命令，不报告整体成功。

### R-007：订阅数据完整性

- 参与者或触发方：订阅添加、更新、禁用、删除或聚合解析。
- 前置条件与输入：订阅清单、provider 文件、config.yaml。
- 必须满足的行为：聚合 provider 先全量校验再写入；允许 legacy 接受的 HTTP/HTTPS；保留 health-check 和完整 provider 定义；禁用/删除后清理 config 中的 provider；失败不产生部分写入。
- 输出或状态变化：provider 文件、manifest、config 和返回码一致；不残留失效 provider。
- 边界与失败行为：部分成功返回部分状态；全部失败保留最后一个可用配置并返回失败。

### R-008：规则模板与 ruleset_prefix

- 参与者或触发方：规则模板下载、应用或订阅更新。
- 前置条件与输入：规则模板、当前 config、`ruleset_prefix` 设置。
- 必须满足的行为：只保留 DNS 实际引用的旧 rule-provider；应用/更新时按前缀重写 rule-provider URL，重复执行不叠加前缀。
- 输出或状态变化：规则模板计数、provider 集合和 URL 与 legacy 一致；不因合并保留无关旧 provider。
- 边界与失败行为：前缀重写失败时保留旧 config 并返回错误。

### R-009：启动配置准备

- 参与者或触发方：服务启动。
- 前置条件与输入：settings、核心配置、TUN/UID/端口设置。
- 必须满足的行为：补齐 legacy 的 external-controller、external-ui、tproxy/redir 端口、TUN block、DNS enhanced-mode 和 UID 名单准备；`find-process-mode` 使用 `strict`；不覆盖用户显式配置。
- 输出或状态变化：候选配置可被 mihomo 校验并反映设置。
- 边界与失败行为：配置无效时启动失败，不写入主 config。

### R-010：legacy oracle 与验收门禁

- 参与者或触发方：修复实施和验收。
- 前置条件与输入：legacy 夹具、Rust 测试、Magisk/KSU 设备。
- 必须满足的行为：订阅/规则数据用 legacy 脚本和 Rust 同输入比较；网络规则用 golden 计划；真机覆盖 TCP/UDP/DNS、本机应用、UID、模式切换和清理。
- 输出或状态变化：每项验收有可重复证据；不以接口 `ok` 或节点数量替代流量行为。
- 边界与失败行为：设备能力不足时标记受阻，不伪造通过。

### R-011：iptables P0，nft 后续

- 参与者或触发方：规则计划器后端选择。
- 前置条件与输入：设备能力 `iptables`/`nft`。
- 必须满足的行为：P0 只要求 iptables 真机通过；nft 路径保留但不计入 P0。
- 输出或状态变化：状态报告明确当前后端和未验证能力。
- 边界与失败行为：nft 不可用时不得假装 nft 路径已验收。

### R-012：分阶段提交与统一发布

- 参与者或触发方：开发交付。
- 前置条件与输入：P0 规则和验收矩阵。
- 必须满足的行为：同一分支分阶段提交；中间包仅内部测试；全部 P0 通过后才生成最终发布包。
- 输出或状态变化：提交历史可审查；最终包对应已验证提交。
- 边界与失败行为：任一 P0 未通过时不出正式发布包。

### 相关约束

- 不提交、不推送、不部署，除非用户另行明确授权；本计划当前仅保存。
- 旧 `legacy/` 只作为只读 oracle，不进入最终包，不作为运行时依赖。
- 当前 PR 分支 `feat/rust-boxctl-migration` 继续作为开发分支，除非用户另选分支。

## 5. 关键决定

### D-001：兼容级别为关键代理行为严格对齐

- 状态：已确认
- 结论：五种模式的流量路径、DNS、本机 OUTPUT、proxy_mode、TUN/mixed 必须等价；非核心细节可不同但记录。
- 理由：这些差异会导致用户流量不走代理或分流错误；性能链和日志文本不影响核心正确性。
- 考虑过的备选：完全逐行为对齐；只修可见 bug。
- 未采用原因：完全对齐成本过高；只修可见 bug 会保留结构性问题。
- 影响范围：R-001 至 R-006、R-009。

### D-002：采用 Rust 事务化规则计划器

- 状态：已确认
- 结论：重构现有规则计划，按阶段生成链、跳转、规则、DNS、策略路由和清理，保留回滚与幂等。
- 理由：现有 `Vec<Rule>` 缺少阶段和本地/转发/DNS 分层，零散补规则会继续偏离。
- 考虑过的备选：增量补现有规则；调用 legacy 脚本。
- 未采用原因：增量补规则顺序/清理不可控；调用 legacy 脚本违背单二进制目标。
- 影响范围：R-001 至 R-006、R-011。

### D-003：五种模式全部 P0

- 状态：已确认
- 结论：`tun / tproxy / redirect / mixed / enhance` 全部达到关键行为标准后才算 P0 完成。
- 理由：用户明确要求五种模式全部 P0；mixed/TUN 当前存在结构错误，不能作为“后续再说”。
- 考虑过的备选：先发 tproxy/redirect，其他模式隐藏。
- 未采用原因：用户选择全部 P0。
- 影响范围：R-001、R-002、R-005、R-010。

### D-004：所有订阅/规则问题纳入 P0

- 状态：已确认
- 结论：审计中的 S1–S6、R1–R4 全部纳入 P0。
- 理由：用户要求所有订阅/规则问题 P0；部分写入、provider 残留和 ruleset_prefix 会直接影响配置和规则结果。
- 考虑过的备选：仅核心数据问题 P0，S1/完整定义缓存放 P1。
- 未采用原因：用户选择全部 P0。
- 影响范围：R-007、R-008、R-010。

### D-005：iptables 为 P0，nft 后续

- 状态：已确认
- 结论：P0 只要求 iptables 真机通过；nft 路径保留但不计入 P0。
- 理由：当前两台设备 `nft=false`，没有可验证环境；把 nft 算 P0 会产生未验证的假通过。
- 考虑过的备选：nft 纳入 P0；删除 nft。
- 未采用原因：nft 无设备；删除会移除已有能力。
- 影响范围：R-011。

### D-006：Magisk 先验收，KSU 最终门禁

- 状态：已确认
- 结论：先用 `192.168.1.33`（Magisk，Android 12/API31）跑完整矩阵；修复完成后在 `192.168.1.12`（KSU，Android 16/API36）复跑关键项，再发布。
- 理由：用户指定先 Magisk；KSU 最终门禁覆盖 API/root 差异。
- 考虑过的备选：只 Magisk；先 KSU。
- 未采用原因：只 Magisk 覆盖不足；先 KSU 与用户顺序冲突。
- 影响范围：R-010、R-012。

### D-007：分层验收门禁

- 状态：已确认
- 结论：单元测试 + 规则 golden + 真机流量矩阵；不以接口成功、启停成功或节点数量作为通过标准。
- 理由：之前验收失败正是因为只验证了接口和生命周期。
- 考虑过的备选：只规则计划测试；只真机手动。
- 未采用原因：单一层无法覆盖内核实际行为或不可重复。
- 影响范围：R-010。

### D-008：Legacy 夹具 + Rust golden

- 状态：已确认
- 结论：订阅/规则数据用 legacy 脚本在同输入下产出 oracle；网络规则用 Rust golden；真机流量做最终确认。
- 理由：避免“实现者理解的 legacy”与真实 legacy 一起错。
- 考虑过的备选：只维护 Rust golden；只真机手动。
- 未采用原因：人工 golden 可能重复理解错误；纯手动不可重复。
- 影响范围：R-007、R-008、R-010。

### D-009：分阶段提交，最终统一发布

- 状态：已确认
- 结论：同一分支分阶段提交；中间包仅内部测试；全部 P0 通过后才出最终包。
- 理由：P0 范围大，分阶段便于审查和回滚；统一发布避免把未完成模式当正式能力。
- 考虑过的备选：先发 tproxy/redirect 预览；一次性大提交。
- 未采用原因：预览仍暴露未完成模式；大提交难审查。
- 影响范围：R-012。

### D-010：本次仅保存计划，不开始开发

- 状态：已确认
- 结论：保存本计划并回读；不修改运行代码，不提交 Git，不推送。
- 理由：用户选择“仅保存计划”。
- 考虑过的备选：保存并开始开发；先 Git 保存再开发。
- 未采用原因：用户未选择开发授权。
- 影响范围：当前执行授权。

### D-011：find-process-mode 使用 strict，订阅更新保留 restart

- 状态：已确认
- 结论：候选配置使用 `find-process-mode: strict`，不使用无条件 `off`；订阅更新后继续 restart core。
- 理由：`off` 会让 `PROCESS-NAME`/`PROCESS-PATH` 规则失效；`always` 在 Android 上可能造成高 CPU。`strict` 只在规则需要进程信息时查找，兼顾兼容性和开销。restart 是保证新 provider/candidate 生效的必要行为。
- 考虑过的备选：保持 `off`；恢复 `always` + reload。
- 未采用原因：`off` 破坏进程规则；`always` 有已知 CPU 风险。
- 影响范围：R-005、R-009、AC-005、AC-009。

## 6. 验收标准

### AC-001：五种模式启动/停止/清理

- 对应规则：R-001、R-006
- 前置条件：Magisk 设备已安装基线包；iptables/TUN 可用；每种模式配置有效。
- 操作或输入：依次启动、停止 `tun/tproxy/redirect/mixed/enhance`。
- 预期结果：每种模式启动成功；规则集合符合 golden；停止后链/跳转/策略路由清理干净；无重复规则。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-002：mixed 语义

- 对应规则：R-002、R-005
- 前置条件：`network_mode=mixed`；TUN 可用。
- 操作或输入：启动服务，检查候选配置和规则计划，发送 TCP/UDP 流量。
- 预期结果：候选配置保留/启用 TUN；存在 redirect 规则；不存在 tproxy 规则；流量按 TUN+redirect 路径工作。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-003：非 TUN DNS 劫持

- 对应规则：R-003
- 前置条件：`network_mode=tproxy/redirect/mixed/enhance`；DNS 劫持开启。
- 操作或输入：发起 UDP/TCP DNS 查询，检查 nat/mangle 规则和 mihomo DNS 日志。
- 预期结果：DNS 查询进入 mihomo DNS；不直接泄漏；IPv6 表不可用时明确降级。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-004：本机应用代理与 proxy_mode

- 对应规则：R-004
- 前置条件：至少一个用于测试的应用；package/UID 清单可读。
- 操作或输入：分别设置 `proxy_mode=core/blacklist/whitelist`，从本机应用发起 TCP/UDP 请求。
- 预期结果：core 全部代理；blacklist 中应用绕过、其他代理；whitelist 中应用代理、其他绕过；OUTPUT 链存在且无回环。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-005：TUN 与 enhance 绕过/去重

- 对应规则：R-005
- 前置条件：`network_mode=tun` 或 `enhance`；TUN/forward 可用。
- 操作或输入：启动并检查 forward/filter/mangle 规则；发送本地和转发流量。
- 预期结果：TUN bypass 规则存在；enhance 不出现 redirect/tproxy 双重接管；停止后无残留。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-006：订阅聚合全量预校验

- 对应规则：R-007
- 前置条件：聚合订阅包含两个 provider，第二个与现有订阅冲突或 URL 非法。
- 操作或输入：执行订阅更新。
- 预期结果：更新前失败；第一个 provider 文件未被覆盖；config 未发生部分写入；返回明确错误。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-007：Provider 定义和禁用清理

- 对应规则：R-007
- 前置条件：订阅包含 health-check 配置；存在启用/禁用订阅。
- 操作或输入：更新、禁用、删除订阅，检查 provider 文件和 config。
- 预期结果：health-check 保留；禁用后 config 不再引用该 provider；删除后无失效条目；节点与规则不残留。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-008：规则模板 provider 与 prefix

- 对应规则：R-008
- 前置条件：当前 config 的 DNS 引用 `cn_domain`；模板不含该 provider；`ruleset_prefix` 非空。
- 操作或输入：下载/应用规则模板，再更新订阅。
- 预期结果：只补 DNS 引用的 provider；不保留无关旧 provider；rule-provider URL 只加一次前缀；重复执行不叠加。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-009：启动配置准备

- 对应规则：R-009
- 前置条件：config 缺少 external-controller/ui、端口或 TUN block；设置要求对应能力。
- 操作或输入：启动服务并检查候选配置。
- 预期结果：缺失字段按 legacy 补齐；用户显式值不被覆盖；mihomo 校验通过。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-010：Legacy oracle 数据对照

- 对应规则：R-007、R-008、R-010
- 前置条件：legacy 夹具和临时 `BOX_HOME` 可用；设备有 busybox/yq。
- 操作或输入：同一输入分别运行 legacy 和 Rust 订阅/规则操作。
- 预期结果：文件、config、JSON、错误码符合预期；差异有明确说明或修复。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-011：Magisk 完整矩阵

- 对应规则：R-010、R-011
- 前置条件：`192.168.1.33` 安装待测包；Magisk root；网络可用。
- 操作或输入：执行五种模式、DNS、本机应用、UID、清理、重启持久性测试。
- 预期结果：关键项全部通过；失败项有日志和复现步骤；无未记录降级。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-012：KSU 最终门禁

- 对应规则：R-010、R-011、R-012
- 前置条件：Magisk 矩阵通过；`192.168.1.12` 安装同一候选包。
- 操作或输入：复跑关键网络/订阅/重启项。
- 预期结果：关键项通过；最终包对应已验证提交；无 main 分支改动。
- 验证状态：未执行
- 验证证据：尚未执行

## 7. 未决问题与假设

### Q-001：nft 真机不可用

- 类型：暂用假设
- 内容：P0 仅验收 iptables；nft 路径保留但不验证。
- 影响范围：R-011、AC-011、AC-012。
- 是否阻塞：否；nft 不计入 P0 通过，但需在发布说明中标记未验证。
- 决定或验证方式：后续有 nft 设备时单独验收。

### Q-002：Magisk 设备当前不是基线包

- 类型：已解决
- 内容：`192.168.1.33` 已安装当前基线包并重启；`boxctl` SHA-256 为 `3494656bf29a20353874aa50af89afecea73126a1f5464e10915e84b9d4e9542`，能力字段不再包含 eBPF，`api_level=31`。
- 影响范围：AC-011。
- 是否阻塞：否；基线已建立。
- 决定或验证方式：安装并重启后通过 `status --json` 和哈希核对。

### Q-003：真机流量测试工具

- 类型：暂用假设
- 内容：使用设备内置 shell/curl、mihomo 日志和必要测试应用做流量验证；具体命令在阶段 1 固化。
- 影响范围：AC-003、AC-004、AC-005、AC-011。
- 是否阻塞：否；但必须在实施前确定可重复命令。
- 决定或验证方式：阶段 1 建立测试脚本和证据格式。

### Q-004：实现授权未授予

- 类型：未决问题
- 内容：用户已授权先搭测试基线，但尚未授权网络行为实现、Git 提交或推送。
- 影响范围：阶段 2 及之后；测试基线阶段不受阻。
- 是否阻塞：是；阻塞行为实现，不阻塞 oracle/golden/基线安装。
- 决定或验证方式：基线完成后由用户确认是否进入阶段 2。

## 8. 技术设计

### 8.1 规则计划器

- 输入：settings、能力探测、应用/UID 清单、DNS 端口、TUN 状态。
- 阶段：preflight → chain/create → local → forward → DNS → policy-routing → verify；每阶段记录 Rule 和回滚动作。
- 数据：规则 ID、family、table、chain、位置（append/insert）、参数、阶段、幂等键。
- 清理：按阶段逆序删除；重复执行允许 already-absent。
- 测试：golden 文件按 `mode/family/caps` 分目录；变更必须解释 golden diff。

### 8.2 Legacy oracle

- 临时根目录模拟 `BOX_HOME`；legacy 脚本使用只读副本；每个夹具独立目录。
- 比较对象：provider 文件、config.yaml、subscriptions.list、manifest、JSON 输出、退出码。
- 网络规则无法用 legacy 脚本直接比较时，使用 golden 计划 + 真机流量。

### 8.3 分阶段实施

1. 基线、夹具、golden 框架（已建立 subscription oracle 脚本、fixtures 和首版 baseline；网络/规则夹具待补）；
2. 事务化计划器与 mixed/DNS/local/proxy_mode；
3. TUN/enhance/CNIP/清理；
4. 订阅/规则 S1–S6、R1–R4；
5. 启动配置准备；
6. Magisk 完整验收，KSU 关键验收；
7. 打包、PR 更新、最终证据。
## 7. 实施记录（当前）

- 订阅下载策略允许 legacy 兼容的 `http://`；聚合 provider 下载和普通订阅下载均不再被 HTTPS-only 策略误拒。
- 聚合更新在写任何 provider 前完成全量冲突预校验；aggregate provider 定义补齐 `health-check`，保留源定义中的 `filter`/`exclude-filter`/`override`/`headers` 等字段，并对已移除 provider 做清理。
- 订阅列表的 aggregate `provider_size`/`updated_at` 从 manifest 对应文件计算，不再固定为 0/空。
- `ruleset_prefix` 在订阅更新和规则模板应用后同步，使用状态文件避免重复添加。
- 规则模板只保留当前 `config.yaml` 中 DNS `rule-set:` 实际引用的 provider，不再把所有旧 provider 全量塞回。
- `find-process-mode` 统一改为 `strict`；mixed 候选保留并强制 TUN。
- 非 TUN 模式生成 DNS hijack OUTPUT 规则；本机 OUTPUT 增加代理模式分流（core/blacklist/whitelist）及局域网/ADB 旁路。
- mixed 生成 TUN bypass + redirect 规则；TUN 模式生成 bypass 链和策略路由。
- 网络计划在 apply 前做结构校验，拒绝重复 chain 和未定义 `BOX_*` 跳转。
- 已验证：Magisk `.33` 上 tun/tproxy/redirect/mixed/enhance 均可启动、停止，外部 HTTPS 返回 204；mixed 保留 TUN 并生成 redirect；停止后清理无残留。订阅 `大杂烩` 聚合更新成功（3 个 provider、25504 bytes、节点可见）；规则模板 fetch/use 成功，DNS 引用的旧 provider 被保留，`ruleset_prefix` 加前缀/清空均生效。
- 已知偏差：tproxy 本机 TCP 使用 nat REDIRECT（redir-port）而非 legacy 的 mangle MARK 路径，实际流量验证通过；nat PREROUTING DNS 转发在当前厂商内核上被回退为仅 OUTPUT。
- 当前构建：`box-2.0.0.zip` SHA-256 `6fc3060a84c72890fb2aed8f828044c839188b02eff39d7a6e3dbae414a67da9`；内置 arm64 `boxctl` SHA-256 `a85452b40c87639e7fdb1778c0270ccb779cbc8c2de7256e8bf81ad99ec0daeb`。
- 不执行：KSU `.12` 最终门禁（用户明确指定 `.33` 为最终验收设备）；nft 不纳入本轮；API24/APatch 已豁免。
