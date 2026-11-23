#### Changelog 1.6.0 - 23-11-2026
- 新增 mihomo 核心配置页与原始 YAML 编辑
- 新增应用与网络列表编辑
- 设置页补齐 CNIP 与 sing-box 订阅配置
- 清理死代码并修复 settings、GitHub Token、eBPF 热更新
- 不再支持 xray/v2fly/hysteria 核心

#### 未发布 - 23-11-2026
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

