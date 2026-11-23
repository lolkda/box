# 应用包名选择器桥接兼容

## 1. 文档信息

- 文档标识：module-package-picker-bridge
- 规格状态：已确认
- 最后更新：2026-11-24
- 实现状态：已实现
- 验证状态：部分完成（本地 37 项前端测试、typecheck、build 通过；未真机验证）
- 关联文档：[module-webui-refactor.md](module-webui-refactor.md)（应用与网络页历史规格，Q-002 假设 `pm.support` 可用）
- 关联文档：[module-webui-config-expansion.md](module-webui-config-expansion.md)（应用与网络清单格式与页面）

## 2. 目标与范围

### 背景

应用包名选择器依赖 KernelSU WebView 的包管理桥接。当前实现只检测 `window.ksu.pm`，并且把桥接异常吞成空数组。

核对官方源码后确认：

- 官方 KernelSU v3.3.1 的 `WebViewInterface` 没有 `pm()`，只有全局 `listPackages(type)` 和 `getPackagesInfo(packagesJson)`。
- 官方 master 已新增 `pm()`，但 v3.3.1 发布版没有。
- `listPackages` 返回包名数组；`getPackagesInfo` 接收包名数组并返回应用名、UID、系统应用标记等信息。
- 两套接口都不是二选一：旧接口需要两个函数配合；新接口 `pm()` 也提供同名方法。

因此当前设备上的警告是能力检测误判，而不是设备真的没有任何包名 API。

### 目标

- 让官方 KernelSU 3.3.1 的包名选择器恢复可用，能显示应用名和包名。
- 未来支持 `ksu.pm()` 的 Manager 自动使用新接口，不维护两套页面。
- 桥接不可用、调用失败、返回空列表时，用户能区分原因，并始终可以手动输入包名。
- 增加本地自动化测试，覆盖新接口、旧接口和失败路径。

### 非目标

- 本次不实现 root shell `pm list packages` 回退。
- 本次不改变“选择包名后写入 UID 0”的现有行为；工作资料/多用户选择留作后续。
- 本次不新增应用图标、应用大小或版本筛选。
- 本次不修改 `package.list.cfg` 的 `UID:包名` 格式，不修改应用清单保存协议。
- 本次不重构应用与网络页的其他 Tab。

## 3. 参与深度与委托

- 参与深度：架构与设计
- 局部例外：无
- 需要用户确认：支持 `ksu.pm()` 优先、官方 3.3.1 旧全局接口兜底；不执行 shell；手动输入与错误提示。
- 模型自主决定：桥接解析函数、错误分类、列表去重、加载状态、测试 fixture 和具体 UI 文案。
- 当前执行授权：用户选择“新建独立规格，保存并开始开发”；允许写入本规格、前端桥接、选择器、测试和构建产物；不包含 Git 提交、推送、发布或真机操作。

## 4. 行为要求与约束

### R-001：桥接选择顺序

- 参与者或触发方：`PackagePicker` 打开时、包名查询函数。
- 前置条件与输入：`window.ksu` 已由 KernelSU WebView 注入。
- 必须满足的行为：
  - 如果 `ksu.pm` 可调用且返回带 `listPackages`、`getPackagesInfo` 方法的对象，优先使用新接口。
  - 否则如果 `ksu.listPackages` 和 `ksu.getPackagesInfo` 都是函数，使用旧全局接口。
  - 兼容 `ksu.pm` 是对象而非函数的实现；两种形态都按新接口处理。
  - 不使用 `ksu.pm` 的 `support` 静态值作为唯一判断，因为 SDK/管理器版本可能不一致。
- 输出或状态变化：返回统一的桥接对象和接口类型。
- 边界与失败行为：两套接口都不存在时返回明确的“不可用”状态，不抛出未处理异常。

### R-002：包名和应用信息查询

- 参与者或触发方：包名选择器、应用与网络页。
- 前置条件与输入：已解析出桥接；用户打开选择器。
- 必须满足的行为：
  - 旧接口先调用 `listPackages("user")`，再调用 `getPackagesInfo(JSON.stringify(names))`。
  - 新接口调用 `pm().listPackages("user")` 和 `pm().getPackagesInfo(JSON.stringify(names))`。
  - 解析 JSON 字符串或数组；`getPackagesInfo` 返回项必须有 `packageName`，`error` 项跳过。
  - 按 `packageName` 去重；保留首个记录的应用名、UID、系统应用标记。
  - 应用名缺失时回退到包名；结果按应用名/包名稳定排序。
- 输出或状态变化：返回 `PackageInfo[]`，供选择器展示。
- 边界与失败行为：JSON 解析失败、方法抛错、返回结构不是数组时返回分类错误，不返回伪空列表。

### R-003：不可用、失败和空列表的区分

- 参与者或触发方：包名选择器。
- 前置条件与输入：打开选择器或点击重试。
- 必须满足的行为：
  - 两套桥接都不存在：显示“当前管理器未提供包列表接口”，保留手动输入。
  - 桥接存在但调用失败：显示具体失败信息，保留手动输入和重试。
  - 桥接成功但返回空数组：显示“没有读取到应用”，不显示“接口不可用”。
  - 加载中、加载成功、不可用、失败、空列表必须是不同状态。
- 输出或状态变化：用户能判断下一步是升级 Manager、重试还是手动输入。
- 边界与失败行为：错误详情不包含敏感信息；不把异常吞成空数组。

### R-004：检测时机

- 参与者或触发方：`PackagePicker` 生命周期。
- 前置条件与输入：页面已加载，`window.ksu` 可能在首屏渲染后才完成注入。
- 必须满足的行为：
  - 不在模块加载阶段把“可用性”固化成一次布尔值。
  - 每次打开选择器时重新解析桥接。
  - 选择器关闭后不保留上一次的失败状态；重新打开重新查询。
- 输出或状态变化：桥接晚注入或切换环境后，重新打开即可恢复。
- 边界与失败行为：重试仍失败时保留错误状态。

### R-005：选择器和手动输入

- 参与者或触发方：包名选择器。
- 前置条件与输入：选择器打开，已有选中值。
- 必须满足的行为：
  - 无论桥接是否可用，都提供手动输入包名的入口。
  - 手动输入遵循 Android 包名格式校验；非法值不加入选择。
  - 已有选中值在重新打开时保留；应用选择只返回包名数组。
  - 本次继续以 UID 0 写入选择结果，不改变现有 `addPickedPackages` 行为。
- 输出或状态变化：`onChange` 收到包名数组；页面继续生成 `{ uid: "0", package }`。
- 边界与失败行为：空输入、重复包名、非法包名不产生条目。

### 相关约束

- 不把完整包列表、UID 或错误详情写入日志。
- 不引入新的 npm 依赖；直接使用 Promise 包装的 WebView 桥接。
- 不修改 KernelSU Manager 或模块后端。
- 旧接口是官方 3.3.1 的唯一可用方式，不能因为标记 deprecated 就删除。
- 本次不执行 shell，不把 `pm list packages` 作为隐式回退。

## 5. 关键决定

### D-001：`ksu.pm()` 优先，旧全局接口兜底

- 状态：已确认
- 结论：新 Manager 走 `ksu.pm()`；官方 3.3.1 走 `ksu.listPackages` + `ksu.getPackagesInfo`。
- 理由：官方 3.3.1 没有 `pm()`，只支持新接口会让当前设备继续不可用；旧接口仍由官方保留。
- 考虑过的备选：只支持 `ksu.pm()`；只支持旧接口；shell。
- 未采用原因：只支持 pm 无法修复 3.3.1；只支持旧接口放弃未来多用户；shell 没有应用名且额外启动 root 进程。
- 影响范围：R-001、R-002、AC-001、AC-002。

### D-002：`listPackages` 与 `getPackagesInfo` 配合使用

- 状态：已确认
- 结论：先列包名，再批量补应用信息；不能二选一。
- 理由：`listPackages` 只有包名；`getPackagesInfo` 需要包名输入。
- 考虑过的备选：只显示包名；逐个调用 `getPackagesInfo`。
- 未采用原因：只显示包名丢失应用名；逐个调用增加桥接次数。
- 影响范围：R-002、AC-001、AC-002。

### D-003：错误显式化并保留手动输入

- 状态：已确认
- 结论：不可用、调用失败、空列表分别展示；弹窗始终允许手动输入包名。
- 理由：当前空数组把三种情况混在一起，用户不知道是环境不支持还是确实没有应用。
- 考虑过的备选：只显示警告；失败时自动退回手动输入但不说明原因。
- 未采用原因：无法排障，也无法判断是否需要更新 Manager。
- 影响范围：R-003、R-005、AC-003、AC-004、AC-005、AC-006。

### D-004：本次保持 UID 0

- 状态：委托范围内决定
- 结论：包名选择结果继续由页面写入 UID 0；不引入多用户选择 UI。
- 理由：本次目标是恢复包名获取；多用户会影响选择结果结构和 `package.list.cfg` 写入语义，应独立设计。
- 考虑过的备选：按 UID 展示；只显示主用户。
- 未采用原因：会扩大页面和数据模型改动范围。
- 影响范围：R-005、AC-006。

## 6. 验收标准

### AC-001：新 pm 接口

- 对应规则：R-001、R-002
- 前置条件：模拟 `window.ksu.pm` 返回带 `listPackages` / `getPackagesInfo` 的对象。
- 操作或输入：打开包名选择器。
- 预期结果：显示应用名和包名；选择后返回包名数组。
- 验证状态：通过
- 验证证据：`webui/src/api/client.test.ts` 覆盖 `ksu.pm()` 对象形态，`listInstalledPackages` 返回去重后的应用信息。

### AC-002：官方 3.3.1 旧接口

- 对应规则：R-001、R-002
- 前置条件：模拟没有 `ksu.pm`，只有 `ksu.listPackages` / `ksu.getPackagesInfo`。
- 操作或输入：打开包名选择器。
- 预期结果：仍显示应用名和包名，不再显示“未提供应用列表 API”。
- 验证状态：通过
- 验证证据：`client.test.ts` 和 `package-picker.test.tsx` 覆盖无 `pm`、仅有 `listPackages`/`getPackagesInfo` 的官方 3.3.1 形态，能显示应用名。

### AC-003：两套接口都不存在

- 对应规则：R-003、R-005
- 前置条件：`window.ksu` 不含任何包管理方法。
- 操作或输入：打开包名选择器。
- 预期结果：显示明确不可用提示和手动输入；不显示伪空列表。
- 验证状态：通过
- 验证证据：`package-picker.test.tsx` 覆盖无桥接时显示手动输入；`client.test.ts` 断言 `PACKAGE_BRIDGE_UNAVAILABLE`。

### AC-004：桥接调用失败

- 对应规则：R-002、R-003
- 前置条件：桥接存在但 `listPackages` 抛错或返回非法 JSON。
- 操作或输入：打开选择器或点击重试。
- 预期结果：显示失败提示和重试/手动输入；不把异常吞成空数组。
- 验证状态：通过
- 验证证据：`client.test.ts` 覆盖非法 JSON 返回 `PACKAGE_BRIDGE_BAD_RESPONSE`。

### AC-005：成功但空列表

- 对应规则：R-003
- 前置条件：桥接存在，`listPackages` 返回 `[]`。
- 操作或输入：打开选择器。
- 预期结果：显示“没有读取到应用”，不显示“接口不可用”。
- 验证状态：通过
- 验证证据：`package-picker.test.tsx` 覆盖空列表显示“没有读取到应用”，不显示接口不可用。

### AC-006：手动输入与 UID 0

- 对应规则：R-005
- 前置条件：选择器打开，桥接不可用或用户不想选择列表。
- 操作或输入：输入合法包名并确认；再输入非法包名。
- 预期结果：合法包名加入选择，非法包名被拒绝；页面继续以 UID 0 写入。
- 验证状态：通过
- 验证证据：`package-picker.test.tsx` 覆盖合法手动包名加入选择、非法包名被拒绝；页面仍按现有 UID 0 写入。

### AC-007：检测时机

- 对应规则：R-004
- 前置条件：首次渲染时 `window.ksu` 尚未注入，打开选择器前注入。
- 操作或输入：打开选择器。
- 预期结果：打开时重新检测并读取列表，不沿用模块加载时的不可用状态。
- 验证状态：通过
- 验证证据：`package-picker.test.tsx` 覆盖首次打开时无桥接、随后注入旧接口再打开后重新检测成功。

## 7. 未决问题与假设

### Q-001：shell 回退

- 类型：未决问题
- 内容：如果未来某个 Manager 同时没有 `pm()` 和旧全局接口，是否增加 `pm list packages` shell 回退？
- 影响范围：R-001、R-003
- 是否阻塞：否，本次官方 3.3.1 已有旧接口；shell 只作为后续选项。
- 决定或验证方式：先观察实际 Manager 兼容性，再决定是否引入无应用名的包名回退。

### Q-002：多用户选择

- 类型：未决问题
- 内容：何时把 `getPackagesInfo` 返回的 `uid` / `userId` 暴露到选择器，并按用户写入前缀？
- 影响范围：R-005、D-004
- 是否阻塞：否，本次保持 UID 0，用户可以手动输入 `用户ID:包名`。
- 决定或验证方式：需要支持工作资料/多用户时另开规格。

## 8. 技术设计

### 8.1 模块与职责

- `webui/src/api/client.ts`：
  - 增加桥接解析类型和解析函数。
  - 保留 `packageManagerAvailable()` 作为轻量兼容判断，内部改为解析桥接。
  - `listInstalledPackages()` 返回结构化结果或抛出分类错误，不再吞掉所有异常。
- `webui/src/components/ui/package-picker.tsx`：
  - 打开时调用查询，维护 `idle/loading/ready/unavailable/error/empty` 状态。
  - 显示错误、重试和手动输入。
  - 保留现有选中集合与 `onChange(string[])` 契约。
- `webui/src/api/types.ts`：
  - 如需要增加 `PackageInfo` 字段或查询结果类型；不改变 `PackageEntry`。
- `webui/src/dev/mock-ksu.ts`：
  - 保留 `pm` 模拟，并增加无 `pm` 的测试场景。
- 测试：
  - 新增 `package-picker.test.tsx` 或 client 层测试，覆盖 pm、legacy、unavailable、error、empty、manual。

### 8.2 桥接解析

```ts
type PackageBridgeKind = "pm" | "legacy";

interface ResolvedPackageBridge {
  kind: PackageBridgeKind;
  listPackages(type: string): string;
  getPackagesInfo(packages: string): string;
}
```

解析顺序：

1. `window.ksu.pm` 是函数：调用一次，得到对象；要求两个方法都是函数。
2. `window.ksu.pm` 是对象：直接检查两个方法。
3. `window.ksu.listPackages` 和 `window.ksu.getPackagesInfo` 都是函数：使用全局对象。
4. 都没有：返回 `null`。

### 8.3 查询与错误

- `listPackages("user")` 和 `getPackagesInfo(JSON.stringify(names))` 的结果既可能是 JSON 字符串，也可能是数组；两者都接受。
- 解析后的 `getPackagesInfo` 必须是数组；否则抛出 `PACKAGE_BRIDGE_BAD_RESPONSE`。
- 没有包名时不调用 `getPackagesInfo`，直接返回空列表。
- 错误类型至少区分：
  - `PACKAGE_BRIDGE_UNAVAILABLE`
  - `PACKAGE_BRIDGE_ERROR`
  - `PACKAGE_BRIDGE_BAD_RESPONSE`
- UI 将错误映射为可读提示；不显示原始异常堆栈。

### 8.4 选择器状态

- 打开时 `useEffect` 重新检测并加载。
- 顶部显示状态提示：
  - 不可用：需要 KernelSU Manager 包列表接口。
  - 失败：读取失败，可重试。
  - 空：没有读取到应用。
- 手动输入框始终存在；Enter 或按钮添加。
- 底部“应用选择（n）”只提交已选包名。

### 8.5 测试设计

- client 单测：pm 形态、pm 对象形态、legacy 形态、无桥接、抛错、非法 JSON、空数组。
- 组件测试：legacy 下能看到应用名；不可用显示手动输入；失败显示重试；手动输入合法/非法包名。
- 现有 `subscriptions`/`apps-network` 测试不得回归。
- 运行 typecheck、Vitest、build；不执行真机安装。
