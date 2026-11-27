# Box for Root

`Box for Root` 是一个面向 Android Root 环境（Magisk / KernelSU / APatch）的透明代理工具箱模块。

项目深受以下仓库启发并在其基础上持续演进：
- [CHIZI-0618/box4magisk](https://github.com/CHIZI-0618/box4magisk)
- [CHIZI-0618/AndroidTProxyShell](https://github.com/CHIZI-0618/AndroidTProxyShell)
- [taamarin/box_for_magisk](https://github.com/taamarin/box_for_magisk)

## 项目定位

本仓库主要提供：
- 统一的 mihomo 代理核心运行管理
- 多网络模式下的透明代理规则编排（TProxy / Redirect / Tun / Mixed / Enhance）
- 订阅、Geo 资源、核心二进制与 WebUI 的统一维护脚本
- 适配 Android Root 生态的模块化目录与服务生命周期管理

## 主要目录

模块工作目录：`/data/adb/box/`

```text
/data/adb/box/
├── bin/                # 代理核心与工具二进制
├── mihomo/             # mihomo 配置目录
├── run/                # 运行时状态与日志
├── settings.json       # 用户设置（唯一配置源）
├── settings.schema.json # 设置字段约束
└── webroot/            # 模块管理页（不放入 /data/adb/box）
```

## 核心脚本

- `boxctl`: Rust 管理 CLI/daemon，负责服务、设置、订阅、任务、网络规则与维护

## 订阅管理（Mihomo）

订阅清单位于 `/data/adb/box/mihomo/subscriptions.list`，每行 `名字|订阅地址`。推荐通过命令维护：

```bash
su -c "/data/adb/box/bin/boxctl sub add <订阅地址> [名字]"
su -c "/data/adb/box/bin/boxctl sub list"
su -c "/data/adb/box/bin/boxctl sub rm <名字>"
su -c "/data/adb/box/bin/boxctl sub update"
```

- 更新订阅会重建 `proxy-providers` 并重载核心。
- 下载或解析失败时保留上一个可用节点文件，不会清空。
- `settings.json` 的 `ruleset_prefix` 可给规则集地址加镜像前缀，留空使用官方地址。

## 管理页面（KernelSU WebUI）

模块自带管理页，入口是 KernelSU → 模块 → `Box for Root` 的 WebUI 按钮（`< >` 图标）。页面通过 KernelSU 的 `kernelsu.exec()` 以 root 调用 `boxctl`，无需额外常驻 HTTP 服务。

当前管理页是本次全链路重构版本：

- 概览：运行状态、启动/停止/重启/重载、关键指标与最近日志
- 订阅：掩码列表、添加/编辑/删除、单条与全部更新
- 策略组、规则、规则集：仅 mihomo，可视化增删改；规则支持结构化构建与高级原文模式
- 核心配置：mihomo 安全运行层表单 + 完整 YAML 高级编辑；`external-ui` 固定为 `./dashboard` 并支持一键更新 zashboard 面板
- 应用与网络：`package.list.cfg` / `gid.list.cfg` / `ap.list.cfg` 可视化清单
- 设置：全部可编辑标量键、数组编辑器、应用包名选择与 GitHub Token 脱敏
- 日志：自动刷新、级别过滤、搜索、复制与清空
- 任务中心：订阅/内核等后台任务由 shell 托管，关闭页面后可恢复查看
- 危险操作：删除配置、清空日志、重置订阅、更新核心（均需二次确认）

`boxctl --json` 统一返回 v2 包络。

源码在 `webui/`，构建：

```bash
cd webui && npm install && npm run build
```

构建产物输出到 `webroot/`，模块打包时直接包含该目录。模块包同时内置 zashboard（构建时获取最新 `dist-no-fonts`），安装/升级后 mihomo 的 `/ui` 固定由 zashboard 提供；`boxctl task start xui` 或核心配置页的「更新面板」可手动更新。

设置数据层使用 `settings.json` + `settings.schema.json`，运行时由 `boxctl` 管理 `settings.json`；旧 `settings.ini` 不兼容并会在升级时重置。

2.0.0 起只保留 mihomo：升级会删除旧 `sing-box/` 配置目录和 `bin/sing-box`，旧 settings 中的 sing-box 字段会自动清理。

订阅支持普通 provider 和完整配置聚合自动拆分；订阅只管理节点，策略组和规则仍由 `config.yaml` 管理。

## 文档与社区

- Wiki: <https://github.com/boxproxy/box/wiki>
- 更新日志: [CHANGELOG.md](./CHANGELOG.md)
- 配套 APP / 通知频道: <https://t.me/zero_o0>

## 致谢

感谢开源社区与上述项目作者提供的设计思路与实现参考。
