# mihomo 面板固定为 zashboard

## 1. 文档信息

- 文档标识：module-dashboard-zashboard
- 规格状态：已确认
- 最后更新：2026-11-23
- 实现状态：已实现
- 验证状态：部分完成（本地构建、shell 与前端测试通过；未执行真机 ADB 升级）
- 关联文档：[module-webui-config-expansion.md](module-webui-config-expansion.md)（核心配置页与 mihomo 配置扩展规格）
- 关联文档：[module-webui.md](module-webui.md)（历史 WebUI 规格）

## 2. 目标与范围

### 背景

模块当前的 `upxui` 兜底下载地址已经是 Zephyruso/zashboard，但历史版本默认下载 `MetaCubeX/metacubexd`。`customize.sh` 在升级时会恢复旧的 `mihomo/` 目录，因此旧设备上的 `dashboard` 目录可能继续包含 MetacubeXD；同时，旧配置中的 `external-ui-url` 也可能覆盖默认下载源。结果是用户打开 `http://127.0.0.1:9090/ui/` 时仍看到 METACUBE (XD)。

### 目标

- 模块包内置最新 `dist-no-fonts.zip` 版本的 zashboard。
- ADB 升级或覆盖安装模块后，mihomo 的 `/ui` 必须由 zashboard 提供。
- `external-ui` 固定为 `./dashboard`，升级时删除旧的 `external-ui-url`。
- mihomo 核心配置页只读展示 `external-ui`，并提供「更新面板」入口。
- 一键更新面板使用现有 `task xui`，下载最新 `dist-no-fonts.zip`；失败时保留当前可用面板。
- sing-box 的外部 UI 行为保持不变。

### 非目标

- 不提供 MetacubeXD 与 zashboard 的运行时切换。
- 不自动在服务启动或后台定时更新面板。
- 不改变 sing-box 的 `external_ui` 配置或更新逻辑。
- 不删除用户已有的自定义 UI 目录；只不再将其作为 mihomo 当前面板。
- 不包含真机安装、真机升级、Git 提交、推送或发布。

## 3. 参与深度与委托

- 参与深度：架构与设计
- 局部例外：面板选择固定为 zashboard；仅处理 mihomo；版本采用构建时最新版，不固定 zashboard tag。
- 需要用户确认：面板交付方式、旧 MetacubeXD 处理、配置路径策略、`external-ui-url` 清理、更新入口位置、更新失败策略均已确认。
- 模型自主决定：zashboard 下载与解压实现、打包临时目录、`customize.sh` 的覆盖顺序、任务调用方式、组件拆分、错误提示、测试拆分。
- 当前执行授权：允许创建本规格、修改 `build.sh`、`customize.sh`、`box/scripts/box.tool`、`webui/`、`webroot/`、`test/`、`README.md`、`CHANGELOG.md`、`module.prop` 及相关规格链接；允许本地构建与测试。不包含真机部署、Git 提交或推送。

## 4. 行为要求与约束

### R-001：构建时内置 zashboard

- 参与者或触发方：`build.sh`、CI 发布构建。
- 前置条件与输入：构建环境可访问 GitHub；存在 `Zephyruso/zashboard` 的最新 release。
- 必须满足的行为：
  - 从 `https://github.com/Zephyruso/zashboard/releases/latest/download/dist-no-fonts.zip` 获取最新资源。
  - 将解压后的面板文件打包到模块内的 `box/mihomo/dashboard/`。
  - 包内 `dashboard/index.html` 必须存在；下载、解压或校验失败时构建失败，不能生成缺少面板的模块包。
- 输出或状态变化：生成的 `box-<version>.zip` 包含完整 zashboard 静态文件。
- 边界与失败行为：网络失败、HTTP 非 200、压缩包无 `index.html` 时明确报错并退出非零状态。

### R-002：升级与覆盖安装强制部署

- 参与者或触发方：`customize.sh`。
- 前置条件与输入：模块安装或升级；`/data/adb/box` 可能已有旧 mihomo 配置和旧面板。
- 必须满足的行为：
  - 恢复旧 `mihomo/` 配置后，将内置 zashboard 部署到 `/data/adb/box/mihomo/dashboard/`。
  - 无论旧 `mihomo/dashboard` 是否是 MetacubeXD，最终内容必须是新内置 zashboard。
  - `config.yaml` 中 `external-ui` 必须为 `./dashboard`。
  - 删除 `config.yaml` 中的 `external-ui-url` 旧值。
  - 旧的自定义 UI 目录保留，但不再被 mihomo 当前配置引用。
- 输出或状态变化：升级完成后 `/ui` 提供 zashboard；旧 MetacubeXD 不再生效。
- 边界与失败行为：内置面板缺失或复制失败时安装报错；不能回退到旧 MetacubeXD 作为当前面板。

### R-003：mihomo `/ui` 服务路径

- 参与者或触发方：mihomo 服务、WebUI 的「打开面板」链接。
- 前置条件与输入：`bin_name = mihomo`；`external-ui = ./dashboard`。
- 必须满足的行为：
  - `http://127.0.0.1:9090/ui/` 加载 zashboard。
  - 不修改 sing-box 的 `external_ui` 和 `external_ui_download_url`。
- 输出或状态变化：mihomo 面板页面为 zashboard；sing-box 保持原状。
- 边界与失败行为：面板目录为空时由安装或一键更新明确报错。

### R-004：WebUI 配置展示与更新入口

- 参与者或触发方：mihomo 核心配置页。
- 前置条件与输入：`bin_name = mihomo`，核心配置加载成功。
- 必须满足的行为：
  - `external-ui` 字段只读展示 `./dashboard`，不提供编辑控件。
  - 页面提供「更新面板」按钮，仅 mihomo 显示。
  - 点击按钮调用现有任务接口启动 `xui` 任务，不在前端直接下载或写文件。
  - 更新任务运行中或已有其他任务时，显示后端返回的忙碌错误。
- 输出或状态变化：任务中心出现「更新面板资源」任务；完成后面板更新。
- 边界与失败行为：任务启动失败时保留当前面板并提示错误。

### R-005：一键更新面板

- 参与者或触发方：`box.tool` 的 `upxui` / `task xui`。
- 前置条件与输入：mihomo 核心运行或已安装；网络可访问 zashboard release。
- 必须满足的行为：
  - 下载最新 `dist-no-fonts.zip`，而不是旧的 `dist.zip` 或 MetacubeXD。
  - 在临时目录完成下载、解压和 `index.html` 校验后，再替换 `/data/adb/box/mihomo/dashboard/`。
  - 下载、解压或校验失败时保留当前面板内容。
  - 成功后保持 `external-ui: ./dashboard`，不恢复旧的 `external-ui-url`。
- 输出或状态变化：面板目录替换为最新 zashboard；任务状态成功。
- 边界与失败行为：失败时任务状态为失败，当前面板仍可用；不自动删除当前面板。

### R-006：安全与兼容边界

- 参与者或触发方：安装器、任务执行器。
- 前置条件与输入：旧模块数据存在。
- 必须满足的行为：
  - 不删除 `mihomo` 之外的自定义 UI 目录。
  - 不修改 sing-box 的 UI 配置。
  - 面板更新不阻塞 mihomo 服务启动；失败只影响面板资源。
- 输出或状态变化：用户配置目录和 sing-box 配置保持可恢复。
- 边界与失败行为：空间不足或权限失败时给出可读错误，不静默成功。

### 相关约束

- zashboard 资源包采用 `dist-no-fonts.zip`（约 2.9 MB），不携带字体。
- 构建时取 latest，不固定 zashboard tag；构建产物可随构建时间变化。
- 当前只处理 mihomo，sing-box 不纳入本次改造。

## 5. 关键决定

### D-001：模块包内置 zashboard

- 状态：已确认
- 结论：模块包直接携带 zashboard 静态文件，安装/升级时部署，不依赖安装时联网。
- 理由：ADB 升级后必须保证 `/ui` 可用且为 zashboard；下载失败会导致旧 MetacubeXD 残留。
- 考虑过的备选：安装时下载、首次打开 WebUI 下载。
- 未采用原因：两者都无法保证升级完成时立即得到可用 zashboard。
- 影响范围：R-001、R-002、AC-001、AC-002。

### D-002：升级强制 `external-ui: ./dashboard`

- 状态：已确认
- 结论：安装器把 `external-ui` 固定为标准路径，并删除 `external-ui-url`；旧自定义 UI 目录保留但不使用。
- 理由：保证旧设备不会继续通过旧路径或旧下载源显示 MetacubeXD，同时避免删除用户文件。
- 考虑过的备选：保留现有路径并替换内容、仅在缺失时补默认值。
- 未采用原因：保留路径可能覆盖自定义文件；仅补默认值不能修复旧设备。
- 影响范围：R-002、R-003、AC-002。

### D-003：版本采用构建时 latest，资源包为 dist-no-fonts

- 状态：已确认
- 结论：每次构建从官方 release 拉取最新 `dist-no-fonts.zip`；不固定 zashboard tag。
- 理由：用户希望模块随构建获得最新面板；no-fonts 包体积较小且可离线使用。
- 考虑过的备选：固定 v3.30.0、使用完整 dist.zip、使用 CDN 字体包。
- 未采用原因：固定版本需手动升级；完整包体积过大；CDN 字体依赖网络。
- 影响范围：R-001、R-005、AC-001、AC-004。

### D-004：只读展示并复用 xui 任务更新

- 状态：已确认
- 结论：mihomo 核心配置页只读展示 `external-ui`；「更新面板」按钮调用现有 `task xui`，更新失败保留当前面板。
- 理由：避免用户误改路径；复用现有后台任务系统，不新增下载接口。
- 考虑过的备选：可编辑字段、增加独立下载 API、自动启动更新。
- 未采用原因：可编辑会破坏固定路径；独立 API 重复造轮子；自动更新不可控。
- 影响范围：R-004、R-005、AC-003、AC-005。

### D-005：仅处理 mihomo

- 状态：已确认
- 结论：zashboard 固定策略只作用于 mihomo；sing-box 的 `external_ui` 行为保持不变。
- 理由：截图与问题均指向 mihomo `/ui`；zashboard 对 sing-box 的兼容性未验证。
- 考虑过的备选：mihomo + sing-box 同时强制 zashboard。
- 未采用原因：扩大范围且可能引入不兼容。
- 影响范围：R-003、R-006、AC-006。

## 6. 验收标准

### AC-001：构建包包含 zashboard

- 对应规则：R-001、D-001、D-003
- 前置条件：构建环境可访问 zashboard release。
- 操作或输入：执行 `./build.sh`。
- 预期结果：生成的 `box-*.zip` 内存在 `box/mihomo/dashboard/index.html`，且文件来自 zashboard 的 `dist-no-fonts.zip`。
- 验证状态：通过
- 验证证据：2026-11-23 执行 `ZASHBOARD_ARCHIVE=<本地 dist-no-fonts.zip> ./build.sh`，生成的模块包包含 `box/mihomo/dashboard/index.html`。

### AC-002：ADB 升级后 /ui 为 zashboard

- 对应规则：R-002、R-003、D-002
- 前置条件：设备已有旧模块，`mihomo/dashboard` 为 MetacubeXD，`config.yaml` 含旧 `external-ui-url` 或 `external-ui: ./ui`。
- 操作或输入：通过 ADB 安装新模块包并打开 `http://127.0.0.1:9090/ui/`。
- 预期结果：`external-ui` 为 `./dashboard`；`external-ui-url` 不存在；面板目录为 zashboard；`/ui` 显示 zashboard 而非 METACUBE (XD)。
- 验证状态：未执行
- 验证证据：尚未执行；需要真机通过 ADB 覆盖安装验证。

### AC-003：核心配置页只读展示

- 对应规则：R-004、D-004
- 前置条件：当前核心为 mihomo。
- 操作或输入：打开 WebUI 的 mihomo 核心配置页。
- 预期结果：`external-ui` 显示 `./dashboard`，没有可编辑输入框；页面存在「更新面板」按钮。
- 验证状态：通过
- 验证证据：`npm test -- --run src/features/core-config/core-config-page.test.tsx` 验证只读字段和更新按钮。

### AC-004：一键更新成功

- 对应规则：R-004、R-005、D-004
- 前置条件：当前核心为 mihomo，网络可访问 zashboard release。
- 操作或输入：点击「更新面板」并等待任务完成。
- 预期结果：任务中心出现「更新面板资源」任务并成功；`/ui` 加载最新版 zashboard。
- 验证状态：通过
- 验证证据：`npm test -- --run src/features/core-config/core-config-page.test.tsx` 验证更新按钮触发任务提示；`test/module-dashboard.test.sh` 验证 `upxui` 成功替换。

### AC-005：更新失败保留旧面板

- 对应规则：R-005
- 前置条件：当前面板为 zashboard 旧版本，模拟下载或解压失败。
- 操作或输入：触发面板更新。
- 预期结果：任务失败并显示错误；当前 `dashboard/index.html` 仍存在且可访问。
- 验证状态：通过
- 验证证据：`test/module-dashboard.test.sh` 模拟 curl 失败，断言日志记录下载失败且当前 `index.html` 内容仍为 `KEEP`。

### AC-006：sing-box 不受影响

- 对应规则：R-003、R-006、D-005
- 前置条件：当前核心为 sing-box。
- 操作或输入：安装/升级模块，或打开核心配置页。
- 预期结果：sing-box 的 `external_ui` 配置和面板资源不被修改；mihomo 专属更新入口不显示。
- 验证状态：未执行
- 验证证据：尚未执行；需要 sing-box 真机/配置回归。

## 7. 未决问题与假设

### Q-001：构建环境网络可用性

- 类型：暂用假设
- 内容：构建机和 CI 可访问 `github.com/Zephyruso/zashboard` 的最新 release。
- 影响范围：R-001、AC-001。
- 是否阻塞：否；若不可用，构建应明确失败，由发布流程处理。
- 决定或验证方式：在 CI 构建日志和本地 `./build.sh` 中验证；允许通过环境变量指定本地已下载的 zashboard 包做离线测试。

## 8. 技术设计

### 8.1 构造与打包

- `build.sh` 增加 zashboard 获取阶段：
  1. 下载最新 `dist-no-fonts.zip` 到临时目录（或读取 `ZASHBOARD_ARCHIVE` 指定的本地包）。
  2. 解压到临时目录，定位 `index.html`。
  3. 将 `index.html` 所在目录的内容复制到模块临时面板目录 `box/mihomo/dashboard/`。
  4. 将面板目录加入最终 zip；构建结束清理临时目录。
- 下载或校验失败时 `build.sh` 退出非零，不产出缺少面板的包。
- 面板目录不提交到 Git；由构建流程生成。

### 8.2 安装与升级

- `customize.sh` 在移动新 `box/` 内容前，把内置 `box/mihomo/dashboard/` 暂存到 `$MODPATH/zashboard`。
- 恢复旧 `mihomo/` 配置后，强制：
  - 删除并重新创建 `/data/adb/box/mihomo/dashboard/`，从 `$MODPATH/zashboard` 复制内置面板。
  - 用 `sed` 将 `external-ui` 设为 `./dashboard`；删除 `external-ui-url` 行。
- 安装结束清理 `$MODPATH/zashboard`。

### 8.3 运行时更新

- `box.tool` 的 mihomo 分支固定目标目录为 `/data/adb/box/mihomo/dashboard`，下载 `dist-no-fonts.zip`。
- 下载、解压和 `index.html` 校验全部在临时目录完成；通过后才替换目标目录，失败时保留现有目录。
- 现有 `task xui` 继续作为任务类型；WebUI 的「更新面板」按钮只调用 `api.taskStart("xui")`，不新增独立 API。

### 8.4 WebUI

- mihomo 核心配置页将 `external-ui` 输入框改为只读展示。
- 页面新增「更新面板」按钮，按钮状态与任务提示沿用现有 `useToast` / `taskStart` 机制。

### 8.5 模块与职责

| 模块 | 职责 |
|---|---|
| `build.sh` | 获取 zashboard、构建模块 zip、校验面板资源 |
| `customize.sh` | 安装/升级时恢复用户数据并强制部署内置 zashboard |
| `box/scripts/box.tool` | `upxui` 下载/校验/原子替换；`task xui` 执行入口 |
| `webui/src/features/core-config/core-config-page.tsx` | 只读展示 `external-ui`、触发面板更新任务 |
| `webui/src/api/client.ts` | 复用 `taskStart("xui")` |
| `test/module-webui.test.sh` / Vitest | 覆盖构建、任务、只读 UI 和失败保留行为 |
