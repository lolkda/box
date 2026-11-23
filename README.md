# Box for Root

`Box for Root` 是一个面向 Android Root 环境（Magisk / KernelSU / APatch）的透明代理工具箱模块。

项目深受以下仓库启发并在其基础上持续演进：
- [CHIZI-0618/box4magisk](https://github.com/CHIZI-0618/box4magisk)
- [CHIZI-0618/AndroidTProxyShell](https://github.com/CHIZI-0618/AndroidTProxyShell)
- [taamarin/box_for_magisk](https://github.com/taamarin/box_for_magisk)

## 项目定位

本仓库主要提供：
- 统一的代理核心运行管理（mihomo / sing-box）
- 多网络模式下的透明代理规则编排（TProxy / Redirect / Tun / Mixed / Enhance）
- 订阅、Geo 资源、核心二进制与 WebUI 的统一维护脚本
- 适配 Android Root 生态的模块化目录与服务生命周期管理

## 主要目录

模块工作目录：`/data/adb/box/`

```text
/data/adb/box/
├── bin/                # 代理核心与工具二进制
├── mihomo/             # mihomo 配置目录
├── sing-box/           # sing-box 配置目录
├── scripts/            # 核心脚本
│   ├── box.service     # 服务生命周期管理
│   ├── box.iptables    # 透明代理规则管理
│   └── box.tool        # 更新与维护工具集
├── run/                # 运行时状态与日志
├── settings.json       # 用户设置（唯一配置源）
├── settings.schema.json # 设置字段约束
└── settings.base.sh    # 静态内部路径与函数
```

## 核心脚本

- `box.service`: 启停、重启、状态与定时任务控制
- `box.iptables`: 透明代理规则启用、重建、清理
- `box.tool`: 订阅更新、Geo 更新、核心更新、配置检查、WebUI 相关维护

## 订阅管理（Mihomo）

订阅清单位于 `/data/adb/box/mihomo/subscriptions.list`，每行 `名字|订阅地址`。推荐通过命令维护：

```bash
su -c "/data/adb/box/scripts/box.tool sub add <订阅地址> [名字]"
su -c "/data/adb/box/scripts/box.tool sub list"
su -c "/data/adb/box/scripts/box.tool sub rm <名字>"
su -c "/data/adb/box/scripts/box.tool sub update"
```

- 更新订阅会重建 `proxy-providers` 并重载核心。
- 下载或解析失败时保留上一个可用节点文件，不会清空。
- `settings.json` 的 `ruleset_prefix` 可给规则集地址加镜像前缀，留空使用官方地址。

## 管理页面（KernelSU WebUI）

模块自带管理页，入口是 KernelSU → 模块 → `Box for Root` 的 WebUI 按钮（`< >` 图标）。页面通过 KernelSU 的 `kernelsu.exec()` 以 root 调用 `box.tool`，无需额外服务。

当前管理页是本次全链路重构版本：

- 概览：运行状态、启动/停止/重启/重载、关键指标与最近日志
- 订阅：掩码列表、添加/编辑/删除、单条与全部更新
- 策略组、规则、规则集：仅 mihomo，可视化增删改；规则支持结构化构建与高级原文模式
- 核心配置：mihomo 安全运行层表单 + 完整 YAML 高级编辑；`external-ui` 固定为 `./dashboard` 并支持一键更新 zashboard 面板
- 应用与网络：`package.list.cfg` / `gid.list.cfg` / `ap.list.cfg` 可视化清单
- 设置：全部可编辑标量键、数组编辑器、应用包名选择与 GitHub Token 脱敏；sing-box 订阅地址在对应核心下显示
- 日志：自动刷新、级别过滤、搜索、复制与清空
- 任务中心：订阅/内核等后台任务由 shell 托管，关闭页面后可恢复查看
- 危险操作：删除配置、清空日志、重置订阅、更新核心（均需二次确认）

`box.tool --json` 在本次重构后统一返回 `{ok,data,error}` 包络；CLI 文本命令保持不变，旧 `--json` 字段不兼容。规格见 [module-webui-refactor.md](docs/specs/module-webui-refactor.md) 与 [module-webui-config-expansion.md](docs/specs/module-webui-config-expansion.md)。

源码在 `webui/`，构建：

```bash
cd webui && npm install && npm run build
```

构建产物输出到 `webroot/`，模块打包时直接包含该目录。模块包同时内置 zashboard（构建时获取最新 `dist-no-fonts`），安装/升级后 mihomo 的 `/ui` 固定由 zashboard 提供；`box.tool upxui` 或核心配置页的「更新面板」可手动更新。规格见 [module-dashboard-zashboard.md](docs/specs/module-dashboard-zashboard.md)。

设置数据层使用 `settings.json` + `settings.schema.json`，运行时由 `box.tool` 生成 `run/settings.runtime.sh`；旧 `settings.ini` 不兼容并会在升级时重置。规格见 [module-settings-json.md](docs/specs/module-settings-json.md)。

## 文档与社区

- Wiki: <https://github.com/boxproxy/box/wiki>
- 更新日志: [CHANGELOG.md](./CHANGELOG.md)
- 配套 APP / 通知频道: <https://t.me/zero_o0>

## 致谢

感谢开源社区与上述项目作者提供的设计思路与实现参考。
