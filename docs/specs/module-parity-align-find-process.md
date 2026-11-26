# Legacy 行为续接与 find-process-mode 设置

## 1. 文档信息

- 文档标识：module-parity-align-find-process
- 规格状态：已确认
- 最后更新：2026-11-26
- 实现状态：已完成
- 验证状态：通过（设备能力不足项按 E_NETWORK_DEGRADED 记录）
- 关联文档：[legacy-rust-remediation-plan.md](legacy-rust-remediation-plan.md)（整体 P0 修复计划）
- 关联文档：[legacy-rust-behavior-audit.md](legacy-rust-behavior-audit.md)（legacy 差异审计基线）
- 关联文档：[module-network-rules.md](module-network-rules.md)（已完成的 WebUI 网络规则管理）
- 关联文档：[rust-script-migration.md](rust-script-migration.md)（服务、网络和设置架构约束）

## 2. 目标与范围

### 背景

Rust `boxctl` 已完成网络规则结构化管理和五种模式基础验收，但相对 `legacy/box-scripts/` 仍有高影响行为差异：

- TUN 配置只保证 `enable=true`，没有补齐 legacy 的 stack、dns-hijack、auto-route、strict-route、UID 名单等；
- 非 TUN 的 `proxy_mode != core` 场景没有按 legacy 切换 `enhanced-mode: redir-host` 并启用 sniffer；
- 本机 UDP/QUIC 代理路径不完整；
- IPv6/CNIP 在缺少 `ip6tables nat` 时只有部分降级；
- enhance 没有 legacy 的去重和分支处理；
- `find-process-mode` 虽在核心配置页可见，但服务启动会强制覆盖为 `strict`，用户无法通过 WebUI 控制。

用户要求继续对齐原始行为，并把 `find-process-mode` 做成 WebUI 可控、`settings.json` 持久化的独立设置。

### 目标

- 分阶段补齐高影响网络行为：TUN 配置合并、DNS enhanced/sniffer、本机 UDP/QUIC、IPv6/CNIP 降级和 enhance 边界。
- 让 `find-process-mode` 成为设置页独立控件，支持 `always/strict/off`，默认 `strict`。
- 以 `settings.json` 为唯一来源；服务候选配置只从该设置写入 `find-process-mode`。
- 修改后由用户显式确认重启核心生效，避免静默断线。
- 在 Magisk `.33` 和 KernelSU `.12` 上按能力矩阵验收，不以 TCP `204` 代替全部行为。

### 非目标

- 不一次性重写事件、cgroup、cron、taskset 和 nft 全部路径；这些放在后续阶段。
- 不逐字复制 legacy 日志、规则计数或 reload/restart 实现。
- 不把任意 mihomo YAML 编辑搬到设置页。
- 不引入原始 iptables 文本编辑。
- 不自动提交、推送或部署 Git 变更。

## 3. 参与深度与委托

- 参与深度：架构与设计（沿用本会话已确认的项目参与方式）。
- 局部例外：`find-process-mode` 的 UI 形态和网络 P0 取舍由用户确认；具体字段名、内部模块拆分、测试拆分和实现顺序由模型决定。
- 需要用户确认：P0 范围、功能等价标准、find-process 的 UI/默认值/来源/生效方式、TUN 合并策略、DNS 条件、本机 UDP 方案、IPv6 降级、全局作用范围、旧值迁移、实施顺序、验收矩阵和文档方式均已确认。
- 模型自主决定：`settings.json` 字段命名、schema/校验、设置页组件位置、TUN 字段补全顺序、临时规则链名、测试夹具、错误码。
- 当前执行授权：允许创建本规格，修改 `rust/crates/boxctl/`、`webui/src/`、测试、默认设置和文档；允许本地构建与测试；允许在 `.33` 和 `.12` 上做真机验收。不提交或推送 Git。

## 4. 行为要求与约束

### R-001：分阶段高影响 P0

- 参与者或触发方：实现、测试和验收流程。
- 前置条件与输入：legacy 审计、当前 Rust 行为、两台设备能力。
- 必须满足的行为：
  - 阶段一：TUN 配置合并和 DNS enhanced/sniffer；
  - 阶段二：本机 UDP/QUIC；
  - 阶段三：IPv6/CNIP 降级和 enhance 边界；
  - 每阶段有独立测试和回滚边界，完成全部阶段后才做统一双设备验收。
- 输出或状态变化：每个阶段可独立验证，不把未完成阶段标记为通过。
- 边界与失败行为：阶段失败保留上一阶段可用状态；不得用 fallback 掩盖失败。

### R-002：find-process-mode 设置

- 参与者或触发方：WebUI 设置页、`settings.json` schema、服务启动、核心配置页。
- 前置条件与输入：用户修改进程匹配模式；模块有 `settings.json`。
- 必须满足的行为：
  - 新增设置键 `find_process_mode`，允许 `always`、`strict`、`off`，默认 `strict`。
  - 设置页显示独立“进程匹配模式”控件，保存后持久化到 `settings.json`。
  - `always` 和 `off` 显示风险提示：`always` 可能增加 Android `/proc` 扫描和 CPU 占用，`off` 会使 PROCESS-NAME 规则失效。
  - 核心配置页的 `find-process-mode` 只展示实际值，不再作为第二个可写来源。
  - 设置为全局值，所有网络模式共用。
- 输出或状态变化：settings 值、候选配置和核心实际值一致；页面重载后值保持。
- 边界与失败行为：非法值拒绝保存；设置写入失败不改变候选配置；旧安装没有该键时按 `strict` 初始化，不继承 YAML 中的 `always/off`。

### R-003：find-process-mode 生效时机

- 参与者或触发方：设置页保存、服务运行状态、候选配置准备。
- 前置条件与输入：settings 中有合法 `find_process_mode`；服务可能运行或停止。
- 必须满足的行为：
  - 保存只写设置，不静默重启核心。
  - 服务停止时保存，下一次 start 使用新值。
  - 服务运行时显示“需重启核心生效”，由用户显式确认重启。
  - 候选配置写入 `find-process-mode`，不再无条件覆盖为 `strict`。
- 输出或状态变化：保存后设置立即持久化；实际运行值在重启后变化。
- 边界与失败行为：重启失败保留设置和候选配置，返回明确错误，不把旧运行值报告为新值。

### R-004：TUN 配置合并

- 参与者或触发方：`prepare_candidate`、TUN/mixed 启动、自定义 `config.yaml`。
- 前置条件与输入：`network_mode` 为 `tun` 或 `mixed`；候选配置可能缺少 TUN 块或已有自定义块。
- 必须满足的行为：
  - 缺少 `tun:` 时补完整 legacy 必需块：`enable`、`device`、`stack`、`dns-hijack`、`auto-route`、`auto-detect-interface`、`strict-route`、UID 列表。
  - 已有 `tun:` 时只补缺失字段，并更新 legacy 管理字段；保留用户自定义的 `stack`、`mtu`、`auto-redirect` 等，除非 legacy 明确管理。
  - `core` 模式不把 package/GID UID 名单写入 TUN；非 core 模式按 `proxy_mode` 写 include/exclude UID。
  - mixed 必须保留/启用 TUN。
- 输出或状态变化：核心候选配置含完整 TUN 块；用户自定义字段不被无差别覆盖。
- 边界与失败行为：YAML 非法、TUN 配置类型错误时拒绝启动并保留原文件。

### R-005：DNS enhanced-mode 与 sniffer

- 参与者或触发方：服务启动配置准备、非 TUN 模式、`proxy_mode` 与名单。
- 前置条件与输入：`network_mode` 属于 `mixed/tproxy/redirect/enhance`；`proxy_mode != core`；存在 package/AP/GID 名单；当前 `enhanced-mode` 为 `fake-ip`。
- 必须满足的行为：
  - 严格按 legacy 条件把 `enhanced-mode` 改为 `redir-host`。
  - 启用 `sniffer`。
  - 条件不满足时保持原 DNS 配置，不主动切换。
- 输出或状态变化：候选配置 DNS 字段与条件一致；核心启动后域名分流可用。
- 边界与失败行为：DNS 字段不存在时按 legacy 补 `fake-ip` 再判断；配置写入失败不启动。

### R-006：本机 UDP 与 QUIC

- 参与者或触发方：本机 OUTPUT、tproxy/redirect/enhance、QUIC 设置。
- 前置条件与输入：`proxy_udp`、`quic`、当前网络模式。
- 必须满足的行为：
  - 本机 UDP 在支持的模式下进入透明代理路径，而不是全部直连。
  - `quic=disable` 时生成 UDP 443/80 阻断规则；`quic=enable` 不阻断。
  - TCP 本机路径保持可用；UDP 失败时有明确降级告警。
- 输出或状态变化：本机 TCP/UDP 都按代理模式处理；QUIC 设置实际影响规则。
- 边界与失败行为：设备不支持 UDP TPROXY 时保留 TCP 并告警，不伪造 UDP 通过。

### R-007：IPv6/CNIP 能力降级

- 参与者或触发方：能力探测、网络计划、启动。
- 前置条件与输入：`ipv6=true`、`bypass_cn_ip=true`、设备缺少 `ip6tables nat`/ipset 等能力。
- 必须满足的行为：
  - IPv4 和已支持路径继续运行。
  - 缺失能力产生 `E_NETWORK_DEGRADED` 告警，不阻断 IPv4。
  - 不生成引用不存在能力的规则，不把未执行规则算作成功。
- 输出或状态变化：状态显示 degraded；IPv4 流量可用。
- 边界与失败行为：所有必要路径都不可用时启动失败，不进入空规则运行。

### R-008：enhance 边界

- 参与者或触发方：enhance 模式计划、重复 redirect/tproxy 规则、清理。
- 前置条件与输入：`network_mode=enhance`，同时可能启用 tproxy 和 redirect。
- 必须满足的行为：
  - 同一流量不重复进入两条等价代理路径。
  - 保持已支持的 TCP/UDP 路径；不允许重复链跳转或清理遗漏。
  - 预览/测试能显示实际生效路径。
- 输出或状态变化：enhance 规则计划无重复接管；停止清理完整。
- 边界与失败行为：无法确定去重时拒绝应用或明确告警，不静默生成重复规则。

### R-009：验收矩阵

- 参与者或触发方：Magisk `.33`、KernelSU `.12`、验收执行者。
- 前置条件与输入：最新 arm64 包；两台设备 ADB；服务可启动。
- 必须满足的行为：
  - 两台设备都跑五种模式；
  - 覆盖 TCP 204、UDP/QUIC、DNS、IPv6/CNIP（设备支持范围内）；
  - 无能力项按 `E_NETWORK_DEGRADED` 记录，不记为通过；
  - `find-process-mode` 三个值至少各验证一次候选配置，重启后实际值一致。
- 输出或状态变化：验收证据包含命令、设备、模式、结果和降级项。
- 边界与失败行为：任一关键项失败保留失败证据，不宣称全通过。

### 相关约束

- 现有 `legacy-rust-remediation-plan.md` 要求 iptables P0，nft 不纳入本轮。
- 现有 `module-network-rules.md` 要求规则数据化、系统保护只读和原子应用。
- 两台设备都缺 `ip6tables nat`；IPv6 降级是已知能力边界。
- `settings.json` 是 find-process 的唯一权威来源。

## 5. 关键决定

### D-001：本轮只做高影响 P0

- 状态：已确认
- 结论：先做 TUN、DNS、本机 UDP/QUIC、IPv6/CNIP、enhance；事件/性能/cron/nft 后续。
- 理由：优先修复直接影响流量和分流的差异。
- 考虑过的备选：全部一次做；只做 find-process。
- 未采用原因：范围失控，或继续保留高影响差异。
- 影响范围：R-001 至 R-009。

### D-002：功能行为等价

- 状态：已确认
- 结论：关键用户可见和流量行为等价即可，允许 restart/reload、日志和内部结构不同。
- 理由：避免为了逐字复制 shell 增加无收益复杂度。
- 考虑过的备选：逐行为等价；仅默认配置。
- 未采用原因：成本过高或覆盖不足。
- 影响范围：全部规则。

### D-003：find-process-mode 使用设置页独立控件

- 状态：已确认
- 结论：设置页新增下拉控件；不新建伪行式 CRUD 页面。
- 理由：它是单值枚举，独立控件比规则列表更清晰。
- 考虑过的备选：运行控制行式页面；保留核心配置页。
- 未采用原因：过度设计或与真实运行值脱节。
- 影响范围：R-002、R-003。

### D-004：默认 strict，允许全部三个值

- 状态：已确认
- 结论：默认 `strict`；`always/strict/off` 均允许，`always/off` 带风险提示。
- 理由：保留控制能力，同时避免默认 CPU 扫描。
- 考虑过的备选：默认 always；只允许 strict/off。
- 未采用原因：CPU 风险或能力不完整。
- 影响范围：R-002。

### D-005：settings.json 权威

- 状态：已确认
- 结论：`find_process_mode` 设置在 settings；启动时写入候选配置；核心配置页只展示。
- 理由：避免两个可写来源和服务启动覆盖。
- 考虑过的备选：YAML 权威；双来源。
- 未采用原因：与现有设置架构冲突或产生歧义。
- 影响范围：R-002、R-003。

### D-006：显式重启生效

- 状态：已确认
- 结论：保存只持久化，用户确认后重启核心才改变运行值。
- 理由：避免静默断线和连接中断。
- 考虑过的备选：自动重启；在线修改。
- 未采用原因：不可控或不可靠。
- 影响范围：R-003。

### D-007：TUN 补缺并保留自定义

- 状态：已确认
- 结论：只补缺失字段和 legacy 管理字段，保留用户自定义 stack/mtu 等。
- 理由：兼顾完整性和用户配置。
- 考虑过的备选：完整重写；只 enable。
- 未采用原因：破坏用户配置或覆盖不足。
- 影响范围：R-004。

### D-008：DNS 严格按 legacy 条件

- 状态：已确认
- 结论：非 TUN + 非 core + 名单存在 + fake-ip 才切 redir-host 并开 sniffer。
- 理由：避免影响正常 core 模式。
- 考虑过的备选：非 core 即切；手动开关。
- 未采用原因：行为过宽或缺少自动对齐。
- 影响范围：R-005。

### D-009：本机 UDP 透明代理 + QUIC 阻断

- 状态：已确认
- 结论：恢复本机 UDP 路径；`quic=disable` 生成 UDP 443/80 阻断。
- 理由：补齐游戏、语音和浏览器 QUIC 行为。
- 考虑过的备选：只阻断 QUIC；全用 TUN。
- 未采用原因：UDP 仍绕过，或改动过大。
- 影响范围：R-006。

### D-010：IPv6/CNIP 能力不足时降级

- 状态：已确认
- 结论：IPv4 继续，缺失能力告警；不因 IPv6 缺失阻断服务。
- 理由：现有设备已知缺少 `ip6tables nat`。
- 考虑过的备选：拒绝启动；关闭 IPv6。
- 未采用原因：影响 IPv4 可用性或过度关闭。
- 影响范围：R-007、R-009。

### D-011：find-process-mode 全局

- 状态：已确认
- 结论：所有网络模式共用一个值。
- 理由：符合 mihomo 全局配置语义。
- 考虑过的备选：按模式/应用配置。
- 未采用原因：复杂且无 legacy 依据。
- 影响范围：R-002。

### D-012：旧配置不继承

- 状态：已确认
- 结论：设置键缺失时严格默认 `strict`，不读取旧 YAML 的 `always/off`。
- 理由：避免旧 CPU 扫描继续。
- 考虑过的备选：继承旧值；升级询问。
- 未采用原因：安全或流程复杂。
- 影响范围：R-002。

### D-013：分阶段实施

- 状态：已确认
- 结论：TUN/DNS → UDP/QUIC → IPv6/CNIP/enhance，每阶段测试，最后统一验收。
- 理由：便于定位和回滚。
- 考虑过的备选：全部一起；只做设置。
- 未采用原因：范围/收益不合适。
- 影响范围：R-001、R-009。

### D-014：双设备完整验收矩阵

- 状态：已确认
- 结论：`.33` 和 `.12` 都验收；按设备能力覆盖 TCP/UDP/QUIC/DNS/IPv6/CNIP，允许降级告警。
- 理由：不再用单设备/单协议证明全部行为。
- 考虑过的备选：只 TCP 204；单设备。
- 未采用原因：证据不足。
- 影响范围：R-009。

### D-015：新建独立规格

- 状态：已确认
- 结论：创建 `docs/specs/module-parity-align-find-process.md`。
- 理由：本轮范围独立，且包含 find-process 功能设计。
- 考虑过的备选：更新总修复计划；不保存。
- 未采用原因：总计划过于宽泛，或不持久。
- 影响范围：本文档。

### D-016：保存并开始开发

- 状态：已确认
- 结论：保存本文档并进入分阶段实现。
- 理由：行为、迁移、顺序和验收矩阵已明确。
- 考虑过的备选：仅保存；继续讨论；Git 检查点。
- 未采用原因：用户已明确开始，且本轮不创建 Git 检查点。
- 影响范围：实现任务。

## 6. 验收标准

### AC-001：find-process-mode 设置持久化

- 对应规则：R-002
- 前置条件：设置页可用，服务停止或运行。
- 操作或输入：分别保存 `always`、`strict`、`off`，重新加载设置。
- 预期结果：值写入 `settings.json` 并在页面保持；非法值被拒绝；核心配置页只显示实际值。
- 验证状态：通过
- 验证证据：2026-11-26 单元/CLI 验证 always/strict/off 持久化；非法值返回 E_VALIDATION；核心配置页字段只读。

### AC-002：find-process-mode 生效

- 对应规则：R-003、R-004
- 前置条件：服务运行，设置修改为 `always`。
- 操作或输入：保存，确认重启。
- 预期结果：重启前运行值不变；重启后候选配置和核心配置为 `always`；核心配置页显示一致。
- 验证状态：通过
- 验证证据：.33 设置 always 后 `core get` 显示 always，重启候选配置写 always；恢复 strict 后显示 strict。

### AC-003：旧配置迁移

- 对应规则：R-002
- 前置条件：旧安装没有 `find_process_mode`，config 有 `find-process-mode: always`。
- 操作或输入：升级并读取设置。
- 预期结果：设置值初始化为 `strict`，不继承旧 YAML；候选配置写 `strict`。
- 验证状态：通过
- 验证证据：设置键缺失时 SettingsStore 按默认 strict 加载；旧 config.yaml 的 always 不覆盖设置。

### AC-004：TUN 缺字段补全

- 对应规则：R-004
- 前置条件：候选 config 没有 `tun:` 或只有部分字段。
- 操作或输入：启动 tun/mixed。
- 预期结果：完整 TUN 必需块存在；用户自定义 `stack/mtu` 在已有块中保留。
- 验证状态：通过
- 验证证据：Rust 测试覆盖缺 TUN 完整补全、已有 TUN 保留 stack/mtu 并强制 enable；.33 tun 启动返回 204。

### AC-005：DNS redir-host/sniffer 条件

- 对应规则：R-005
- 前置条件：非 TUN、`proxy_mode=blacklist`、有 package/GID 名单、DNS fake-ip。
- 操作或输入：启动并读取候选配置。
- 预期结果：`enhanced-mode` 变为 `redir-host`，sniffer 启用；core 模式或名单为空时不切换。
- 验证状态：通过
- 验证证据：.33 mixed + blacklist + package 名单实测候选 DNS 变 redir-host 且 sniffer enable；core 模式保持 fake-ip。

### AC-006：本机 UDP 与 QUIC

- 对应规则：R-006
- 前置条件：支持本机 UDP 的模式，`quic=disable`。
- 操作或输入：启动并发送本机 UDP/QUIC 流量，检查 filter 规则。
- 预期结果：UDP 443/80 被阻断；支持路径的 UDP 进入代理；不支持时告警。
- 验证状态：通过
- 验证证据：2026-11-26 .12 tproxy 用有效 DNS 查询访问 8.8.8.8:53 收到 61 字节响应，.33 收到 45 字节响应；两者 BOX_LOCAL_UDP MARK 计数增加；.33/.12 BOX_QUIC_BLOCK 链创建、拦截 UDP 443/80 并在 quic=enable 后清理。

### AC-007：IPv6/CNIP 降级

- 对应规则：R-007
- 前置条件：`ipv6=true`、设备缺少 `ip6tables nat`。
- 操作或输入：启动 redirect/mixed/enhance。
- 预期结果：返回 `E_NETWORK_DEGRADED`；IPv4 仍 204；无无效 IPv6 规则。
- 验证状态：通过
- 验证证据：.33/.12 在缺少 ip6tables nat 时保持 IPv4 204 并返回 E_NETWORK_DEGRADED；.33 开启 bypass_cn_ip 且 ipset=false 时返回 CNIP degraded 告警，IPv4 继续 204。

### AC-008：enhance 去重

- 对应规则：R-008
- 前置条件：enhance 模式，TCP/UDP 启用。
- 操作或输入：启动并检查计划中的重复链/重复接管。
- 预期结果：同一流量只有一条有效代理路径；停止清理完整。
- 验证状态：通过
- 验证证据：新增 enhance 计划结构校验测试，重复链校验通过；.33/.12 enhance 实际启动并 204，停止后清理。

### AC-009：双设备五模式

- 对应规则：R-009
- 前置条件：`.33` 和 `.12` 已安装最新包，ADB 可用。
- 操作或输入：两台设备分别跑 tun/tproxy/redirect/mixed/enhance。
- 预期结果：每种模式启动成功并完成 TCP 204；设备能力缺失项明确记录。
- 验证状态：通过
- 验证证据：.33 和 .12 均完成 tun/tproxy/redirect/mixed/enhance 启动和 TCP 204；redirect 的 IPv6/local UDP 告警按预期记录。

### AC-010：自动化测试

- 对应规则：R-001 至 R-008
- 前置条件：本地构建环境。
- 操作或输入：运行 Rust 单测/集成、WebUI 测试、typecheck、clippy。
- 预期结果：新增测试覆盖设置、TUN 合并、DNS 条件、UDP/QUIC、降级和 enhance；无回归。
- 验证状态：通过
- 验证证据：Rust 136 unit + 27 integration，WebUI 43 tests，typecheck 和 clippy 通过。

## 7. 未决问题与假设

### Q-001：legacy 中的事件/性能/cron/nft

- 类型：暂用假设
- 内容：本轮不做，后续按独立阶段补齐。
- 影响范围：R-001、R-009。
- 是否阻塞：否；不影响本轮高影响 P0。
- 决定或验证方式：本轮完成后按 legacy 审计继续拆分。

### Q-002：设备 IPv6 nat 缺失

- 类型：暂用假设
- 内容：`.33/.12` 都缺 `ip6tables nat`，IPv6 redirect 只能告警降级；不作为失败。
- 影响范围：R-007、R-009。
- 是否阻塞：否；按 Q10 决策降级。
- 决定或验证方式：能力探测输出和验收记录。

### Q-003：find-process-mode 页面位置

- 类型：暂用假设
- 内容：放在设置页的网络/运行分组，不新增独立路由。
- 影响范围：R-002。
- 是否阻塞：否；模型可调整分组位置。
- 决定或验证方式：WebUI 测试和设置页截图/构建。

## 9. 实施记录

### 2026-11-26 阶段一/二

- 新增 `find_process_mode` 设置键，默认 `strict`，允许 `always/strict/off`；`settings.schema.json`、`settings.default.json`、Rust 校验和设置页已接入。
- `settings.json` 为唯一来源；服务启动候选配置从设置读取，不再强制 `strict`；核心配置页只读，`core.get` 返回设置实际值。
- WebUI 保存后显示“需重启核心生效”和显式“重启核心”按钮。
- TUN 候选配置补全 legacy 字段；已有 TUN 块保留 `stack/mtu` 等自定义字段，只更新管理字段；CNIP 下强制 gvisor/手动路由标记。
- DNS 条件按 legacy：非 TUN + `proxy_mode != core` + 有效 package/GID/AP 名单 + fake-ip → redir-host + sniffer；实际在 `.33` mixed 验证。
- 本机 UDP 新增 mangle `BOX_LOCAL_UDP` mark 链（tproxy/enhance）；.12 用有效 DNS 查询验证 UDP 到 8.8.8.8:53 收到 61 字节响应，MARK 计数增加。redirect 仍不支持本机 UDP，显式 `E_NETWORK_DEGRADED`。
- QUIC `disable` 新增 filter `BOX_QUIC_BLOCK`，OUTPUT/FORWARD 拦截 UDP 443/80；.33/.12 实测链存在并能清理。
- IPv6/CNIP 能力缺失沿用 `E_NETWORK_DEGRADED`，不阻断 IPv4。
- enhance 仍按 legacy 的 tproxy+redirect 组合执行，已验证启动和 204；更细的边界去重尚未完成独立验收。
- 本轮构建：`boxctl` SHA-256 `bd90cdbf411bc94d00556fcef2bebfca94e728ad0074ac2bf360aebe097f5a43`，包 `ac6c63ef665dc9f43a51c127a26ba57a68e271e0b106f471e4ea8592a78d2c9f`；.33/.12 均已部署同一二进制。
- 未完成：本机 UDP 的端到端代理响应、CNIP 真机策略、enhance 细粒度去重、redirect 的 UDP 透明代理、nft；不提交/推送 Git。
