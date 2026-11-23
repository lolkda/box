# 模块管理页面全链路重构

## 1. 文档信息

- 文档标识：module-webui-refactor
- 规格状态：已确认
- 最后更新：2026-11-23
- 实现状态：已完成
- 验证状态：本地自动验证通过；真机待用户复测
- 关联文档：[module-webui.md](module-webui.md)（旧 WebUI 实现规格，保留为历史；本规格记录重构后的当前实现）
- 关联文档：[module-subscription.md](module-subscription.md)

## 2. 目标与范围

### 背景

旧管理页（`webui/src/app.jsx`）是 459 行单文件 Preact 应用，所有页面、请求与状态管理集中在一个组件中，样式只有 74 行原生 CSS。实际以 390px 宽度检查时，顶部 8 个 tab 溢出，订阅表格把中文名字压成竖排，策略/规则/规则集表单在手机上大量截断，设置页 22 个字段堆成一张 1761px 长卡片。页面还存在切换 tab 丢失未保存修改、运行状态未加载时先显示“已停止”、日志不自动刷新、订阅删除无确认、订阅 URL/token 明文展示、长任务期间全页 `busy` 等问题。

后端 `box.tool` 的 JSON 接口在 1.4.0 中为页面新增，结构不统一：部分命令返回裸对象，部分命令只能输出文本，`config apply` 缺少规则引用校验，`settings apply` 对未知键静默忽略，数组值写回没有处理空格和引号。用户明确选择全链路重构，并接受重写现有 `--json` 输出、不保留旧 JSON 兼容。

### 目标

- 将 `webui/` 重写为 React + TypeScript + Vite 移动端管理应用。
- 采用 Android 原生工具风，底部 5 项导航：概览、订阅、策略、规则、更多。
- 覆盖现有全部功能：概览、服务控制、订阅、策略组、规则、规则集、设置、日志、危险操作。
- 将 `box.tool --json` 统一为 `{ok,data,error}` 包络，并补齐后端校验、错误码和原子写入。
- 新增后台任务中心，支持订阅更新、内核更新等长任务在 WebView 关闭后继续运行并可恢复查看。
- 提交成功的本地自动验证：Vite build、shell 回归、Vitest/Testing Library、Chromium 390/768/1280 截图。

### 非目标

- 不引入本地 HTTP 服务、守护进程或远程访问。
- 不修改代理核心本身的运行逻辑、iptables 规则或网络模式实现。
- 不提供多用户、权限体系或登录。
- 不自动备份危险操作；沿用旧规格 D-003，只做明确确认。
- 不包含真机部署、模块刷入或 Git 提交/推送；真机 KernelSU WebView 由用户按清单复测。
- 不保留旧 `--json` 输出兼容；CLI 文本命令保留。

## 3. 参与深度与委托

- 参与深度：架构与设计
- 局部例外：设置字段、规则构建器、任务中心和 JSON 接口结构由用户确认方向，具体字段名、组件拆分、状态管理和实现细节由模型决定。
- 需要用户确认：重构边界、技术栈、视觉方向、信息架构、JSON 兼容策略、长任务交互、设置覆盖范围、规则编辑深度、任务持久性、验收环境、文档处理方式均已确认。
- 模型自主决定：设计令牌具体值、组件 API、前端目录结构、路由实现、表单校验细节、JSON 错误码命名、任务目录结构、测试用例拆分。
- 当前执行授权：允许在 `lolkda/box` 当前工作树创建本规格、修改 `box.tool`、新增 `box/scripts` 辅助文件、重写 `webui/`、更新 `webroot/` 构建产物、更新 `README.md` 与 `CHANGELOG.md`、新增/修改本地测试；不包含真机部署、Git 提交或推送。

## 4. 行为要求与约束

### R-001：移动端应用壳与导航

- 参与者或触发方：用户打开 KernelSU 模块 WebUI。
- 前置条件与输入：`webroot/index.html` 可加载，KernelSU WebView 或本地 Chromium 模拟环境可用。
- 必须满足的行为：提供底部 5 项导航「概览 / 订阅 / 策略 / 规则 / 更多」；“更多”页面提供设置、规则集、日志、任务、危险操作入口；子页面提供返回；Android 返回键通过 hash 路由回到上一页。
- 输出或状态变化：当前页面有明确选中态；底部导航适配 safe-area；桌面/平板宽度下内容居中且不拉伸失控。
- 边界与失败行为：不支持的 hash 路由回到概览；KernelSU API 不可用时显示可读错误页，不出现 `ksu is not defined`。

### R-002：统一 JSON 契约

- 参与者或触发方：前端调用 `box.tool`。
- 前置条件与输入：命令带 `--json`。
- 必须满足的行为：所有 UI 读取/写入命令返回统一包络 `{"ok":true,"data":...,"meta":{"version":1}}` 或 `{"ok":false,"error":{"code":"...","message":"...","detail":"..."}}`；成功退出码 0，失败退出码非 0；`log` 在 JSON 模式下写 stderr，不污染 stdout。
- 输出或状态变化：前端只解析 stdout 的包络，按错误码映射提示。
- 边界与失败行为：非 JSON 输出、空输出、超时、命令失败均转为可读错误；不得把 stderr 当 JSON 解析。

### R-003：概览与服务控制

- 参与者或触发方：概览页。
- 必须满足的行为：首次加载显示骨架屏，不先显示“已停止”；展示运行状态、PID、核心、模式、配置、订阅数、活动任务数；支持启动、停止、重启、重载；重载调用 `box.tool reload` 而不是不存在的 `box.service reload`。
- 输出或状态变化：操作中按钮显示对应 loading，完成后刷新状态并提示结果。
- 边界与失败行为：服务操作失败显示 stderr/错误码；长操作不阻塞其他只读页面。

### R-004：订阅管理

- 参与者或触发方：订阅页。
- 必须满足的行为：列出订阅名、掩码 URL、provider 状态、文件大小/更新时间（可用时）；支持添加、编辑（改名/改 URL）、删除、更新全部、更新单条。
- 输出或状态变化：新增/编辑/删除后刷新列表；更新任务在任务中心展示；列表 URL 默认掩码，编辑时才通过 `sub get` 获取完整 URL。
- 边界与失败行为：URL 必须 http/https；名字不能为空或包含 `|`；生成的安全文件名冲突要拒绝；删除必须二次确认；部分订阅更新失败返回 `partial:true` 并展示失败数量。

### R-005：策略组

- 参与者或触发方：策略页。
- 必须满足的行为：列出策略组名称、类型、成员数、include-all 状态；支持新增、编辑、删除；类型至少支持 `select`、`url-test`、`fallback`；保留未在表单中编辑的字段（如 `icon`、`exclude-type`）。
- 输出或状态变化：保存调用 `config apply`，成功后刷新配置。
- 边界与失败行为：名称不能为空且不能重复；被规则引用的策略组删除必须由后端拒绝；保存失败不覆盖原配置。

### R-006：规则

- 参与者或触发方：规则页。
- 必须满足的行为：规则按顺序展示；支持结构化编辑常见类型（DOMAIN、DOMAIN-SUFFIX、DOMAIN-KEYWORD、IP-CIDR、IP-CIDR6、GEOIP、GEOSITE、RULE-SET、DST-PORT、PROCESS-NAME、MATCH）以及高级原文模式；支持新增、编辑、删除、上移、下移。
- 输出或状态变化：结构化模式即时生成规则字符串和预览；保存调用 `config apply`。
- 边界与失败行为：引用不存在的策略组/规则集拒绝保存；无法解析的高级原文保留原样并提示风险；规则为空或语法非法拒绝保存。

### R-007：规则集

- 参与者或触发方：“更多 → 规则集”页面。
- 必须满足的行为：列出名称、类型、behavior、format、interval、URL/path；支持新增、编辑、删除；保留未编辑字段。
- 输出或状态变化：保存调用 `config apply`。
- 边界与失败行为：名称不能为空；被规则引用的规则集删除必须由后端拒绝；URL 非空时必须是 http/https。

### R-008：设置

- 参与者或触发方：“更多 → 设置”页面。
- 必须满足的行为：覆盖 `settings.ini` 中所有用户可编辑标量键和数组键，并按基础、网络、代理、热点与网络控制、性能、更新与下载分组；布尔用 Switch，枚举用 Select，端口/内存/CPU 用合适输入；数组使用标签编辑器，应用包名可从已安装应用选择；敏感 token 显示“已设置/未设置”，不回填明文。
- 输出或状态变化：修改后出现 sticky 保存栏；保存成功刷新设置并清除 dirty；离开页面时若有未保存修改必须确认。
- 边界与失败行为：未知键、非法枚举、非法端口、非法数组项拒绝写入；写回前做 shell 语法校验；失败保留原文件；不修改内部路径/常量键。

### R-009：日志

- 参与者或触发方：“更多 → 日志”页面。
- 必须满足的行为：显示最近 200 行；运行中每 2 秒自动刷新；支持暂停/恢复自动刷新、按级别过滤（Info/Warn/Error）、关键词搜索、复制当前过滤结果、清空日志（二次确认）。
- 输出或状态变化：日志区域可滚动，保持底部跟随；文件路径可查看。
- 边界与失败行为：日志文件不存在显示空状态；自动刷新失败不弹重复错误，保留最后一次内容。

### R-010：危险操作

- 参与者或触发方：“更多 → 危险操作”页面。
- 必须满足的行为：删除配置、清空日志、重置订阅、更新 mihomo 核心、更新 sing-box 核心均可触发；每个操作必须在确认对话框中写清影响；核心更新通过任务中心执行。
- 输出或状态变化：确认后执行；成功后刷新相关数据。
- 边界与失败行为：取消不执行；不自动备份；错误显示错误码和详情。

### R-011：后台任务中心

- 参与者或触发方：订阅更新、内核更新、CNIP/Geo/面板更新。
- 必须满足的行为：`box.tool task start|status|logs|list|cancel` 提供任务生命周期；任务在 `run/tasks/<id>/` 保存 kind、title、state、pid、started_at、finished_at、exit_code、output.log；WebView 关闭后任务继续；重新打开页面后任务列表可恢复；同一时间只允许一个变更任务。
- 输出或状态变化：任务状态 `running/success/failed/canceled/interrupted`；前端任务中心轮询状态与日志，完成后 toast 并刷新受影响查询。
- 边界与失败行为：内核替换阶段不可取消；取消只对可取消任务生效；任务 ID 必须校验；旧任务最多保留 20 个。

### R-012：后端校验与错误

- 参与者或触发方：所有 JSON 写命令。
- 必须满足的行为：配置 apply 校验 `proxy-groups`、`rule-providers`、`rules` 结构、名称唯一性、策略组类型、规则集字段、RULE-SET/MATCH 引用；设置 apply 校验键白名单、值类型、枚举、端口、数组；错误返回稳定错误码。
- 输出或状态变化：校验失败不修改目标文件；成功写入采用临时文件加原子替换。
- 边界与失败行为：`E_VALIDATION`、`E_NOT_FOUND`、`E_CONFLICT`、`E_IO`、`E_BUSY`、`E_COMMAND`、`E_USAGE`、`E_CANCELED`、`E_INTERRUPTED`。

### R-013：视觉、可访问性与响应式

- 参与者或触发方：所有页面。
- 必须满足的行为：Android 原生工具风；品牌蓝 `#2563EB`；浅色背景 `#F6F7F9`、深色背景 `#0B0F14`；卡片 16px 圆角；触控目标不小于 44px；系统字体与等宽数字；支持 `prefers-color-scheme`、`prefers-reduced-motion`；所有表单有 label/错误提示；对话框有可访问名称。
- 输出或状态变化：390px、768px、1280px 下不出现横向溢出。
- 边界与失败行为：窄屏下表格改为卡片列表；底部抽屉不遮挡输入框；安全区适配。

### R-014：构建与测试

- 参与者或触发方：开发流程。
- 必须满足的行为：`npm run build` 输出到 `webroot/`；`npm test` 运行 Vitest；`npm run typecheck` 通过；`bash test/module-webui.test.sh` 通过；`webroot/index.html` 引用新的 hash 路由入口。
- 输出或状态变化：构建产物可离线加载，不依赖外部 CDN；不使用运行时字体/图标网络请求。
- 边界与失败行为：构建失败不得覆盖上一份可用 `webroot`；测试失败不得标记完成。

### 相关约束

- 页面运行在 Android WebView，不能依赖桌面浏览器专属 API。
- 所有 root 命令通过绝对路径 `/data/adb/box/scripts/box.tool` 或 `/data/adb/box/scripts/box.service`。
- 不得把订阅 URL、token、GitHub token 写入日志、页面标题或错误详情。
- `webui/` 是源码，`webroot/` 是构建产物；不能直接手改 `webroot/assets`。
- 现有 CLI 文本命令保持不变；破坏范围仅限 `--json` 输出。
- 不依赖 `window.kernelsu`；KernelSU SDK 使用全局 `ksu` 桥。

## 5. 关键决定

### D-001：全链路重构

- 状态：已确认
- 结论：前端完全重写，同时重构 `box.tool --json` 契约、校验与任务接口；保留 CLI 文本命令。
- 理由：现有 UI 债务与后端校验/接口问题同时存在，只重做皮肤会保留状态管理与数据契约缺陷。
- 考虑过的备选：只重做界面层；只改 CSS。
- 未采用原因：无法解决 tab 丢状态、长任务和配置校验问题。
- 影响范围：R-001 至 R-014。

### D-002：React + TypeScript + Vite + Tailwind + Radix

- 状态：已确认
- 结论：React 19 + TypeScript + Vite + Tailwind CSS + Radix primitives；TanStack Query、React Hook Form、Zod、React Router hash、Lucide。
- 理由：复杂动态表单、可访问弹层和任务状态需要成熟组件生态；Vite 保持构建简单；本地页面无网络依赖，包体可接受。
- 考虑过的备选：Preact + TypeScript + CSS 设计系统；Vue 3 + Vant；原生 Web Components。
- 未采用原因：用户明确选择 React/Radix；其他方案在组件生态或迁移成本上不占优。
- 影响范围：R-001、R-005 至 R-008、R-013、R-014。

### D-003：重写现有 `--json` 输出

- 状态：已确认
- 结论：所有 UI 使用的 `--json` 命令统一 `{ok,data,error}` 包络；不保留旧 JSON 字段兼容；CLI 文本输出保持。
- 理由：旧 JSON 接口形成时间短、结构不一致；一次性统一比长期维护双格式更干净。
- 考虑过的备选：新增 `box.tool ui` 门面并保留旧 JSON；只补后端校验。
- 未采用原因：用户明确选择直接重写，接受外部 JSON 消费者破坏。
- 影响范围：R-002、R-012，旧测试需重写。

### D-004：底部 5 项导航

- 状态：已确认
- 结论：概览 / 订阅 / 策略 / 规则 / 更多；设置、规则集、日志、任务、危险操作进入“更多”。
- 理由：手机单手操作下最稳，避免 8 tab 横向溢出。
- 考虑过的备选：顶部 8 tab 可滑动；首页卡片入口；左侧抽屉。
- 未采用原因：用户确认底部 5 项方案。
- 影响范围：R-001、R-013。

### D-005：Android 原生工具风

- 状态：已确认
- 结论：Material 3 线索、系统深浅色、底部抽屉、safe-area、44px 触控目标；品牌蓝沿用 `#2563EB`。
- 理由：运行环境是 Android KernelSU WebView，原生感降低学习成本。
- 考虑过的备选：深色技术仪表盘；iOS 软卡片；中性极简。
- 未采用原因：用户确认 Android 原生工具风。
- 影响范围：R-013。

### D-006：后台任务中心

- 状态：已确认
- 结论：任务由 shell 托管，新增 `box.tool task` 子命令；WebView 关闭后继续，重新打开可恢复。
- 理由：订阅/内核更新耗时长，不能绑定 WebView 生命周期。
- 考虑过的备选：页面内任务中心；阻塞式等待。
- 未采用原因：用户选择后台任务中心。
- 影响范围：R-011。

### D-007：设置页覆盖全部可编辑标量 + 数组

- 状态：已确认
- 结论：覆盖所有用户可编辑标量键与数组键；内部路径/常量键不暴露；GitHub token 脱敏。
- 理由：设置页应成为唯一可信管理入口。
- 考虑过的备选：保留当前 22 个字段；常用设置 + 高级 JSON。
- 未采用原因：用户选择完整覆盖。
- 影响范围：R-008、R-012。

### D-008：规则结构化构建器 + 高级原文

- 状态：已确认
- 结论：常见规则用结构化表单，复杂规则用高级原文模式；两者可切换并保留预览。
- 理由：减少策略组/规则集引用错误，同时保留高级能力。
- 考虑过的备选：仅原文编辑；只读 + 导入导出。
- 未采用原因：用户选择结构化构建器。
- 影响范围：R-006、R-012。

### D-009：后端权威校验

- 状态：已确认
- 结论：配置与设置校验在后端执行，前端做同构即时提示；后端失败不写文件。
- 理由：root 权限接口不能信任 WebView 输入；旧实现校验缺口已确认存在。
- 考虑过的备选：只在前端校验。
- 未采用原因：可绕过，且无法保护 CLI 调用。
- 影响范围：R-005 至 R-008、R-012。

### D-010：敏感值脱敏

- 状态：已确认
- 结论：`githubtoken` 不在 `settings get` 返回明文；订阅列表只返回 `url_masked`；完整 URL 仅在编辑时 `sub get` 获取。
- 理由：避免 token 出现在页面截图、日志和缓存中。
- 考虑过的备选：全部明文展示。
- 未采用原因：root 管理页截图和远程调试风险不可接受。
- 影响范围：R-004、R-008。

### D-011：验收边界

- 状态：已确认
- 结论：本地自动验收 + 真机清单；真机 KernelSU WebView 结果由用户复测并单独记录。
- 理由：当前开发环境没有 Android 真机，不能伪造真机通过。
- 考虑过的备选：必须真机通过；只构建。
- 未采用原因：用户选择本地自动验收并接受真机后续复测。
- 影响范围：R-014、AC-014、AC-015。

### D-012：新建重构规格

- 状态：已确认
- 结论：新建 `docs/specs/module-webui-refactor.md`；旧 `module-webui.md` 保留为历史，不标为取代。
- 理由：旧文档记录 1.4.0 的 Preact 实现，重构规格需要独立决策和验收；用户明确选择新建。
- 考虑过的备选：更新旧文档。
- 未采用原因：用户选择保留旧文档作为历史。
- 影响范围：文档维护。

## 6. 验收标准

### AC-001：应用构建与响应式

- 对应规则：R-001、R-013、R-014
- 前置条件：依赖安装完成，mock KernelSU 数据可用。
- 操作或输入：执行 `npm run build`，在 Chromium 以 390×844、768×1024、1280×900 打开页面。
- 预期结果：构建成功；三个宽度下无横向溢出；底部导航可见；内容不被安全区遮挡。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-002：导航与返回

- 对应规则：R-001
- 前置条件：页面加载完成。
- 操作或输入：依次点击 5 个底部入口；进入“更多 → 设置”后点击返回；修改 hash 为未知路由。
- 预期结果：选中态正确；返回回到上一页；未知路由回到概览；无 `ksu is not defined`。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-003：服务控制

- 对应规则：R-003
- 前置条件：mock 状态为 stopped，服务命令桩返回成功。
- 操作或输入：点击启动，待状态刷新；点击重载；断开 KernelSU mock 后刷新。
- 预期结果：启动期间按钮 loading；状态更新为运行中；重载调用 `box.tool reload`；KernelSU 不可用时显示友好错误。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-004：订阅 CRUD 与更新

- 对应规则：R-004、R-010
- 前置条件：订阅列表为空。
- 操作或输入：添加合法订阅；编辑名称和 URL；删除并在确认框取消；再次删除并确认；启动全部更新任务。
- 预期结果：列表出现后更新；取消删除不执行；确认后消失；URL 默认掩码；更新任务出现在任务中心。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-005：策略组校验

- 对应规则：R-005、R-012
- 前置条件：配置中有一个策略组被规则引用。
- 操作或输入：新增重名策略组；删除被引用策略组；新增合法策略组。
- 预期结果：重名和删除被拒绝并显示错误；合法新增保存后刷新；原配置不被破坏。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-006：规则结构化编辑

- 对应规则：R-006、R-012
- 前置条件：配置中有策略组和规则集。
- 操作或输入：新增 `RULE-SET` 规则并选择不存在的规则集；改为存在的规则集；上移规则；切到高级原文模式。
- 预期结果：不存在引用被拒绝；合法规则保存成功；顺序变化；高级模式保留原文。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-007：规则集 CRUD

- 对应规则：R-007、R-012
- 前置条件：规则引用一个规则集。
- 操作或输入：删除该规则集；新增合法规则集；修改 interval。
- 预期结果：被引用删除被拒绝；新增/修改成功；配置其余段保持不变。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-008：设置完整性与脏状态

- 对应规则：R-008、R-010
- 前置条件：默认 settings.ini。
- 操作或输入：修改 network_mode、加入 CIDR、添加应用包名、设置 GitHub token；直接切页；返回后保存。
- 预期结果：切页提示未保存；保存后文件正确；数组含空格/特殊字符仍可解析；token 不回填明文；其他键不变。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-009：日志体验

- 对应规则：R-009
- 前置条件：日志文件有 200 行以上。
- 操作或输入：打开日志页；筛选 Error；搜索关键词；暂停自动刷新；清空日志并取消。
- 预期结果：只显示匹配行；暂停后不再轮询；取消清空不执行；日志区域可滚动。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-010：危险操作确认

- 对应规则：R-010
- 前置条件：页面已打开。
- 操作或输入：点击删除配置，取消；再次点击并确认。
- 预期结果：取消不执行；确认对话框写明不可逆影响；确认后调用对应命令并刷新。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-011：后台任务生命周期

- 对应规则：R-011
- 前置条件：无运行中任务。
- 操作或输入：启动 `sub-update` 任务；轮询状态和日志；刷新页面；尝试取消 `upkernel`。
- 预期结果：任务状态 running→success；页面刷新后任务仍可查询；upkernel 不可取消；并发任务返回 E_BUSY。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-012：JSON 包络与错误码

- 对应规则：R-002、R-012
- 前置条件：`BOX_HOME` 指向临时目录。
- 操作或输入：调用 status、settings get、config apply、非法规则、非法设置、未知任务。
- 预期结果：成功均为 `ok:true`；失败均为 `ok:false` 且错误码稳定；stdout 无日志污染。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-013：后端配置校验

- 对应规则：R-005 至 R-008、R-012
- 前置条件：临时 `BOX_HOME` 中有默认配置。
- 操作或输入：提交重名策略组、缺失字段、非法规则引用、非法设置枚举、非法数组。
- 预期结果：全部返回非零和 `E_VALIDATION`；原文件哈希不变。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-014：本地测试

- 对应规则：R-014
- 前置条件：依赖安装完成。
- 操作或输入：执行 `npm run typecheck`、`npm test`、`npm run build`、`bash test/module-webui.test.sh`。
- 预期结果：全部退出码 0；Vitest 覆盖 API 包络、导航、设置 dirty、任务中心基本流程。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-015：浏览器控制台与视觉检查

- 对应规则：R-001、R-013
- 前置条件：mock 数据完整。
- 操作或输入：Chromium 打开 390/768/1280 页面，逐页浏览并检查 console。
- 预期结果：无未处理异常；无横向滚动；关键页面截图归档。
- 验证状态：未执行
- 验证证据：尚未执行

### AC-016：CLI 文本兼容

- 对应规则：R-002、R-014
- 前置条件：临时 `BOX_HOME`。
- 操作或输入：执行 `box.tool status`、`sub list`、`settings get`、`config get` 不带 `--json`。
- 预期结果：文本输出仍可用；不因 JSON 重构破坏脚本调用。
- 验证状态：未执行
- 验证证据：尚未执行

## 7. 未决问题与假设

### Q-001：真机 KernelSU WebView 版本差异

- 类型：暂用假设
- 内容：假设 Android 10+ 设备的系统 WebView 支持 React 19 构建产物与 Tailwind CSS 输出；不同厂商的 WebView 版本可能造成渲染差异。
- 影响范围：R-013、R-014、AC-015
- 是否阻塞：否，不影响本地实现与自动验收；真机结果按 D-011 单独记录。
- 决定或验证方式：用户按真机清单复测并反馈截图/控制台。

### Q-002：KernelSU `pm` API 可用性

- 类型：暂用假设
- 内容：应用包名选择器依赖 `pm.support` 和 `pm.listPackages`；旧版 KernelSU 管理器可能不提供该桥。
- 影响范围：R-008、R-013
- 是否阻塞：否，不支持时降级为手动输入包名。
- 决定或验证方式：运行时检测 `pm.support`，并在界面显示降级提示。

### Q-003：旧 JSON 外部消费者

- 类型：暂用假设
- 内容：无法确认仓库外是否存在脚本消费旧 `--json` 字段；用户已选择破坏性重写。
- 影响范围：R-002、R-012
- 是否阻塞：否，已在变更日志与 README 中说明。
- 决定或验证方式：发布说明记录破坏性变更；不在本任务中提供兼容层。

## 8. 技术设计

### 架构与模块职责

- `box/scripts/box.tool`：保留 CLI 文本入口；新增统一 JSON 包络、配置/设置校验、订阅 JSON 命令、任务命令。
- `box/scripts/box.service`：服务生命周期保持；页面通过 `box.tool service` 包装调用。
- `webui/src/api/`：类型化 `exec` 桥、临时 JSON 文件、包络解析、错误映射。
- `webui/src/features/`：概览、订阅、策略、规则、规则集、设置、日志、任务、危险页面。
- `webui/src/components/ui/`：Button、Card、Field、Input、Select、Switch、Sheet、Dialog、AlertDialog、Badge、Toast、Skeleton、TagInput。
- `webui/src/app/`：QueryClient、RouterProvider、AppShell、BottomNav、TaskCenter。
- `webui/src/styles/index.css`：Tailwind 入口、设计令牌、深浅色、safe-area、动画降级。
- `test/module-webui.test.sh`：JSON 契约、配置/设置校验、任务生命周期回归。
- `webui/src/**/*.test.tsx`：组件与 API 桥测试。
- `webroot/`：Vite 构建产物；不手改。

### 数据和状态

- 服务器状态：TanStack Query 管理，状态查询 5 秒轮询，日志查询页面可见时 2 秒轮询，任务查询 1.5 秒轮询。
- 表单状态：React Hook Form + Zod；设置表单支持 dirty 检测与离开确认。
- 路由：`createHashRouter`，底部导航 5 项；子页使用 history 返回。
- 错误：`ApiError` 带 `code/message/detail`；Toast 只显示 message，详情在可展开区域。
- 任务：全局 `TaskCenterProvider` 订阅任务查询；运行中任务显示在应用头部与任务页。

### JSON 接口

| 接口 | 调用方式 | 成功 data 关键字段 | 主要错误 |
|---|---|---|---|
| 状态 | `status --json` | running, pid, core, mode, config, config_exists, subscriptions, active_tasks | E_COMMAND |
| 服务 | `service <start\|stop\|restart\|reload> --json` | action, output, status | E_COMMAND、E_USAGE |
| 订阅列表 | `sub list --json` | items[].name, url_masked, provider, provider_size, updated_at | E_IO |
| 订阅详情 | `sub get <name> --json` | name, url, provider, provider_size | E_NOT_FOUND |
| 添加订阅 | `sub add <url> [name] --json` | name, url_masked | E_VALIDATION、E_CONFLICT |
| 编辑订阅 | `sub set <old> <new> <url> --json` | name, url_masked | E_VALIDATION、E_CONFLICT、E_NOT_FOUND |
| 删除订阅 | `sub rm <name> --json` | name | E_NOT_FOUND |
| 更新订阅 | `sub update [name] --json` | updated, partial, failed | E_COMMAND、E_NOT_FOUND |
| 设置读取 | `settings get --json` | settings, arrays, sensitive.githubtoken | E_IO |
| 设置写入 | `settings apply <file> --json` | changed[] | E_VALIDATION、E_IO |
| 配置读取 | `config get --json` | proxy-groups, rule-providers, rules | E_IO |
| 配置写入 | `config apply <file> --json` | changed[] | E_VALIDATION、E_IO |
| 配置删除 | `config delete --json` | path | E_NOT_FOUND、E_IO |
| 日志 | `logs [n] --json` | file, lines[] | E_IO |
| 清空日志 | `logs clear --json` | cleared[] | E_IO |
| 重置订阅 | `reset subscriptions --json` | cleared | E_IO |
| 任务启动 | `task start <kind> [arg] --json` | id, kind, title, state, cancelable | E_BUSY、E_VALIDATION |
| 任务状态 | `task status <id> --json` | id, state, exit_code, started_at, finished_at, pid | E_NOT_FOUND |
| 任务日志 | `task logs <id> [n] --json` | id, file, lines[] | E_NOT_FOUND |
| 任务列表 | `task list --json` | items[] | E_IO |
| 任务取消 | `task cancel <id> --json` | id, state | E_NOT_FOUND、E_VALIDATION |

### 任务目录与状态

```text
${box_run}/tasks/<task-id>/
├── kind
├── title
├── state          # running/success/failed/canceled/interrupted
├── pid
├── started_at
├── finished_at
├── exit_code
└── output.log
```

- 任务 ID 采用 `YYYYMMDDHHMMSS-PID`，只允许数字和连字符。
- `task start` 检查是否有 running 任务；有则返回 `E_BUSY`。
- `task _run` 为内部子命令，只接受白名单 kind。
- `upkernel` 的 `cancelable=false`，其余任务为 true。
- 任务完成后最多保留 20 个目录。

### 视觉令牌

| 令牌 | 浅色 | 深色 |
|---|---|---|
| `--color-bg` | `#F6F7F9` | `#0B0F14` |
| `--color-surface` | `#FFFFFF` | `#141A21` |
| `--color-surface-muted` | `#EEF1F5` | `#1B232C` |
| `--color-fg` | `#101418` | `#F3F6F9` |
| `--color-fg-muted` | `#667085` | `#98A2B3` |
| `--color-border` | `#E4E7EC` | `#26303B` |
| `--color-primary` | `#2563EB` | `#60A5FA` |
| `--color-success` | `#16A34A` | `#4ADE80` |
| `--color-warning` | `#D97706` | `#FBBF24` |
| `--color-danger` | `#DC2626` | `#F87171` |

### 已知限制与取舍

- 真机 KernelSU WebView 不在当前环境，最终真机兼容性由用户复测。
- 旧 `--json` 字段不兼容，已在 CHANGELOG 中标记 breaking change。
- 应用包名选择器在 `pm.support=false` 时降级为文本输入。
- 任务取消对子进程采用尽力而为的 TERM；内核替换阶段禁止取消。
- 前端依赖体积较 Preact 版本增加，但全部本地加载，无 CDN 依赖。

### 本地验证记录

- `cd webui && npm run typecheck`：通过。
- `cd webui && npm test`：4 个测试文件、11 项测试通过。
- `cd webui && npm run build`：通过，产物输出到 `webroot/`；路由已按页面懒加载。
- `bash test/module-webui.test.sh`：40 项通过；覆盖 JSON 包络、设置/配置校验、任务并发/取消、服务包装与 CLI 文本保留。
- `bash test/module-subscription.test.sh`：17 项通过。
- Chromium 390 / 768 / 1280 宽度逐页检查：无横向溢出；390px 深色模式检查通过。
- `webroot/` 生产产物通过静态服务器冒烟：无 KernelSU API 时正确降级到提示页。
- 最终截图保存在本地 `artifacts/final/`，不作为源码提交。
