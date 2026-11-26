# Legacy shell 与 Rust 行为对齐（Parity）清单

本文件不是新的产品规格，而是 Rust 重写的回归门禁。原则：旧 shell 是行为参考，不是可执行依赖；每个已发现的差异必须有 Rust 测试。

## 已建立的对齐项

| 行为 | 旧 shell 参考 | Rust 实现/测试 | 状态 |
|---|---|---|---|
| 非 TUN 模式关闭 TUN | `legacy/box-scripts/box.service` 非 TUN 分支 | `service::disable_tun_candidate`；`candidate_disables_tun_and_process_scanner_for_non_tun_modes` | 已覆盖 |
| Android 进程扫描不拖垮 core | 旧服务未把 `find-process-mode: always` 带入代理路径 | 候选配置由 `find_process_mode` 设置控制，默认 `strict`；同一测试 | 已覆盖 |
| 聚合订阅空 `proxies: []` | `box.tool::sub_is_aggregate_file` 按数量判断 | `sub::parse_aggregate`；`aggregate_parser_reads_provider_urls_and_rejects_mixed` | 已覆盖 |
| provider 合并而非整段丢弃 | `box.tool::update_mihomo_providers` 以现有 provider 为基线 | `rule_profile::use_pending` 合并已有 provider；`use_pending_applies_config_and_marks_active` | 已覆盖 |
| `RULE-SET,...,no-resolve` 取第三列 | `box.tool` 规则校验使用 `cut -f3` | `config::validate` 去掉修饰符；`rule_set_target_allows_no_resolve_suffix` | 已覆盖 |
| 旧 eBPF 设置迁移 | 用户 2026-11-26 要求移除 eBPF | `settings::load` 迁移 `ebpf → tproxy/ipset`；`migrates_removed_ebpf_values` | 已覆盖 |
| tproxy/redirect 本地绕过 | `box.iptables` local chain / uid bypass | `network::tproxy_rules` / `redirect_rules`；规则计划测试 | 部分覆盖，真机已验 CPU |

| 订阅 provider 写入 config 并重启 core | `box.tool` 更新 provider 后 reload | `sub::sync_proxy_providers` + `restart_core_if_running`；真机节点 API 已验证 | 已覆盖 |

| WebUI 订阅更新任务不自我中断 | 旧脚本更新后重载；Rust restart 曾取消任务 | `service::restart` 保留任务；WebUI 点击任务已验证完成 | 已覆盖 |

## 仍需差分验证

| 行为 | 风险 | 下一步 |
|---|---|---|
| 订阅 Base64/普通 provider 解析 | 旧 shell 的边界分支多 | 用 legacy test fixture 做 Rust 对照 |
| rule-profile 合并的 provider 冲突/禁用语义 | 合并可能保留本应删除的 provider | 增加冲突/删除夹具 |
| 五种网络模式规则逐条对齐 | 规则顺序、链名、清理差异会影响真机 | 对同一 fixture 比较规则集合，不只比较 start/stop |
| nft backend | 当前设备 nft=false | 有设备时补真实规则测试 |
| TUN include/exclude UID | 旧脚本直接维护 TUN UID 列表 | 确认 Rust TUN 路径是否等价，补测试 |
| 安装/升级/卸载副作用 | 备份、purge、钩子中断恢复 | 临时 home 差分测试 + 真机 |

## 门禁

- 任何新 bug 先写失败测试，再修复；
- 不以“Rust 自测通过”替代 legacy 差分；
- 在五项网络模式的规则集合和订阅/规则模板差分完成前，不宣称“等价重构完成”。
