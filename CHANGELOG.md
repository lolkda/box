#### Changelog 2.0.0 - 24-11-2026
- BREAKING: 完全移除 sing-box，仅保留 mihomo
- 升级时删除旧 sing-box 配置目录与二进制
- 移除核心选择、sing-box 设置/订阅/内核更新与相关 WebUI 入口
- 自动迁移旧 settings 中的 sing-box 字段
- 保留 mihomo 订阅、provider、User-Agent、规则和网络模式功能

#### Changelog 1.11.9 - 24-11-2026
- 移除设置页的订阅模式选项
- 订阅更新统一自动识别 provider、proxies 与完整配置聚合
- 兼容旧 subscription_mode=provider，不再拒绝聚合订阅
- 保留全局与按订阅 User-Agent 配置

#### Changelog 1.11.8 - 24-11-2026
- 新增全局与按订阅 User-Agent 配置
- 订阅 UA 支持预设与自定义，留空继承全局
- 订阅与聚合 provider 下载使用对应 UA
- 补齐订阅 UA 校验、重命名/删除清理与回归测试

#### Changelog 1.11.7 - 24-11-2026
- 订阅与 provider 下载改用兼容的 User-Agent，避免转换服务返回不可达节点入口
- 保留 provider 完整字段合并与 gstatic 默认测速
- 新增订阅 User-Agent 回归断言
- 完整 shell 测试 156/156 通过

#### Changelog 1.11.6 - 24-11-2026
- 合并订阅 proxy-providers 完整定义，保留 health-check/override/exclude-filter/interval；同名整块覆盖、不同名追加；只改写本地 path
- 保留手动 provider 和订阅中未出现的旧 provider
- subscription 同时包含 proxies 与 proxy-providers 时拒绝并保留旧配置
- 只有顶层 proxies 的订阅默认 health-check 改为 http://www.gstatic.com/generate_204、timeout 5000
- 新增 provider 合并回归测试

#### Changelog 1.11.5 - 24-11-2026
- 服务启动后补建透明代理规则，避免显示运行中但未接管流量
- 重启改为串行：清理规则、停止核心、解析 UID、启动核心、应用规则
- 规则应用失败时停止服务并返回错误，不再显示假正常
- 补充启动/重启回归测试

#### Changelog 1.11.4 - 24-11-2026
- 停止服务时先清理 iptables、DNS 重定向和策略路由，再停止核心
- 清理失败会取消停止并保留核心，避免停止后 WiFi 无网络
- 新增停止成功和清理失败的回归测试

#### Changelog 1.11.3 - 24-11-2026
- 兼容官方 KernelSU 3.3.1 的 listPackages/getPackagesInfo 包名接口
- 包名选择器区分接口不可用、读取失败和空列表，并始终支持手动输入
- 新 Manager 仍优先使用 ksu.pm()

#### Changelog 1.11.2 - 24-11-2026
- 模块作者增加 lolkda

#### Changelog 1.11.1 - 24-11-2026
- 规则订阅操作改名为“应用此更新”，明确是应用待应用快照
- “停用当前”改为“停用”
- 旧版迁移的待应用快照显示实际暂存时间，不再显示“尚未下载”

#### Changelog 1.11.0 - 24-11-2026
- 规则与策略订阅支持多条，同一时间只使用一条，更新暂存、使用才应用
- 节点订阅支持禁用/启用，禁用后节点移出 proxy-providers，保留 URL 和本地文件
- 概览页去掉最近日志，日志只在日志页刷新
- 规则订阅支持旧单条 URL 自动迁移为“默认规则”

#### Changelog 1.10.3 - 24-11-2026
- 订阅页拆分为“节点订阅”和“规则与策略”双 Tab
- 规则订阅移出节点列表，使用独立子路由 /subscriptions/rule-profile
- 节点/规则操作隔离，返回键与底部“订阅”选中态保持
- 保留规则订阅 URL、数量、更新与策略/规则/规则集/config.yaml 入口

#### Changelog 1.10.2 - 24-11-2026
- 修复 tproxy + fake-ip 环境下规则模板和节点订阅下载失败
- 下载前解析真实 IPv4，并使用 root:net_admin 绕过 tproxy
- curl 失败时回退 wget，失败返回明确 E_DOWNLOAD
- 应用规则模板时保留 DNS 段引用的旧规则集，避免 mihomo 启动失败

#### Changelog 1.10.1 - 24-11-2026
- 修复 1.10.0 的 versionCode 仍按 UTC 日期生成、与 1.9.2 冲突的问题
- versionCode 改为 Asia/Shanghai 日期并保证单调递增
- 包含 1.10.0 的全局规则与策略订阅功能

#### Changelog 1.10.0 - 23-11-2026
- 订阅页新增全局规则与策略订阅
- 支持从独立 config.yaml 导入 proxy-groups、rule-providers、rules
- 节点订阅仍只维护 proxy-providers，互不覆盖
- 应用前可选择仅保存或保存并重载
- 规则订阅 URL 在日志中脱敏

#### Changelog 1.9.2 - 23-11-2026
- 去掉管理页顶部栏，页面内容直接进入
- 移除顶部任务快捷按钮，任务中心继续通过「更多 → 任务」进入
- 子页面保留轻量返回按钮与安全区适配

#### Changelog 1.9.1 - 23-11-2026
- 修复 WebView 点击与滚动卡顿：命令改用异步 ksu.spawn()
- 任务仅在有运行中任务时轮询
- 状态仅在概览页刷新，概览日志改为 5 秒
- 补充旧版 KernelSU 升级提示、单元测试与性能规格

#### Changelog 1.9.0 - 23-11-2026
- 新增订阅自动识别：完整配置聚合自动拆出 proxy-providers；新增 subscription_mode（auto/provider）；订阅页显示派生 provider 数量
- 移除 renew 覆盖模式；首次升级重置旧设置和默认 config
- 聚合 provider 逐项下载并保留旧文件；保留 proxy-groups、rules、rule-providers

#### Changelog 1.8.0 - 23-11-2026
- 设置数据层重构：settings.json 成为唯一用户配置源，自动生成 settings.runtime.sh
- 旧 settings.ini 不兼容，升级直接重置默认设置
- 设置读取改为单次 JSON 解析，避免逐键 shell 开销
- WebUI 设置页适配扁平设置接口

#### Changelog 1.7.0 - 23-11-2026
- mihomo /ui 固定为 zashboard
- 模块包内置最新 dist-no-fonts
- 升级强制部署 zashboard 并清理 external-ui-url
- 核心配置页新增一键更新面板

#### Changelog 1.6.0 - 23-11-2026
- 新增 mihomo 核心配置页与原始 YAML 编辑
- 新增应用与网络列表编辑
- 设置页补齐 CNIP 与 sing-box 订阅配置
- 清理死代码并修复 settings、GitHub Token、eBPF 热更新
- 不再支持 xray/v2fly/hysteria 核心

#### 未发布 - 23-11-2026
- 修复 KernelSU WebView 高频轮询导致的点击与滚动卡顿：命令改用异步 `ksu.spawn()`，任务仅在运行中轮询，状态仅在概览页刷新
- 订阅自动识别：完整配置聚合自动拆出 `proxy-providers`，新增 `subscription_mode`（auto/provider），移除 `renew` 覆盖模式
- 首次升级自动重置旧设置和占位 `config.yaml`，订阅页显示派生 provider 数量
- 设置数据层重构：settings.json 成为唯一用户配置源，自动生成 settings.runtime.sh；旧 settings.ini 不兼容并直接重置
- mihomo `/ui` 固定为 zashboard：模块包内置最新 `dist-no-fonts`，升级强制部署、清理旧 `external-ui-url`，核心配置页支持一键更新
- 核心精简：仅保留 mihomo + sing-box；安装/升级移除 xray、v2fly、hysteria 并清理旧文件
- 新增 mihomo 核心配置页：安全运行层表单 + 完整 YAML 高级编辑
- 新增应用与网络页：应用包（支持 UID:包名）、GID、接口 allow/ignore 清单
- 设置页补齐 cn_ip_url、cn_ipv6_url、subscription_url_singbox
- 管理页策略/规则/规则集/核心配置增加 mihomo-only 保护
- 破坏性变更：不再支持 xray / v2fly / hysteria 核心
- 清理死代码与失效分支：修复 settings 路径、GitHub Token 参数、eBPF CNIP 热更新

#### Changelog 1.5.0 - 23-11-2026
- 全链路重构 KernelSU WebUI：React + TypeScript 新版界面
- 统一 box.tool JSON 契约与后端校验
- 新增后台任务中心与设置数组编辑
- 破坏性变更：旧 --json 输出不兼容

#### Changelog 1.4.0 - 22-11-2026
- 新增 KernelSU WebUI 管理页面：订阅、服务、策略组、规则、规则集、设置、日志
- box.tool 新增 status/logs/settings/config JSON 接口
- 危险操作与核心更新支持二次确认
- 修复 webroot 被服务启动覆盖的问题

#### Changelog 1.3.2 - 22-11-2026
- 合并 upstream/test：适配 mihomo 与 sing-box 新版 eBPF 入站配置
- 新增 box.tool sub 订阅管理（subscriptions.list）
- 订阅更新失败保留上一个可用 provider
- 规则集地址支持 ruleset_prefix

#### Changelog 1.3.1 - 22-11-2026
- 修正模块更新元数据指向 fork 仓库
- 订阅管理改为 subscriptions.list 与 box.tool sub 命令

#### Changelog 1.3.0 - 22-11-2026
- feat: manage mihomo subscriptions from subscriptions.list (9dc5a59)

#### Changelog 1.2.8 - 13-06-2026
- fix(log): 统一脚本日志输出 (dfa2661)
- fix(net): 修复 TUN 模式下黑白名单与 fake-ip 连接异常 (ec78ef8)
- fix(net): 移除 net.inotify 并发锁，优化日志输出与防回环规则更新 (461f767)
- 提交上一版缺失的 box.tool 修改 (f98a590)
- feat: 优化网络规则刷新 (9f615f4)
- refactor: 重构代理规则与内核配置覆写，新增性能模式与其余模式 CNIP 兼容 (ea408a1)

#### Changelog 1.2.7 - 04-04-2026
- 重构网络与 iptables 规则框架，增强 TProxy/IPv6 与网络匹配稳定性
- 新增 CNIP（ipset）分流能力，并提供 upcnip 更新命令
- 安装恢复流程补充 gid.list.cfg

#### Changelog 1.2.6 - 06-11-2025
- 优化 WebUI 不再是固定 dashboard 目录
- 为模块安装新增删除更新残留选项，新增超时自动选择
- 为更新订阅新增自动修改 mihomo 的 proxy-providers 配置内容

#### Changelog 1.2.5 - 02-10-2025
- 优化安装脚本优化 service 的 singbox 部分内容
- 增加 GitHub Token 密钥填写
- 精简优化默认 mihomo 配置

#### Changelog 1.2.4 - 23-08-2025
- 优化与重构

# Changelog

