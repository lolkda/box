# Legacy shell → Rust 行为深度比对审计

审计对象：

- Legacy：`legacy/box-scripts/box.service`（982 行）、`box.tool`（4591 行）、`box.iptables`（3285 行）、`box.inotify`、`net.inotify`
- Rust：`rust/crates/boxctl/src/{service,sub,rule_profile,config,network,events,settings}.rs`
- 方法：按函数边界逐个读输入校验、配置变换、规则顺序、文件副作用、失败回滚、清理和返回语义；不只看接口是否 `ok`。

## 结论

当前 Rust 版本实现了可运行的主链路，但**不是 legacy shell 的行为等价重写**。差异不只在边缘格式，至少 6 项属于核心代理行为差异，会导致“服务正常但流量不走代理/绕过策略失效/DNS 泄漏”一类问题。

## 1. 订阅

| # | Legacy 行为 | Rust 现状 | 差异/风险 |
|---|---|---|---|
| S1 | 聚合 provider URL 接受 `http://` 和 `https://`（`box.tool` 聚合校验） | `sub.rs:470` 强制 `https://`，否则 `E_TLS` | 合法的 HTTP provider 聚合会被整体拒绝 |
| S2 | 聚合 provider 先做全量名称/URL/冲突校验，再开始下载（`box.tool:1160-1205`） | `sub.rs:495-505` 在下载循环中才检查冲突 | 第二个 provider 冲突时，第一个 provider 已被覆盖，产生部分写入 |
| S3 | 生成 provider block 时写入 `health-check`，且聚合完整定义缓存可保留 provider 专属字段（`box.tool:710-790`, `1081-1129`） | `sub.rs::sync_proxy_providers` 只写 `type/url/path/interval` | 丢失 health-check、filter、headers、override 等，URLTEST 行为和节点过滤会变 |
| S4 | 订阅更新和规则应用后执行 `update_mihomo_rulesets`，按 `ruleset_prefix` 重写 rule-providers URL，并保存旧前缀状态（`box.tool:879-918`） | Rust 没有对应实现；`settings.rs` 只校验该字段 | 设置 `ruleset_prefix` 后规则集 URL 不会加前缀/恢复官方 |
| S5 | 禁用订阅时 `update_mihomo_providers` 会从合并结果删除该订阅 provider（`box.tool:820-870`） | `sub::set_enabled` 只改清单，不重建 config | 禁用后 config 仍保留 provider，节点/规则继续可见 |
| S6 | 订阅更新失败保留 provider 和旧 config；全部失败返回非 0（`upsubs`） | Rust 文件保留基本一致，但 config 同步失败时的错误层次不同 | 失败语义和 UI 提示可能不一致 |

## 2. 规则模板

| # | Legacy 行为 | Rust 现状 | 差异/风险 |
|---|---|---|---|
| R1 | 仅保留旧配置中 **被 DNS 引用** 的 rule-provider（`fake-ip-filter` / `nameserver-policy` 的 `rule-set:` 项），见 `box.tool:3219-3250` | `rule_profile.rs:319-331` 把旧配置 **所有** rule-providers 合并进模板 | 模板本应删除的 provider 被保留；实际计数 39，而模板是 33 |
| R2 | 应用后调用 `update_mihomo_rulesets`（`box.tool:3350`） | 没有前缀同步 | 与 S4 相同 |
| R3 | 应用后 `reload` 只重载 core，不重建 candidate | Rust 改为 restart，确保源 config 生效 | 行为更重但更可靠；属于有意偏离，不是 bug，但语义不同 |
| R4 | 模板缺失 DNS 引用的 provider 时先补旧定义再校验（`rule_profile_preserve_dns_providers`） | Rust 在应用阶段补，但补的是全部旧 provider | 能避免启动失败，但引入 R1 的污染 |

## 3. 服务启动与配置准备

| # | Legacy 行为 | Rust 现状 | 差异/风险 |
|---|---|---|---|
| V1 | `prepare_mihomo` 自动补 `external-controller`、`external-ui`、`tproxy-port`、`redir-port`（`box.service:390-425`） | `start` 只复制 config，`disable_tun_candidate` 只处理 tun/find-process（`service.rs:44-180`, `652-710`） | 自定义 config 缺少这些字段时，控制面和端口不会自动补齐 |
| V2 | `mixed` 视为 TUN 模式，保留/启用 TUN，随后应用 redirect 规则（`box.service:439-515`, `box.iptables:3027-3040`） | `selected_mode != tun` 就关闭 TUN；`network::plan` 对 mixed 应用 tproxy+redirect | **核心差异**：mixed 实际语义从 TUN+redirect 变成 tproxy+redirect |
| V3 | TUN 模式写 `tun:` block、stack、DNS hijack、auto-route/strict-route、include/exclude-uid（`box.service:439-515`） | 不写 TUN block；只保留原 config 内容 | 自定义 TUN 配置缺失或 UID 名单不生效 |
| V4 | 非 core 模式且存在 UID/接口名单时，DNS `enhanced-mode` 从 fake-ip 改 redir-host，并启用 sniffer（`box.service:500-525`） | 未实现 | UID/接口名单与 fake-ip 的组合行为不同 |
| V5 | 启动前校验权限、用户组、cgroup、taskset、cron、旧进程（`box.service` 大量函数） | Rust 主要检查二进制/配置/pid，spawn 直接运行 | 权限、cgroup、CPU 绑定、旧进程处理不等价 |
| V6 | `find-process-mode` 由用户配置决定 | 已改为 WebUI `find_process_mode` 设置控制，默认 `strict` | 不再是强制偏离；用户可选择 always/strict/off，风险由界面提示 |

## 4. 网络规则

| # | Legacy 行为 | Rust 现状 | 差异/风险 |
|---|---|---|---|
| N1 | `mixed` = TUN bypass + redirect（`box.iptables:3027`, `3127-3195`） | `network.rs:394-430` 把 mixed 加入 tproxy，且没有 TUN bypass | 混合模式实际接管方式错误 |
| N2 | DNS NAT 劫持在 tproxy/redirect/mixed/enhance 都建立（`box.iptables:684-890`） | `network.rs` 的 `dns_rules` 只在 TUN 分支调用，见 `323-395` | 非 TUN 模式 DNS 可能泄漏或绕过 mihomo DNS |
| N3 | 本地应用 OUTPUT 流量有完整 `BOX_LOCAL` 链（`box.iptables:1135-1210`, `1991-2105`） | 只有 `cnip_force_proxy_apps` 的 force UID 才创建 OUTPUT 链；普通本地流量无 OUTPUT 代理 | 设备本机应用可能不走代理，只有转发流量走 |
| N4 | `proxy_mode=blacklist/whitelist` 通过 UID 名单决定代理范围（`box.iptables` 的 UID/package 逻辑） | `network.rs` 完全不读取 `proxy_mode` 或 package 名单；只有 CNIP force UID | 黑/白名单设置实际无效 |
| N5 | TUN 模式建立 forward/TUN bypass 规则，处理 CNIP、fake-ip、loopback（`box.iptables:1424-1455`, `apply_tun_family`） | TUN 分支只返回 DNS 规则 | TUN 的绕过/分流/回环行为不等价 |
| N6 | enhance 模式对 redirect/tproxy 做特殊去重和 UDP/TCP 分支处理（`box.iptables` 多处 `network_mode != enhance`） | enhance 直接按 tproxy+redirect 组合生成普通规则 | 双重接管/回环风险 |
| N7 | 提供 QUIC block、loopback reject、fake-ip ICMP、performance/conntrack 链、普通 DNS forward 等（`box.iptables`） | Rust 没有对应规则 | 这些功能打开后不生效 |
| N8 | CNIP 支持 ipset，并在 TUN/本地/转发多个链处理 | Rust 只在 ipset 可用时生成部分 CNIP 规则，TUN 路径不完整 | 大陆 IP 绕过在 TUN/本地流量上不等价 |
| N9 | IPv6 规则覆盖 DNS、redirect、tproxy、local | Rust 有部分 IPv6 分支，但 DNS/local/TUN 覆盖不完整 | 双栈环境行为不同 |

## 5. 事件与自动控制

| # | Legacy 行为 | Rust 现状 | 差异/风险 |
|---|---|---|---|
| E1 | `net.inotify` 维护本地 IP 防回环规则、网络服务控制、Wi-Fi/MAC 匹配、稳定性窗口 | `events.rs` 有状态跟踪和 Wi-Fi 解析，但规则维护由服务重跑承担 | 网络切换时的防回环/增量更新语义不同 |
| E2 | `box.inotify` 在模块 disable 时按事件启停服务 | Rust 事件路径和模块禁用判断需逐项核对 | 禁用/重启用边界可能不同 |
| E3 | 手动/自动控制以期望状态和网络事件协调 | Rust 已实现 manual/auto，但 restart/stop 会写 manual | 事件后状态语义可能和旧版不同 |

## 6. 设置与 WebUI

| # | Legacy 行为 | Rust 现状 | 差异/风险 |
|---|---|---|---|
| C1 | eBPF 是可配置模式 | 用户明确要求移除，Rust 已删除并迁移旧值 | 有意破坏性变更，不属于 bug |
| C2 | `boot_auto_start` 默认 true | Rust 默认/重置 false（D-019） | 有意变更，需在发布说明中明示 |
| C3 | WebUI 计数来自实际 config | Rust profile 计数因 R1 保留全部 provider，显示 39 而非模板 33 | 用户可见数据不一致 |
| C4 | 订阅更新只 reload core | Rust 更新后 restart core | 更重，但解决 candidate 不生效；属于实现差异 |

## 必须优先修复

1. **N3/N4**：实现普通本地 OUTPUT 代理和 `proxy_mode` UID 名单；否则本机应用代理行为不成立。
2. **N2**：把 DNS 劫持扩展到所有非 TUN 模式。
3. **V2/N1**：修正 `mixed` 为 TUN + redirect，并在候选配置中保留 TUN。
4. **N5/N6**：补齐 TUN bypass 与 enhance 去重规则。
5. **R1**：规则模板只保留 DNS 实际引用的旧 provider。
6. **S4/R2**：实现 `ruleset_prefix` 重写。
7. **S2/S3**：聚合 provider 全量预校验，并保留 health-check/完整定义。
8. **V1/V3/V4**：补齐启动时 config 注入和 TUN/DNS/UID 配置准备。

## 当前验收结论

之前的“five modes start/stop + nodes 99 + CPU 0”只能证明**生命周期和节点加载**通过，不能证明透明代理、DNS、UID 分流、TUN、mixed、enhance 行为等价。上述 N2/N3/N4/N5/N1 未修复前，不应宣称 Rust 迁移完成。
