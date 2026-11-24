# 订阅 proxy-providers 整块合并

## 1. 文档信息

- 文档标识：module-subscription-provider-merge
- 规格状态：已确认
- 最后更新：2026-11-24
- 实现状态：已实现
- 验证状态：部分完成（完整 shell 测试 156/156 通过；真机 mihomo -t 与设备 yq 兼容性未验证）
- 关联文档：[module-subscription-auto-provider.md](module-subscription-auto-provider.md)（现有订阅自动识别与 provider 拆分规格）
- 关联文档：[module-subscription.md](module-subscription.md)（订阅链路历史规格）

本规格补充现有 provider 订阅行为：订阅中的 `proxy-providers` 不再只提取 URL，而是把整个 provider 定义块合并进运行时配置。

## 2. 目标与范围

### 背景

当前 Box 的订阅更新只读取 `proxy-providers` 中每个 provider 的 `url`，然后在 `config.yaml` 中重新生成固定模板：

```yaml
type: http
url: <provider URL>
path: ./proxy_provider/<safe>.yaml
interval: 86400
health-check:
  enable: true
  url: https://cp.cloudflare.com
  interval: 300
  timeout: 1000
  tolerance: 100
```

这导致订阅自带的 `health-check`、`override`、`exclude-filter` 等字段被丢弃。实际测试中，xusx 节点本身可以通过代理访问网络，但默认 `https://cp.cloudflare.com` 在部分节点上 TLS 失败，面板显示全部节点无延迟。

### 目标

- 订阅存在 `proxy-providers` 时，把每个 provider 的完整定义块合并进 `config.yaml`。
- 同名 provider 用订阅里的整个 block 覆盖；不同名则追加。
- 除本地 `path` 外，provider 字段原样保留。
- 订阅只有顶层 `proxies` 时，继续生成一个 URL provider，并使用新的默认 health-check。
- 不删除订阅里未出现的旧 provider。
- 不导入 `proxy-groups`、`rules` 或 `rule-providers`。

### 非目标

- 不实现按订阅的 health-check 覆盖设置。
- 不实现 provider 字段白名单或字段级合并；block 整块覆盖/追加。
- 不实现 provider 列表收缩、旧 provider 清理或删除订阅后的自动清理。
- 不导入订阅中的 `proxy-groups` 和 `rules`。
- 不修改 DNS、TPROXY 端口、策略组和规则生成逻辑。
- 不执行 Git 提交、推送或发布。

## 3. 参与深度与委托

- 参与深度：架构与设计
- 局部例外：无
- 需要用户确认：
  - 同名 provider 整块覆盖
  - 除 path 外字段原样保留
  - 只支持 URL provider
  - `proxies`-only 订阅生成 URL provider
  - 新旧 provider 只合并不删除
  - 默认 health-check 使用 gstatic/5000
  - mixed source 拒绝
- 模型自主决定：
  - provider 定义缓存文件格式和位置
  - config.yaml 的原子写入和校验方式
  - provider 安全文件名、日志掩码和测试 fixture
- 当前执行授权：仅保存本规格；不修改代码、不安装、不提交、不推送。

## 4. 行为要求与约束

### R-001：订阅来源识别

- 参与者或触发方：`box.tool sub update` / `upsubs`。
- 前置条件与输入：订阅已下载为临时 YAML。
- 必须满足的行为：
  - `proxies` 与 `proxy-providers` 同时为非空时，拒绝本次订阅更新，保留上次可用配置。
  - 只有 `proxy-providers` 为非空映射时，进入 provider 整块合并流程。
  - 只有 `proxies` 为非空数组、没有 `proxy-providers` 时，进入 URL provider 流程。
  - 两者都不存在或为空时，保留上次可用配置并返回明确错误。
- 输出或状态变化：选择 provider 合并、proxies provider 或失败回退路径。
- 边界与失败行为：不得静默忽略其中一种来源。

### R-002：provider 整块合并

- 参与者或触发方：`box.tool sub update` 和 `update_mihomo_providers`。
- 前置条件与输入：订阅中存在至少一个带 `http://` 或 `https://` URL 的 provider。
- 必须满足的行为：
  - 读取每个 provider 的完整定义块。
  - 同名 provider 用订阅 block 整体替换现有 block。
  - 不同名 provider 追加到现有 `proxy-providers`。
  - 不删除订阅中未出现的旧 provider。
  - 保留 `health-check`、`override`、`exclude-filter`、`interval` 及其他字段。
  - 只把 `path` 改写为 Box 管理的 `./proxy_provider/<安全文件名>.yaml`。
  - 只接受带 URL 的 provider；无 URL 的 provider 跳过并告警。
- 输出或状态变化：`config.yaml` 的 `proxy-providers` 增加或覆盖 provider block。
- 边界与失败行为：单个 provider 非法时跳过该项，其他 provider 继续，返回部分成功；若全部无法合并，保留旧配置。

### R-003：同名覆盖

- 参与者或触发方：provider 合并流程。
- 前置条件与输入：订阅 provider 名与 `config.yaml` 中已有 provider 同名。
- 必须满足的行为：
  - 用订阅定义替换整个旧 block。
  - 不保留旧 block 的字段。
  - 允许覆盖手动 provider；这是已确认行为，日志必须明确记录被覆盖的名称。
- 输出或状态变化：同名 provider 内容与订阅一致，仅 path 被改写。
- 边界与失败行为：覆盖前保留旧配置备份；不能覆盖其他订阅的 provider 文件路径而不更新路径。

### R-004：proxies-only 订阅

- 参与者或触发方：`box.tool sub update` 的普通 provider 流程。
- 前置条件与输入：订阅只有顶层 `proxies`，没有 `proxy-providers`。
- 必须满足的行为：
  - 继续生成一个 URL provider。
  - provider 名使用订阅名。
  - 默认 health-check 使用：
    - `url: http://www.gstatic.com/generate_204`
    - `timeout: 5000`
    - `interval: 300`
    - `tolerance: 100`
  - 节点内容不写入 `config.yaml`，留在订阅响应和本地 provider 文件。
- 输出或状态变化：`config.yaml` 增加或更新一个 URL provider；`proxies` 内容不复制进 `config.yaml`，留在订阅响应和本地 provider 文件。
- 边界与失败行为：无法解析顶层 proxies 时保留旧配置并报错。

### R-005：URL 和敏感信息

- 参与者或触发方：订阅下载、provider 合并、日志。
- 前置条件与输入：provider URL 可能包含 token。
- 必须满足的行为：
  - 只接受 `http://` 或 `https://` URL。
  - 日志、错误详情和 JSON 输出中的 URL 必须掩码。
  - provider 定义缓存权限为 `0600`。
- 输出或状态变化：配置和缓存包含 URL，但不对用户日志暴露 token。
- 边界与失败行为：非法 URL 不得写入配置。

### R-006：配置完整性

- 参与者或触发方：`update_mihomo_providers`。
- 前置条件与输入：现有 `config.yaml`。
- 必须满足的行为：
  - 只修改 `proxy-providers` 段。
  - 保留 `proxy-groups`、`rules`、`rule-providers`、DNS、端口等其他段。
  - 使用临时文件加原子替换。
  - 配置写入前使用 mihomo 配置检查；检查失败时恢复旧配置。
- 输出或状态变化：主配置其余部分不变。
- 边界与失败行为：检查失败或写入失败时保留旧配置并返回错误。

### 相关约束

- 不把完整 `proxies` 列表写入 `config.yaml`。
- 不导入来源的 `proxy-groups`、`rules`。
- 不实现 provider 删除/收缩清理。
- 订阅同时有 `proxies` 和 `proxy-providers` 时拒绝，不能猜。
- 不修改 sing-box 流程。

## 5. 关键决定

### D-001：整块合并，不做字段级合并

- 状态：已确认
- 结论：订阅 provider block 原样保留，只改写 path；同名覆盖，不同名追加。
- 理由：订阅作者已经配置好 health-check、override、exclude-filter，逐字段白名单容易丢参数。
- 考虑过的备选：只保留 URL/health-check；字段白名单；完整 provider 定义缓存。
- 未采用原因：字段级方案会继续丢失上游字段，违背“原样合并”的目标。
- 影响范围：R-002、R-003、AC-001。

### D-002：同名provider整块覆盖

- 状态：已确认
- 结论：同名时用订阅 block 替换旧 block，不合并字段。
- 理由：行为可预测，避免新旧字段混合。
- 考虑过的备选：跳过冲突；加前缀；字段级合并。
- 未采用原因：跳过会让订阅更新不完整；加前缀改变名称；字段级合并难以判断优先级。
- 影响范围：R-003、AC-002。

### D-003：proxies-only 使用 URL provider

- 状态：已确认
- 结论：没有 `proxy-providers` 的订阅仍生成 URL provider，不物化 inline。
- 理由：避免节点密码进入 config.yaml，保持现有订阅刷新流程。
- 考虑过的备选：inline provider；file provider。
- 未采用原因：配置膨胀、凭据重复、文件管理复杂。
- 影响范围：R-004、AC-003。

### D-004：默认 health-check 改为 gstatic/5000

- 状态：已确认
- 结论：没有 provider 定义的 URL provider 使用 `http://www.gstatic.com/generate_204` 和 5000ms。
- 理由：`cp.cloudflare.com` 在部分节点上 TLS 失败，1 秒超时过紧。
- 考虑过的备选：保持 cloudflare；只换 URL 不改超时；统一强制 gstatic。
- 未采用原因：保持旧值无法解决测速；只换 URL 仍可能超时；统一强制会覆盖订阅 provider 自带值。
- 影响范围：R-004、AC-003。

### D-005：mixed source 拒绝

- 状态：已确认
- 结论：同时有非空 `proxies` 和非空 `proxy-providers` 时拒绝更新。
- 理由：两种来源的节点集合和更新语义不同，自动选择会静默丢失数据。
- 考虑过的备选：providers 优先；proxies 优先；两者都导入。
- 未采用原因：都会产生重复或丢失，且用户明确不支持同时混合。
- 影响范围：R-001、AC-004。

### D-006：只合并不删除

- 状态：已确认
- 结论：订阅未出现的 provider 不自动删除；provider 收缩清理不在本次范围。
- 理由：避免更新时意外删除手动 provider 或仍被其他配置引用的旧 provider。
- 考虑过的备选：按订阅归属删除；全量替换。
- 未采用原因：需要额外的 provider 归属元数据和引用检查，超出当前范围。
- 影响范围：R-002、AC-005。

## 6. 验收标准

### AC-001：合并完整 provider block

- 对应规则：R-001、R-002
- 前置条件：fixture 含 `proxy-providers.xusx`，包含 URL、health-check、override、exclude-filter、path。
- 操作或输入：执行 `box.tool sub update`。
- 预期结果：`config.yaml` 的 `xusx` 保留全部字段；只有 path 变为 `./proxy_provider/xusx.yaml`；其他配置段不变。
- 验证状态：通过
- 验证证据：test/module-webui.test.sh 聚合 provider 字段与 path 断言通过

### AC-002：同名覆盖

- 对应规则：R-003
- 前置条件：`config.yaml` 已有 `xusx`，fixture 中 `xusx` 字段不同。
- 操作或输入：执行 provider 合并。
- 预期结果：旧 `xusx` 整个 block 被订阅 block 替换；不保留旧 health-check/override；其他 provider 不变。
- 验证状态：通过
- 验证证据：test/module-webui.test.sh 同名 provider 覆盖断言通过

### AC-003：proxies-only 生成 URL provider

- 对应规则：R-004
- 前置条件：fixture 只有顶层 `proxies`。
- 操作或输入：执行 `box.tool sub update`。
- 预期结果：生成一个 URL provider；health-check URL 为 gstatic，timeout 为 5000；节点不写入 config.yaml。
- 验证状态：通过
- 验证证据：test/module-webui.test.sh proxies-only 默认 health-check 断言通过

### AC-004：mixed source 拒绝

- 对应规则：R-001、D-005
- 前置条件：fixture 同时有非空 `proxies` 和非空 `proxy-providers`。
- 操作或输入：执行订阅更新。
- 预期结果：更新失败并返回明确错误；旧 provider 和 config 不变。
- 验证状态：通过
- 验证证据：test/module-webui.test.sh mixed source 拒绝与旧配置保留断言通过

### AC-005：不删除旧 provider

- 对应规则：R-002、D-006
- 前置条件：config 有 `mano`，订阅只包含 `xusx` 和 `imagic`。
- 操作或输入：执行 provider 合并。
- 预期结果：`mano` 仍存在；`xusx`、`imagic` 新增或覆盖；不删除 `mano`。
- 验证状态：通过
- 验证证据：test/module-webui.test.sh 手动 provider 保留断言通过

### AC-006：配置完整性

- 对应规则：R-006
- 前置条件：config 含 proxy-groups、rules、rule-providers、DNS、端口。
- 操作或输入：执行 provider 合并。
- 预期结果：只有 proxy-providers 段变化；其他段内容、顺序和注释保持不变；mihomo 配置检查通过。
- 验证状态：未执行
- 验证证据：尚未执行

## 7. 未决问题与假设

### Q-001：provider 列表收缩后的清理

- 类型：未决问题
- 内容：订阅删掉 provider 后，旧 provider 和本地文件是否清理？
- 影响范围：R-002、D-006；AC-005
- 是否阻塞：否，本次明确为只合并不删除。
- 决定或验证方式：后续单独确认归属和引用检查方案。

### Q-002：非 URL provider

- 类型：未决问题
- 内容：`type: file`、inline provider 或无 URL provider 如何处理？
- 影响范围：R-002、AC-001
- 是否阻塞：否；本次只支持 URL provider，跳过并告警。
- 决定或验证方式：后续按实际订阅格式扩展。

### Q-003：混合来源

- 类型：未决问题
- 内容：同时有 `proxies` 和 `proxy-providers` 的订阅是否要支持拆分导入？
- 影响范围：R-001、AC-004
- 是否阻塞：否；本次明确拒绝混合来源，不实现；若未来需要，另立规格。
- 决定或验证方式：用户明确要求单一路径；若未来需要，另立规格。

## 8. 技术设计

### 8.1 数据流

```text
box.tool sub update
  → 下载订阅到临时 YAML
  → 识别来源：
      proxy-providers 非空且 proxies 为空
        → 提取每个 provider 完整 block
        → 写 provider 定义缓存
        → 更新本地 provider 文件
      proxies 非空且 proxy-providers 为空
        → 现有 URL provider 流程
      两者同时存在
        → 拒绝更新
  → update_mihomo_providers
      → 读取现有 config 的 proxy-providers
      → 按 provider 名称覆盖/追加
      → 只改写 path
      → 原子写回并检查
```

### 8.2 provider 定义缓存

现有 `run/subscriptions.providers` 只保存 `source|provider|url|local_path`，无法承载完整 block。新增内部缓存：

```text
run/state/provider-defs/<safe-source>.yaml
```

内容为规范化后的 `proxy-providers` block，保留 URL provider 的字段；缓存权限 `0600`，写临时文件后原子替换。旧订阅没有缓存时，继续按现有 URL provider 逻辑处理，直到下次更新生成缓存。

订阅更新失败或来源歧义时，来源名写入 `run/subscriptions.failed`。`update_mihomo_providers` 跳过失败来源，保留其基线中的旧 provider；更新结束后删除该临时文件。

### 8.3 合并算法

1. 读取现有 `config.yaml` 的 `proxy-providers`。
2. 对来源 block 中每个 provider：
   - 如果同名，替换整个 block；
   - 如果不同名，追加；
   - 将 `path` 改写为 `./proxy_provider/<safe>.yaml`；
   - 其他字段原样保留。
3. 不删除现有 provider。
4. 写临时文件，执行 mihomo 配置检查。
5. 检查通过后原子替换；失败恢复旧文件。

### 8.4 默认 health-check

只用于“没有 provider 定义的 proxies-only 订阅”或没有 health-check 的生成 URL provider：

```yaml
health-check:
  enable: true
  url: http://www.gstatic.com/generate_204
  interval: 300
  timeout: 5000
  tolerance: 100
```

从订阅复制的 provider block 不注入或改写 health-check；订阅有就原样保留，没有就保持没有。

### 8.5 proxies-only 的明确产物

订阅只有顶层 `proxies` 时，`config.yaml` 里生成的是 URL provider，不是把 `proxies` 列表展开：

```yaml
proxy-providers:
  bkm:
    type: http
    url: "<订阅 URL，日志中掩码>"
    path: ./proxy_provider/bkm.yaml
    interval: 86400
    health-check:
      enable: true
      url: http://www.gstatic.com/generate_204
      interval: 300
      timeout: 5000
      tolerance: 100
```

下载后的 provider 文件内容才是：

```yaml
proxies:
  - name: 香港 01
    type: trojan
    ...
```

因此：

- `proxies` 不写入 `config.yaml`
- `proxies` 不生成 inline provider
- 订阅更新时由 URL provider 重新读取
- `proxy-providers` 与 `proxies` 同时非空时，不进入这条路径，而是拒绝更新

### 8.6 测试

- 完整 provider block 合并、字段保留、path 改写。
- 同名整块覆盖。
- proxies-only 默认 health-check。
- mixed source 拒绝。
- 旧 provider 保留。
- groups/rules/DNS 不变。
- 无效 URL / 非 URL provider 跳过并部分成功。
