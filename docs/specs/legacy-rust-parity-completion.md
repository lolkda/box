# Legacy Shell 到 Rust `boxctl` 完整行为对齐与未迁移能力收口计划

## 1. 文档信息

- 文档标识：legacy-rust-parity-completion
- 规格状态：已确认
- 最后更新：2026-11-27
- 实现状态：已实现
- 验证状态：通过
- 关联文档：
  - [legacy-rust-remediation-plan.md](legacy-rust-remediation-plan.md)（早期 P0 计划草稿；本文为独立新建的全量迁移与实测 Bug 收口规格，不覆盖原文档，覆盖了原文档未纳入的 cgroup/taskset/performance_mode/bond/crontab.cfg/inotify、纯 mangle TPROXY、candidate 热重载、`root:net_admin` 精确放行与双机验收要求）
  - [legacy-rust-behavior-audit.md](legacy-rust-behavior-audit.md)
  - [legacy-parity.md](legacy-parity.md)
  - [module-network-rules.md](module-network-rules.md)
  - [module-parity-align-find-process.md](module-parity-align-find-process.md)

## 2. 目标与范围

### 背景

通过静态逐行审计与 KernelSU 真机 `192.168.1.12`（OnePlus PKX110 / Android 16 / API 36 / Linux 6.6.89）及 Magisk 真机 `192.168.1.33`（Android 12 / API 31 / Linux 4.19.157）只读检查，确认当前 Rust `boxctl` 已完成主链路骨架、UDS Daemon 与单测，但仍存在一批会直接导致本机 UDP 丢包、TCP DNS 劫持失效、自定义网络规则报错回滚、候选配置 `dns.sniffer` 层级写错、事件自控语义反转、定时调度器永久卡死、聚合订阅合并反转与工具路径矛盾等 P0/P1 缺陷，且部分 legacy 能力尚未迁移。

### 目标

- 修复网络平面、服务配置准备、进程身份、事件自控、定时调度、订阅/规则模板/工具、WebUI 契约与打包脚本中的全部已识别 P0/P1 缺陷；
- 将 legacy 尚未迁移的运行时能力全部迁回 Rust `boxctl`：模块 `disable` 监控（`box.inotify`）、动态本机 IP 防回环（`net.inotify` `LOCAL_IP_V4/6`）、资源与性能调优（`cgroup_memcg` / `cgroup_blkio` / `taskset_cpu` / `performance_mode` / `bond0` / `bond1`）及 `box/crontab.cfg` 兼容执行；
- 修复 `nft_plan` 代码级缺陷并补齐单元测试；
- 在 `.12`（KSU）和 `.33`（Magisk）双机上完成 iptables 全量真实流量与功能矩阵验收，并产出干净无杂项的模块安装包。

### 非目标

- 不恢复 eBPF 模式（`network_mode=ebpf` / `cnip_mode=ebpf` 继续保持移除与自动迁移）；
- 不改变 `boot_auto_start` 默认值 `"false"`；
- 不把 `nft` 纳入真机验收门禁（两台真机均 `nft=false`，本次仅代码修复与单测覆盖）；
- 仅在已确认范围内完成本地开发与验收，不自动执行新的 Git 提交或推送。

## 3. 参与深度与委托

- 参与深度：架构与设计
- 局部例外：无
- 需要用户确认：目标范围、未迁移能力去留、核心身份/网络路径/重载策略/接口契约/事件与调度器语义/下载器出站策略、文档新建及 Git 检查点方式（均已在 Q1–Q13 确认）。
- 模型自主决定：内部模块与结构体拆分、函数命名、单测与 golden 组织、具体规则链构建细节、不改变已确认外部契约的重构实现。
- 当前执行授权：用户已选择“新建独立草稿 + 先 Git 保存当前状态（仅本地提交），再保存并开始开发”。
  - Git 前置检查点方式：**仅本地提交（不推送）**；
  - 基线提交：`d1458ca26324e0f5d30a667c03496bee0774fd78`（`chore: checkpoint pre-parity-completion state`，分支 `feat/rust-boxctl-migration`）；
  - 纳入范围：`box/settings*.json`、`box/network-rules.default.json`、`docs/specs/*`、`rust/crates/boxctl/src/*`、`tools/legacy-oracle/*`、`webui/src/*`、`webroot/*`；
  - 未纳入改动（保留于工作树未追踪）：`AutoADBWiFi.zip`、`box-2.0.0.zip`、`box/bin/boxctl`；
  - 推送结果：不推送。
  - 当前已获授权在本规格范围内完成文档落盘、代码实现、测试、交叉编译与 `.12`/`.33` 双机真机验收；不自动授权再次提交或推送后续改动。

## 4. 行为要求与约束

### R-001：核心运行身份、FD 上限与防回环放行

- 参与者或触发方：服务启动与透明代理规则应用。
- 前置条件与输入：设备具备 root 权限；核心二进制与配置有效。
- 必须满足的行为：`mihomo` 以 `uid=0, gid=3005 (root:net_admin)` 启动并尝试抬升 `RLIMIT_NOFILE`（目标 `1000000`，内核拒绝时降级记录不阻断）；iptables 本地出站链精确旁路 `--uid-owner 0 --gid-owner 3005`（不再放行所有 `uid 0`）；`filter/OUTPUT` 拒绝核心直连 loopback `redir_port`/`tproxy_port` 防环路；`whitelist` 模式显式放通 `uid 0` 与 Android `uid 1052 (dns/netd)`。
- 输出或状态变化：`/proc/<pid>/status` 中 `Gid` 为 `3005`；普通 root 与非 root 进程均受透明代理与黑白名单控制。
- 边界与失败行为：若非 root 测试环境无法 `setgid(3005)`，单元测试可跳过或优雅回退；实机 root 下必须生效。

### R-002：五种网络模式正确语义与 IPv4/IPv6 全栈对齐

- 参与者或触发方：`network_mode` 切换为 `tun`、`tproxy`、`redirect`、`mixed`、`enhance` 之一并启动服务。
- 前置条件与输入：`settings.json`、`network-rules.json`、`package.list.cfg`、`gid.list.cfg`、`ap.list.cfg`、内核能力探测结果。
- 必须满足的行为：
  - `tproxy`：纯 `mangle` 路径，本机与转发 TCP+UDP（IPv4+IPv6）均走 `mangle/OUTPUT MARK 0x01000000` + `ip rule pref 100 table 2024` + `mangle/PREROUTING TPROXY`；**移除 `BOX_TPROXY` 中 `mark 0x01000000 RETURN` 的错误规则**；`PREROUTING` 仅接管 `-i lo` 与 `ap.list.cfg` `allow` 接口并绕过 `ignore` 接口及按 `mac_filter`/`mac_mode`/`macs_list` 过滤热点客户端源 MAC。
  - `redirect`：`nat` TCP REDIRECT（PREROUTING + OUTPUT），不向 `nat` 表硬编码 `-m socket --transparent`（仅在探测支持 `xt_socket` 且仅限 `mangle` 使用）。
  - `mixed`：候选配置启用 TUN；规则为 `filter/FORWARD` 放行动态读取的 `tun.device` + `sysctl` + `redirect` TCP；仅当 `bypass_cn_ip=true` 时建立 TUN bypass 策略路由。
  - `enhance`：`nat` 仅管 TCP REDIRECT，`mangle` 严格只管 UDP TPROXY（屏蔽所有 TCP 规则），互不双重劫持。
  - `tun`：动态读取 `config.yaml` 中 `tun.device`；配置 `filter/FORWARD` 与 `sysctl`（`ip_forward=1`, `rp_filter=2`）；仅在 `bypass_cn_ip=true` 时才建立 `BOX_TUN_BYPASS_PRE/OUT` 与 table 2025（含 IPv6），否则交由 mihomo `auto-route: true`；`rollback_rules` 正确回滚 `ip route replace`。
  - `ipv6=false` 时禁用 IPv6 sysctl 并添加 `ip -6 rule add unreachable pref 100` 防泄漏，停服或开启 IPv6 时恢复。
  - 本机出站链必须包含 CNIP 绕过、`force_proxy_cidrs`、Fake-IP ICMP/v6/TUN 优先规则；`cnip_force_proxy_apps` 非空时插在 CNIP bypass 之前，不得屏蔽常规本机代理规则；启动时在 `ipset` 可用时自动按 `bypass_cn_ip_v4`/`v6` 创建并导入 `cn_ip.txt`/`cn_ipv6.txt`。
- 输出或状态变化：五种模式规则无重复、顺序正确、停止后清理彻底。
- 边界与失败行为：无 `ipset` 或无 `ip6tables nat` 时明确降级并告警，不产生无效命令；`status.get` 返回与 `service.start` 一致的规则计划。

### R-003：DNS 劫持、自定义网络规则与停止清理安全

- 参与者或触发方：DNS 查询、用户配置自定义 `network-rules`、或执行 `service stop`。
- 前置条件与输入：`dns_hijack_mode`、`mihomo_dns_forward`、`dns_hijack_tcp/udp`、`dns.listen`（默认回退 `1053`）、`network-rules.json`。
- 必须满足的行为：
  - 尊重 `dns_hijack_mode`（`disable`/`tproxy`/`redirect`）与 `mihomo_dns_forward`；在 `nat/OUTPUT` 与 `nat/PREROUTING` 中，DNS 链必须位于常规 REDIRECT 链**之前**（保证 TCP 53 命中 `BOX_DNS` 而非被截到 `redir_port`）；支持 IPv6 DNS 劫持与 `tproxy` mangle DNS 分流。
  - 修复自定义 `network-rules`：`filter` 表仅挂 `INPUT`/`FORWARD`/`OUTPUT`（严禁挂不存在的 `filter/PREROUTING`）；`user_rules_for` 按链方向区分本地 `OUTPUT` 与入站 `PREROUTING`（修复 `uids`/`gids`/`-i`/`-o`）；端口范围 `start-end` 转译为 iptables `start:end`；去重 `240.0.0.0/4` 与 `5555`。
  - `stop_internal` 清理规则失败时不得吞错，必须中止停服并保留核心运行（遵守 `module-service-stop-cleanup` R-002）；清理顺序先删 `FORWARD` 跳转再 `-X BOX_QUIC_BLOCK`；`cleanup` 后无链/规则/路由残留，并兜底清理孤儿 `mihomo` 进程。
- 输出或状态变化：TCP/UDP DNS 正常命中 mihomo DNS；含端口范围、UID/GID、Reject 的自定义规则可正确生效与回滚。
- 边界与失败行为：规则下发任一步失败时全量逆序回滚，不留半套链。

### R-004：候选配置准备与热重载

- 参与者或触发方：`service start`、`service reload`、`sub update/enable/disable`、`rule-profile apply/use`、`geox`、`upkernel`。
- 前置条件与输入：`auto_modify_config`、`settings.json`、`mihomo/config.yaml`、各 list 文件。
- 必须满足的行为：
  - `auto_modify_config=false` 时，`prepare_candidate`、`enable_tun_candidate`、`disable_tun_candidate` 均不得修改配置字段；
  - `prepare_dns_block` 必须把 `sniffer.enable: true` 写入**顶层 `sniffer:`**（而非 `dns.sniffer`），仅在已有 `dns:` 时补全；`proxy_lists_configured` 仅将 `ap.list.cfg` 中的 `ignore` 行计入触发条件；`proxy_uids` 严格 trim 并跳过注释/`mode:` 行；
  - 订阅/规则模板/GeoX 变更在核心运行时，重新生成最新 candidate 并通过 `PUT /configs?force=true` 传入 `{"path": "<candidate>"}` 执行热重载（不断流）；核心未运行时返回 `reloaded: false` 不误拉起核心；仅 `upkernel` 替换二进制后走 `service::restart`；
  - 自动清理过期的 `run/candidates/` 与 `run/tasks/`（各保留最近 20 条）。
- 输出或状态变化：配置热生效且不断流，无候选目录无限堆积。
- 边界与失败行为：重载失败返回 `E_RELOAD` 且保留旧运行态。

### R-005：模块 `disable` 监控与动态本机 IP 防回环（`inotify` 原生迁移）

- 参与者或触发方：Root 管理器切换模块禁用开关，或设备网络地址/路由变化。
- 前置条件与输入：`/data/adb/modules/box_for_root/disable` 状态、当前网卡 IPv4/IPv6 地址。
- 必须满足的行为：
  - 启动前检查模块 `disable` 文件：存在时拒绝开机自启；`daemon` 运行期间持续监控 `disable` 文件创建/删除，禁用时立即停服+清规则，重新启用时按配置恢复；
  - `daemon` 监听网络接口/地址变化，动态维护 `LOCAL_IP_V4`/`LOCAL_IP_V6` 绕过链（采集当前非 TUN 接口的 IPv4 与 global IPv6），网络切换时热刷新链内容防回环而不重启核心。
- 输出或状态变化：禁用模块即断规则停核，切换 Wi-Fi/5G 时新本机 IP 自动绕过。
- 边界与失败行为：刷新失败记日志并在下次网络变更/心跳重试，不影响核心运行。

### R-006：资源调优、性能模式与 `crontab.cfg` 全量迁移

- 参与者或触发方：服务启动、网络规则构建、定时任务调度、`boxctl` 命令行。
- 前置条件与输入：`cgroup_memcg`/`memcg_limit`、`cgroup_blkio`/`weight`、`taskset_cpu`/`allow_cpu`、`performance_mode`、`bond0`/`bond1`、`box/crontab.cfg`。
- 必须满足的行为：
  - `taskset_cpu=true` 时按 `allow_cpu` 掩码设置核心进程 CPU 亲和性（`sched_setaffinity`）；
  - `cgroup_memcg=true` / `cgroup_blkio=true` 时在核心启动后探测 cgroup v1/v2 挂载点并写入内存上限/IO 权重与 PID，不支持时安全降级记录警告并避免阻塞启动；
  - `performance_mode=true` 时在 iptables 路径启用 conntrack REPLY/ESTABLISHED 快速放行与 `DIVERT`（仅在内核支持 `xt_socket` 时）；
  - 迁回 `bond0`/`bond1` 网络接口调优子命令与 `check`（`mihomo -t`）子命令；
  - 调度器改用**设备本地时区**，修复首次开启不立即触发、修复超 `grace` 窗口自动推进 `last_run` 防止永久卡死，并解析执行 `box/crontab.cfg`（旧 `box.tool` 命令映射至 `boxctl` 任务，其余受控执行）。
- 输出或状态变化：全部设置项均有真实运行时消费，无“界面能点但后台不读”的死开关。
- 边界与失败行为：不支持的 cgroup 控制器或网卡优雅降级并留日志，不破坏代理启动。

### R-007：事件自控语义对齐

- 参与者或触发方：开启 `enable_network_service_control=true` 且网络状态变化。
- 前置条件与输入：当前传输类型（Wi-Fi / 蜂窝 / 无网）、SSID/BSSID、`use_module_on_wifi`、`use_module_on_wifi_disconnect`、`use_ssid_matching`、`use_wifi_list_mode`、`wifi_ssids_list`、`wifi_bssids_list`。
- 必须满足的行为：
  - 修复 `use_module_on_wifi_disconnect` 语义：`true` 表示断开 Wi-Fi 后继续启用模块，`false` 表示断开 Wi-Fi 时停止模块；
  - 修复 `validated` 门控：断开 Wi-Fi 切到蜂窝时不得因无 SSID 把 `validated` 置 `None` 而阻断决策；
  - Wi-Fi 连接下命中黑名单、不在白名单或 `use_module_on_wifi=false` 时应停止运行中的服务；停机状态切入允许网络时应直接自动启动（不强制要求先存在 `event_stop` 标记）；
  - 手动启停仅在当前网络状态生效覆盖，下次网络状态发生真实切换时自动恢复事件决策，不再永久锁死。
- 输出或状态变化：网络切换启停结果与 `legacy/box-scripts/ctr.utils` 一致。
- 边界与失败行为：网络探测瞬时未知时保持现状并防抖，不误启停。

### R-008：订阅、规则模板、下载器与更新工具完整性

- 参与者或触发方：订阅增删改启停更新、规则模板拉取/应用、GeoX/内核/面板/CNIP 更新。
- 前置条件与输入：订阅源、规则模板、`settings.json`、`config.yaml`。
- 必须满足的行为：
  - **下载器（Q10）**：优先以 `gid=3005 (net_admin)` + 直连真实 DNS（避开 fake-ip）直连下载，失败且核心在运行时自动回退经代理重试，全程保持严格 TLS 校验；
  - **订阅**：聚合合并以新 `source_def` 覆盖同名字段（保留源 `interval`、`health-check`、`override`/`filter`/`exclude-filter`、清除旧 `stale-field`）；聚合部分失败保留已有可用 provider 与 manifest；写入 `config.yaml` 前跑 `mihomo -t` 校验；`sub disable` 检查 `proxy-groups[].use` 引用保护；`sub enable` 恢复 `provider-defs` 缓存并在缺失本地文件时仍写回定义；`sub set` 改名同步 `config.yaml` 与聚合 manifest/defs；Base64 解码支持多行折行与无 `=` padding；
  - **规则模板**：无 `pending.yaml` 时 `use` 自动先 `fetch` 再应用；拒绝缺少 `proxy-groups`/`rule-providers`/`rules` 键的模板；`fetch` 阶段合并 DNS 引用 provider 并正确计数；支持旧单文件 `rule-profile.url` 迁移为 `default`；
  - **更新工具与接口**：`upxui` 统一部署到 `./dashboard` 并同步 `external-ui: ./dashboard`；`upkernel` 支持 `mihomo_stable=disable` 拉取 `Prerelease-Alpha`、修复 `x86_64` 为 `linux-amd64`、设 `cancelable=false`、更新后自动 `restart` 运行中核心；`geo::update` 第二文件 rename 失败恢复旧 `Country.mmdb` 而非删除；`upcnip` 仅在服务运行且 `ipset` 可用时执行 `ipset restore`；`config::validate` 补 `MATCH,` 校验；`core.get` 保留 `external-ui` 原始大小写并与 `settings.json` 对齐 `find-process-mode`；`settings.get` 不返回 `preview` 并清理 WebUI 残留引用；`logs::get` 支持回退 `tool.log`；任务取消终止子进程树；
  - **打包与 oracle**：[build.sh](../../build.sh) 排除 `tools/*` 与非模块 `.zip`；更新 [oracle.py](../../tools/legacy-oracle/oracle.py) 兼容 `find_process_mode` 恢复自动差分。
- 输出或状态变化：全量订阅/规则/更新操作满足 legacy 测试断言与前后端契约。
- 边界与失败行为：任何校验或写入失败保留最后一次可用配置并输出明确错误码。

### R-009：`nft_plan` 代码级缺陷修复

- 参与者或触发方：仅 `nft` 可用时的规则计划生成。
- 前置条件与输入：`caps.iptables=false && caps.nft=true`。
- 必须满足的行为：TUN 模式使用 `nft` 时先建 `inet box` 表；`mixed` 编排为 TUN + `redirect`；移除 `box_tproxy` 中前置的 `meta mark return` 并为 TPROXY 规则补 `meta mark set 0x01000000`；补齐本地 `OUTPUT` 代理、`proxy_mode` UID/GID、DNS 转发与 `cleanup_stale` 删表。
- 输出或状态变化：Rust 单元测试覆盖全部 5 种 `nft` 模式与清理计划。
- 边界与失败行为：本轮不声称已通过 `nft` 真机验收。

### 相关约束

- 不恢复 eBPF，不将 `legacy/` 打入最终发布包。
- 不破坏现有 `.12`/`.33` 真机用户配置与数据；测试中若临时切模式/改设置，验收结束后恢复用户原有配置（`.12` 为 `tproxy` + `blacklist`，`.33` 为原配置）并确认服务正常运行。
- 任何新修复先补可复现的 Rust 测试或验证脚本，再落地修复。

## 5. 关键决定

### D-001：资源调优类与 `crontab.cfg` 全部迁回 Rust 实现

- 状态：已确认
- 结论：`cgroup_memcg`/`memcg_limit`、`cgroup_blkio`/`weight`、`taskset_cpu`/`allow_cpu`、`performance_mode`、`bond0`/`bond1`、`box/crontab.cfg` 全部在 Rust 中实现，不删除对应设置项。
- 理由：用户明确要求完整迁回，消除“设置存在但后台不读”的假开关并保持老用户习惯。
- 考虑过的备选：直接移除死开关；仅保留部分实现。
- 未采用原因：用户选择全部迁回实现。
- 影响范围：R-006、AC-006、AC-010。

### D-002：原生迁移模块 `disable` 监控与动态本机 IP 防回环

- 状态：已确认
- 结论：在 `boxctl daemon` 内原生实现 `box.inotify`（模块 `disable` 监控）与 `net.inotify`（动态 `LOCAL_IP_V4`/`LOCAL_IP_V6` 绕过热更新）。
- 理由：Root 管理器模块禁用必须立刻停服清规则；双栈移动设备切网后必须动态更新本机 IP 绕过防回环。
- 考虑过的备选：仅迁移 `disable` 监控；两项暂不迁移。
- 未采用原因：两者均直接影响日常可用性与防环路安全。
- 影响范围：R-005、AC-005。

### D-003：核心进程恢复 `root:net_admin (0:3005)` 精确放行

- 状态：已确认
- 结论：`mihomo` 以 `uid=0, gid=3005` 启动并抬升 `RLIMIT_NOFILE`，iptables 精确旁路 `0:3005` 并补 loopback 端口防环路与白名单 `uid 1052`。
- 理由：避免所有 root 进程绕过代理及测试假象，与 legacy Android 权限模型对齐。
- 考虑过的备选：保持 `root:root` 全放行 `uid 0`。
- 未采用原因：导致 root 流量无法代理且掩盖真机测试结果。
- 影响范围：R-001、R-002、AC-001、AC-002。

### D-004：`tproxy` 回归 Legacy 纯 `mangle` TPROXY

- 状态：已确认
- 结论：`tproxy` 模式本机与转发 TCP+UDP（IPv4+IPv6）全部走 `mangle` MARK + TPROXY；`enhance` 严格 TCP `nat` + UDP `mangle`。
- 理由：`.12` 和 `.33` 均无 `ip6tables nat`，纯 `mangle` 是双栈本机透明代理且区分 `tproxy`/`enhance` 语义的唯一干净架构。
- 考虑过的备选：IPv4 本机 TCP 继续走 `nat REDIRECT`。
- 未采用原因：破坏 `tproxy` 语义且割裂 IPv4/IPv6 实现。
- 影响范围：R-002、AC-002。

### D-005：新 candidate + Mihomo API 热重载

- 状态：已确认
- 结论：订阅/规则模板/GeoX 变更后重生成最新 candidate 并以 `PUT /configs?force=true` 传新 `path` 热重载；仅 `upkernel` 走 `service::restart`。
- 理由：兼得 candidate 隔离、不断流秒级生效和与 legacy/WebUI 语义一致。
- 考虑过的备选：全部继续走 `service::restart`。
- 未采用原因：每次订阅/规则更新都会断流并耗时 5 秒以上。
- 影响范围：R-004、R-008、AC-004、AC-007。

### D-006：`find-process-mode` 单真源与 `githubtoken.preview` 清理

- 状态：已确认
- 结论：`find-process-mode` 以 `settings.json` 为唯一真源并同步前后端展示与写入，且项目默认配置改为 `off`（避免 Android 16 上 Mihomo `strict`/`always` 触发高 CPU 占用，同时不影响模块自身的内核级分应用黑白名单代理）；不返回 `githubtoken.preview` 并清理 WebUI 残留引用。
- 理由：消除前后端失配与“改完读回旧值”的困惑，同时保持 token 不泄露片段的安全原则。
- 考虑过的备选：恢复返回 token preview 及 `core.get` 不覆写。
- 未采用原因：违背已有 `find_process_mode` 规格与不回传 token 片段的安全收敛。
- 影响范围：R-004、R-008、AC-008。

### D-007：事件自控对齐 Legacy 决策与调度器本地时区兼容 `crontab.cfg`

- 状态：已确认
- 结论：修复 `use_module_on_wifi_disconnect`、`validated`、Wi-Fi 黑白名单停止及手动覆盖不永久锁死；调度器改本地时区、修复超宽限期卡死并支持 `crontab.cfg`。
- 理由：恢复用户日常自动化体验并消除调度器休眠 1 小时后永久死亡的致命 Bug。
- 考虑过的备选：手动永久锁定 + 仅白名单 cron。
- 未采用原因：不符合 legacy 行为与 `crontab.cfg` 迁回需求。
- 影响范围：R-006、R-007、AC-006。

### D-008：下载器优先 `gid=3005` 直连绕过，失败回退代理

- 状态：已确认
- 结论：下载任务先以 `gid=3005` + 直连真实 DNS 尝试绕过透明代理与 fake-ip，失败且核心在运行时自动回退经代理重试，保持严格 TLS。
- 理由：既保留坏节点时不停服自救更新订阅的能力，又兼容未开 `ghproxy` 时需走代理访问海外源的场景。
- 考虑过的备选：始终纯直连；始终走默认路由。
- 未采用原因：纯直连在无镜像访问 GitHub 时易失败；纯默认路由在节点失效时死锁。
- 影响范围：R-008、AC-007。

### D-009：双机协同验收 iptables + 顺手修复 `nft_plan` 代码 Bug

- 状态：已确认
- 结论：`.12`（KSU/API 36）与 `.33`（Magisk/API 31）双机完成 iptables 真实流量与功能矩阵验收；同步修复 `nft_plan` 明显代码 Bug 并补单测（不计入真机门禁）。
- 理由：两台机互补覆盖 Android 16/KSU 与 Android 12/Magisk/cgroup v1，同时避免 `nft_plan` 遗留明显错误。
- 考虑过的备选：仅验 `.12`；不动 `nft`。
- 未采用原因：覆盖不完整且 `nft_plan` 已知必败错误修复成本低。
- 影响范围：R-009、AC-009、AC-010。

## 6. 验收标准

### AC-001：核心身份与放行规则（双机）

- 对应规则：R-001
- 前置条件：`.12` 和 `.33` 安装新 `boxctl` 并启动服务。
- 操作或输入：检查 `/proc/<mihomo_pid>/status`、`limits`、`iptables -S`，分别以 `uid 0`（非 `gid 3005`）、`uid 2000` 和黑名单 UID 发起连接。
- 预期结果：`mihomo` 运行在 `Uid=0, Gid=3005`；iptables 仅放行 `0:3005`；普通 `uid 0` 与 `uid 2000` 均经代理，黑名单 UID 绕过；存在 loopback 端口 `REJECT` 规则。
- 验证状态：通过
- 验证证据：`.12`（PID 24215）与 `.33`（PID 11795）均实测 `Uid: 0 / Gid: 3005`、`Max open files: 1000000`；`BOX_TPROXY_LOCAL`/`BOX_DNS` 精确匹配 `--uid-owner 0 --gid-owner 3005`；`filter/OUTPUT` 存在 `127.0.0.1/32 --dport 9898 --uid-owner 0 --gid-owner 3005 REJECT`；`.12` 实测 `uid 2000`、`uid 0 (gid 0)` 均经代理返回 204，黑名单 `uid 10385` 命中 `RETURN` 绕过（13 包）。

### AC-002：五种模式与双栈真实流量矩阵（双机）

- 对应规则：R-002
- 前置条件：`.12` 和 `.33` 上依次切换 `tproxy`、`redirect`、`mixed`、`enhance`、`tun` 并启动。
- 操作或输入：检查实际 iptables/ip6tables/ip rule/ip route；在 `tproxy` 下发起非 root TCP、UDP、IPv6 流量并查 iptables 计数器与 mihomo `/connections`；检查 `enhance` mangle 无 TCP 规则；检查 `tun`/`mixed` 的 `filter/FORWARD`、`sysctl` 与停服后清理。
- 预期结果：`tproxy` 本机 TCP+UDP 实际命中 `TPROXY 9898`（计数器 > 0，mihomo `/connections` 可见 `TProxy` TCP/UDP）；`enhance` 仅 UDP 走 TPROXY、TCP 走 REDIRECT；`mixed` 启用 TUN + REDIRECT；`ipv6=false` 生效 `ip -6 rule unreachable`；每种模式停服后链、规则、table 2024/2025 路由全部清零。
- 验证状态：通过
- 验证证据：`.12` 实测 `tproxy` 模式下 `BOX_TPROXY` `-i lo` 命中 `tcp 49 pkts / udp 1 pkt`、mihomo `/connections` 可见 `tcp/TProxy/9898` 与 `udp/TProxy/9898`，`ip6tables` 生成完整 `BOX_TPROXY` + `BOX_TPROXY_LOCAL` + `LOCAL_IP_V6`；`redirect`/`enhance`/`mixed`/`tun` 均返回 Google HTTPS 204，`enhance` mangle `tcp_tproxy=0 / udp_tproxy=7`，`mixed`/`tun` `filter/FORWARD` 放行 `meta`（2 条）且 `ip_forward=1`，每次停服后 `chains=0 ip_rules=0`；`ipv6=false` 生效 `100: from all unreachable`；`.33` 实测 `tproxy`（`BOX_TPROXY` `-i lo` `tcp 204 pkts / udp 1 pkt`、`uid 2000` & `uid 0` 均返回 Google 204）与 `tun`（Google 204）及全 5 模式停服清零（`pre-leftovers=0`）。

### AC-003：DNS 劫持、自定义网络规则与停止清理保护

- 对应规则：R-003
- 前置条件：服务运行中，配置含端口范围（如 `8000-8010`）、`uids` 及 `Forward/Reject` 的自定义 `network-rules`；测试 `dns_hijack_mode` 切换与 TCP/UDP 53 查询。
- 操作或输入：发起非 root TCP:53 与 UDP:53 查询查看 `BOX_DNS` 计数器；应用自定义规则并验证无报错回滚；通过单测验证清理失败时 `stop_internal` 保留核心。
- 预期结果：TCP:53 与 UDP:53 均命中 `BOX_DNS`（不被 `BOX_LOCAL_REDIRECT` 截胡）；自定义端口范围/UID/Reject 规则全部正常下发并清理；清理失败单测通过。
- 验证状态：通过
- 验证证据：`.12` `redirect` 模式实测 `nat/OUTPUT` 顺序为先 `BOX_DNS` 后 `BOX_LOCAL_REDIRECT`，`uid 2000` TCP:53 实测命中 `BOX_DNS tcp dpt:53 -> REDIRECT 1053`（1 pkt）；`network-rules apply` 下发 `8000-8010` Forward Reject 与 `uid 10999` Local Bypass 成功，实测生成 `filter/BOX_FILTER_FWD --dport 8000:8010 -j REJECT` 与 `nat/BOX_LOCAL_REDIRECT --uid-owner 10999 -j RETURN`；单测 `service::tests::stop_aborts_and_keeps_core_when_cleanup_fails` 通过。

### AC-004：候选配置准备与 API 热重载

- 对应规则：R-004
- 前置条件：源 `config.yaml` 无顶层 `sniffer` 或 `auto_modify_config=false`；服务运行中与停止中各测一次。
- 操作或输入：触发候选准备并检查 YAML 结构；运行中执行 `sub update` / `rule-profile use --reload`；停止中执行 `rule-profile use --reload`。
- 预期结果：`sniffer.enable` 位于顶层而非 `dns.sniffer`；`auto_modify_config=false` 不改写候选；运行中热重载不更换 `mihomo` PID、不断流；停止态返回 `ok: true, reloaded: false` 不拉起核心；`candidates/` 数量受 20 条上限清理。
- 验证状态：通过
- 验证证据：`.12` 实测候选配置 `.dns.sniffer == null`、`.sniffer.enable == true`、`candidates/` 自动清理为 20 个；`.12` 与 `.33` 均通过 `PUT /configs?force=true` payload 热重载返回 HTTP 204，且 `mihomo` PID 不变（`.12` `6667 -> 6667`、订阅更新 `17687 -> 17687`；`.33` `11795 -> 11795`）；单测覆盖 `auto_modify_config=false` 与停机态 `reloaded: false`。

### AC-005：模块 `disable` 监控与动态本机 IP 绕过

- 对应规则：R-005
- 前置条件：真机服务运行中。
- 操作或输入：在测试窗口临时创建 `/data/adb/modules/box_for_root/disable` 再删除；检查 `LOCAL_IP_V4`/`LOCAL_IP_V6` 链中是否包含当前网卡 IPv4/IPv6 地址。
- 预期结果：创建 `disable` 后服务与规则在数秒内自动停止/清理；删除 `disable` 后按策略恢复；`LOCAL_IP_V4`/`LOCAL_IP_V6` 包含设备实时本机地址并在地址变化时刷新。
- 验证状态：通过
- 验证证据：`.12` 实测创建 `/data/adb/modules/box_for_root/disable` 后 6 秒内自动 `running=false`，删除 `disable` 后 2 秒内自动恢复 `running=true`；实测 `LOCAL_IP_V4` 自动加载 `192.168.1.0/24`（已命中 430 包）与 `172.30.225.139`，`LOCAL_IP_V6` 自动加载双公网 IPv6（`2408:...` / `2409:...`）。

### AC-006：资源调优、性能模式、事件自控与定时调度器

- 对应规则：R-006、R-007
- 前置条件：单测 + `.33`（有 `/dev/memcg`、`/dev/blkio`、`/dev/cpuset`）与 `.12` 真机验证。
- 操作或输入：测试 `taskset_cpu`（核查 `/proc/<pid>/status` `Cpus_allowed_list`）、`cgroup_memcg`/`cgroup_blkio`、`performance_mode`（`DIVERT`/conntrack 规则）、`bond0`/`bond1`、`events` 决策单测（含 `use_module_on_wifi_disconnect`、断网 `validated`、黑白名单停止、手动覆盖恢复）、`schedule` 单测（本地时区、超 `grace` 推进、`crontab.cfg` 解析）。
- 预期结果：全部单元测试通过；`.33` 真机验证 cgroup/taskset/performance_mode 生效，`.12` 不支持的 cgroup 控制器安全降级不阻断启动。
- 验证状态：通过
- 验证证据：`.33` 实测 `cgroup_memcg=true (128M)` 写入 `/dev/memcg/box/memory.limit_in_bytes=134217728`、`cgroup_blkio=true (800)` 写入 `/dev/blkio/box/blkio.weight=800` 且 `/proc/15712/cgroup` 显示 `4:memory:/box` 与 `1:blkio:/box`；`.12` 与 `.33` 均实测 `taskset_cpu=true (0-3)` 生效 `Cpus_allowed_list: 0-3`、`performance_mode=true` 生成 `DIVERT` + `conntrack --ctdir REPLY`；`.12` 实测 `boxctl check` 与 `boxctl bond0` 均返回 `ok: true`；`events::tests` 与 `schedule::tests` 全部通过。

### AC-007：订阅、规则模板、下载器与工具修复

- 对应规则：R-008
- 前置条件：本地 Rust 测试、`oracle.py` 差分以及真机实测。
- 操作或输入：运行订阅聚合合并/部分失败/`disable` 引用保护/多行 Base64/`sub set` 改名、规则模板无快照 `use`/缺键拒绝/旧单文件迁移、`upxui` `./ui -> ./dashboard`、`upkernel` Alpha 通道与重启、`geo::update` 回滚、`upcnip` 停机态测试，以及真机下载器直连+回退测试。
- 预期结果：全部单测、`oracle.py` 差分与真机验证通过。
- 验证状态：通过
- 验证证据：`cargo test` 129 单元 + 27 集成全部通过；`oracle.py` 恢复对 legacy 与 Rust 的 6 组订阅夹具对照（normal/aggregate 完全一致，normal.b64 Rust 增强解析成功，mixed/empty/fail 均正确拒绝）；`.12` 真机实测 `task start sub-update --wait` 成功（`exit_code=0`，`gid=3005` 直连+回退下载与热重载 PID `17687 -> 17687` 不变）。

### AC-008：WebUI 契约收口与干净构建包

- 对应规则：R-008
- 前置条件：更新 WebUI 与 [build.sh](../../build.sh)。
- 操作或输入：运行 WebUI 测试与构建、`build.sh` 生成 `box-2.0.0.zip` 并检查 `unzip -l`。
- 预期结果：WebUI 单测与构建通过，不再引用 `githubtoken.preview`，`find-process-mode` 展示与保存一致；zip 包内不含 `tools/`、`AutoADBWiFi.zip` 等杂项。
- 验证状态：通过
- 验证证据：`npm --prefix webui test` 10 文件 43 测试全部通过，`npm --prefix webui run build` 构建成功；`box-2.0.0.zip`（SHA-256 `608c4fbd714277047b345f2b2d519ea5318b063bb33ab6a6ca1579ee3b32e6c1`，内置 arm64 `boxctl` SHA-256 `638aac2c1eea7d83c39097e8fee0435273f18d2f6e0f42c52d0a631df520dd9a`）经 `unzip -l` 确认不含 `.git`、`tools/`、`legacy/` 或无关 `.zip`；同时修复 `customize.sh` 升级覆盖安装时遗漏恢复 `/data/adb/box/state/` 与 `/data/adb/box/githubtoken` 的缺陷，并将 `find_process_mode` 项目默认配置调整为 `off`。

### AC-009：`nft_plan` 代码级单测门禁

- 对应规则：R-009
- 前置条件：`cargo test`。
- 操作或输入：运行 `network::tests::nft_*` 全部测试。
- 预期结果：覆盖 5 种 `nft` 模式建表、本地 OUTPUT、`proxy_mode`、mark 顺序与 `cleanup_stale` 删表，全部通过（明确标注未做真机验收）。
- 验证状态：通过
- 验证证据：`network::tests::nft_all_five_modes_create_table_and_fix_mixed_and_local_output` 通过（注：`.12`/`.33` 均 `nft=false`，未做真机内核实测）。

### AC-010：双机完整回归并恢复用户运行状态

- 对应规则：R-001 至 R-008
- 前置条件：上述各专项在 `.12` 和 `.33` 验证完成。
- 操作或输入：在 `.12` 和 `.33` 上恢复原用户设置并确认重启/启动后状态、代理流量、WebUI/API 全部健康。
- 预期结果：两台真机均稳定运行，无遗留测试规则或测试文件。
- 验证状态：通过
- 验证证据：
  - 真机完整刷包安装验证：使用最终构建的 `box-2.0.0.zip`（SHA-256 `c9a1cefa3c9f1bd4b1d0a7687c132fd58b24a7daaca8276c773c5710031f4da7`）在 `.33` 通过 `magisk --install-module`、在 `.12` 通过 `/data/adb/ksud module install` 完成全量模块安装；
  - 两机安装后 `modules_update/box_for_root/` 仅包含干净的 `action.sh`、`module.prop`、`uninstall.sh`、`webroot/`（无 `AutoADBWiFi.zip`、`tools/` 等杂项），`/data/adb/box/state/`、`settings.json`、`mihomo/`、`dashboard/` 与 `boxctl`（SHA-256 `84da69e83c13662c98293592ea72dc02cc9b48ce5260346f85359d80313f2726`）均完整就位；
  - 安装后重启 daemon 并启动服务：`.12`（`tproxy` + `blacklist`）`doctor` 全绿且 `uid 2000` 访问 Google 返回 `204`；`.33`（`tun`）`doctor` 正常且 `uid 2000` 访问 Google 返回 `204`。

## 7. 未决问题与假设

### Q-001：两台真机均无 `ipset` 与 `nft`

- 类型：暂用假设
- 内容：已实测 `.12`（KSU/API 36）和 `.33`（Magisk/API 31）均为 `ipset=false`、`nft=false`、`ip6tables_nat=false`。
- 影响范围：R-002、R-009、AC-002、AC-009。
- 是否阻塞：否；CNIP `ipset` 规则与 `nft` 规则通过 Rust 单测/golden 验证，两台真机重点验证无 `ipset` 时的安全降级与 `upcnip` 不报错。
- 决定或验证方式：待未来接入具备 `ipset`/`nft` 的设备时再补充内核实测。

### Q-002：Android 16（`.12`）cgroup v2 路径差异

- 类型：暂用假设
- 内容：`.33` 已确认存在 `/dev/memcg`、`/dev/blkio`、`/dev/cpuset`（cgroup v1）；`.12` 为 Android 16 / Linux 6.6，cgroup 挂载布局可能为 cgroup v2 或受 SELinux 限制。
- 影响范围：R-006、AC-006。
- 是否阻塞：否；实现中兼容 v1/v2 探测并在不可用时记录警告优雅降级，实施时在 `.12` 上实际核查挂载点。
- 决定或验证方式：阶段实现后在 `.33` 与 `.12` 双机实测确认。

## 8. 技术设计

### 8.1 模块划分与实施批次

按依赖关系分 6 个批次推进（每批先补失败单测再改代码并跑 `cargo test`）：

1. **批次 1 — 网络平面重写与核心身份（`service.rs`、`network.rs`、`network_rules.rs`）**：
   - `service.rs`：`spawn_core` `pre_exec` 设置 `setgid(3005)`、`RLIMIT_NOFILE`、`sched_setaffinity`（`taskset_cpu`）；启动后执行 `cgroup_memcg`/`cgroup_blkio`；修复 `prepare_dns_block`（顶层 `sniffer`）、`auto_modify_config=false` 守卫、`proxy_lists_configured`、`proxy_uids` trim/过滤、`stop_internal` 清理失败不吞错、孤儿 `mihomo` 清理、`candidates/` 20 条上限清理；实现 candidate 重生成 + `PUT /configs?force=true` 传 `path` 热重载。
   - `network.rs`：全面修正 `tproxy`（纯 `mangle` + `table 2024 pref 100`，去错误 `mark RETURN`）、`enhance`（mangle 屏蔽 TCP）、`mixed`（TUN + redirect）、`tun`（动态 `tun.device`、`filter/FORWARD`、`sysctl`、仅 `bypass_cn_ip=true` 建 bypass 链、`route replace` 回滚）；接通 `ap.list.cfg`、`mac_filter`/`macs_list`、`dns_hijack_mode`、`mihomo_dns_forward`、`bypass_cn_ip_v4/v6`、启动载入 `ipset`、`cnip_force_proxy_apps` 位置、本机链 CNIP/`force_proxy_cidrs`、Fake-IP ICMP/v6/TUN 规则、`ipv6=false` 阻断与 `ipv6=true` 补齐、精确 `0:3005` 放行 + loopback port REJECT + 白名单 `1052`、`performance_mode`（`DIVERT`/conntrack 快速路径）；修复 `filter/PREROUTING`、`user_rules_for` 链方向、端口范围 `:`、`BOX_DNS` 先于 `BOX_LOCAL_REDIRECT`、`BOX_QUIC_BLOCK` 清理顺序、去重规则；同步修复 `nft_plan` 与 `status.get` 规则计划一致。
2. **批次 2 — 实时监控、事件自控与调度器（`daemon.rs`、`events.rs`、`schedule.rs`）**：
   - `daemon.rs`：启动前检查 `/data/adb/modules/box_for_root/disable`；增加 `disable` 文件监控线程；增加网络地址变化监听与 `LOCAL_IP_V4`/`LOCAL_IP_V6` 热刷新；`bond0`/`bond1` 与 `check` 命令接入 CLI/daemon。
   - `events.rs`：反转修正 `use_module_on_wifi_disconnect`；修复 `validated` 在非 Wi-Fi 下的判断；补齐 Wi-Fi 黑白名单/禁用停止与无 `event_stop` 自动启动；手动覆盖在下次网络真实切换后释放。
   - `schedule.rs`：使用 `libc::localtime_r` 改为设备本地时区；修复首次开启不立即触发与超 `grace` 推进 `last_run` 防永久卡死；解析并执行 `box/crontab.cfg`。
3. **批次 3 — 订阅、规则模板、下载器与工具（`download.rs`、`sub.rs`、`rule_profile.rs`、`config.rs`、`core.rs`、`geo.rs`、`tools.rs`、`task.rs`、`logs.rs`）**：
   - `download.rs`：支持优先 `gid=3005` + 直连真实 DNS（UDP 53 解析 `223.5.5.5`/`1.1.1.1` 避开 fake-ip），失败且核心运行时回退经代理重试。
   - `sub.rs`：修复聚合合并优先级与 `interval` 保留、部分失败保留旧 provider、`mihomo -t` 校验、`sub disable` 策略组引用检查、`sub enable` 恢复 `provider-defs`、多行/无 padding Base64、`sub set` 改名同步与热重载联动。
   - `rule_profile.rs`：无快照 `use` 自动 `fetch`、`--reload` 改用热重载且停机态不拉起、缺键校验、DNS 引用 provider 合并与计数、旧单文件迁移。
   - `tools.rs` / `geo.rs` / `task.rs` / `config.rs` / `core.rs` / `logs.rs`：修 `upxui` 路径、`upkernel` Alpha/架构/重启/`cancelable=false`、`geo::update` 回滚与重载、`upcnip` 停机态、`task` 20 条清理与子进程树终止、`config::validate` `MATCH,`、`core.get` 大小写与 `find-process-mode` 同步、`logs::get` 回退 `tool.log`、本地时区时间戳。
4. **批次 4 — WebUI 契约收口、打包脚本与 Oracle 修复**：
   - 清理 WebUI 对 `githubtoken.preview` 的残留引用，核对 `find-process-mode` 展示与保存，重建 `webroot/`；
   - 修复 [build.sh](../../build.sh) 排除 `tools/*` 和非 `box-*.zip` 的 `.zip`；
   - 修复 [oracle.py](../../tools/legacy-oracle/oracle.py) 兼容 `find_process_mode` 并重跑差分。
5. **批次 5 — 交叉编译与 `.12`/`.33` 双机真机验收**：
   - 编译 arm64 `boxctl` 并打包 `box-2.0.0.zip`；
   - 在 `.12`（KSU）和 `.33`（Magisk）逐项执行 AC-001 至 AC-010 并记录真实证据，更新本文档验证状态。
