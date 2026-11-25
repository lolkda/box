export type SettingFieldKind = "switch" | "select" | "user-agent" | "text" | "number" | "password" | "array" | "package-array";

export interface SettingOption {
  value: string;
  label: string;
}

export interface SettingField {
  key: string;
  label: string;
  kind: SettingFieldKind;
  options?: SettingOption[];
  placeholder?: string;
  hint?: string;
  sensitive?: boolean;
  inputMode?: "text" | "numeric" | "url";
  showWhen?: { key: string; equals: string };
}

export interface SettingsGroup {
  id: string;
  title: string;
  description: string;
  fields: SettingField[];
}

export const settingsGroups: SettingsGroup[] = [
  {
    id: "basic",
    title: "基础",
    description: "启动、核心与自动更新",
    fields: [
      { key: "boot_auto_start", label: "开机自启", kind: "switch" },
      { key: "auto_modify_config", label: "自动维护配置", kind: "switch", hint: "更新订阅时自动重写 proxy-providers 与 rule-providers" },
      { key: "update_subscription", label: "自动更新订阅", kind: "switch" },
      { key: "update_geo", label: "自动更新 Geo", kind: "switch" },
      { key: "run_crontab", label: "启用定时任务", kind: "switch" },
      { key: "interva_update", label: "定时表达式", kind: "text", placeholder: "0 0,6,12,18 * * *", hint: "标准 5 字段 cron 表达式" },
      { key: "subscription_keep_last_good", label: "保留上次可用订阅", kind: "switch", hint: "更新失败时保留上一个可用 provider 文件" },
      { key: "subscription_user_agent", label: "节点订阅 User-Agent", kind: "user-agent", hint: "仅影响 mihomo 节点订阅和 provider 下载；修改后下次更新订阅生效" },
    ],
  },
  {
    id: "network",
    title: "网络",
    description: "网络模式、端口与 DNS",
    fields: [
      { key: "network_mode", label: "网络模式", kind: "select", options: [
        { value: "tun", label: "TUN" },
        { value: "tproxy", label: "TProxy" },
        { value: "redirect", label: "Redirect" },
        { value: "mixed", label: "Mixed" },
        { value: "enhance", label: "Enhance" },
      ] },
      { key: "tproxy_port", label: "TProxy 端口", kind: "number", inputMode: "numeric", placeholder: "9898" },
      { key: "redir_port", label: "Redirect 端口", kind: "number", inputMode: "numeric", placeholder: "9797" },
      { key: "ipv6", label: "IPv6", kind: "switch" },
      { key: "mihomo_dns_forward", label: "DNS 转发", kind: "select", options: [
        { value: "enable", label: "开启" },
        { value: "disable", label: "关闭" },
      ] },
      { key: "quic", label: "QUIC", kind: "select", options: [
        { value: "enable", label: "开启" },
        { value: "disable", label: "关闭" },
      ] },
      { key: "dns_hijack_mode", label: "DNS 劫持", kind: "select", options: [
        { value: "disable", label: "关闭" },
        { value: "tproxy", label: "TProxy" },
        { value: "redirect", label: "Redirect" },
      ] },
    ],
  },
  {
    id: "proxy",
    title: "代理",
    description: "流量范围、CNIP 与例外",
    fields: [
      { key: "proxy_mode", label: "代理模式", kind: "select", options: [
        { value: "core", label: "核心模式" },
        { value: "blacklist", label: "黑名单" },
        { value: "whitelist", label: "白名单" },
      ] },
      { key: "proxy_tcp", label: "代理 TCP", kind: "switch" },
      { key: "proxy_udp", label: "代理 UDP", kind: "switch" },
      { key: "dns_hijack_tcp", label: "劫持 TCP DNS", kind: "switch" },
      { key: "dns_hijack_udp", label: "劫持 UDP DNS", kind: "switch" },
      { key: "bypass_cn_ip", label: "绕过大陆 IP", kind: "switch", hint: "需要设备支持 ipset" },
      { key: "bypass_cn_ip_v4", label: "绕过 IPv4 大陆 IP", kind: "switch" },
      { key: "bypass_cn_ip_v6", label: "绕过 IPv6 大陆 IP", kind: "switch" },
      { key: "cnip_mode", label: "CNIP 匹配", kind: "select", options: [
        { value: "ipset", label: "ipset" },
      ] },
      { key: "cn_ip_url", label: "CNIP IPv4 源", kind: "text", inputMode: "url", placeholder: "https://ispip.clang.cn/all_cn.txt" },
      { key: "cn_ipv6_url", label: "CNIP IPv6 源", kind: "text", inputMode: "url", placeholder: "https://ispip.clang.cn/all_cn_ipv6.txt" },
      { key: "cnip_force_proxy_apps", label: "强制代理应用", kind: "package-array", hint: "可填写包名，或从已安装应用中选择；支持 用户ID:包名" },
      { key: "force_proxy_cidrs", label: "强制代理 IPv4 网段", kind: "array", placeholder: "192.168.1.0/24", hint: "CIDR 格式，例如 192.168.1.0/24" },
      { key: "force_proxy_cidrs6", label: "强制代理 IPv6 网段", kind: "array", placeholder: "fd00::/8" },
    ],
  },
  {
    id: "network-control",
    title: "热点与网络控制",
    description: "Wi-Fi 匹配、MAC 过滤与网络状态联动",
    fields: [
      { key: "enable_network_service_control", label: "网络状态控制", kind: "switch" },
      { key: "use_module_on_wifi", label: "Wi-Fi 连接时启用", kind: "switch" },
      { key: "use_module_on_wifi_disconnect", label: "Wi-Fi 断开时启用", kind: "switch" },
      { key: "use_ssid_matching", label: "启用 SSID/BSSID 匹配", kind: "switch" },
      { key: "use_wifi_list_mode", label: "Wi-Fi 匹配模式", kind: "select", options: [
        { value: "blacklist", label: "黑名单" },
        { value: "whitelist", label: "白名单" },
      ] },
      { key: "wifi_ssids_list", label: "Wi-Fi SSID 列表", kind: "array", placeholder: "My Wi-Fi" },
      { key: "wifi_bssids_list", label: "Wi-Fi BSSID 列表", kind: "array", placeholder: "AA:BB:CC:DD:EE:FF" },
      { key: "mac_filter", label: "热点 MAC 过滤", kind: "switch" },
      { key: "mac_mode", label: "MAC 模式", kind: "select", options: [
        { value: "blacklist", label: "黑名单" },
        { value: "whitelist", label: "白名单" },
      ] },
      { key: "macs_list", label: "MAC 列表", kind: "array", placeholder: "AA:BB:CC:DD:EE:FF" },
      { key: "inotify_log_enabled", label: "网络切换调试日志", kind: "switch" },
    ],
  },
  {
    id: "performance",
    title: "性能",
    description: "cgroup、CPU 与内存限制",
    fields: [
      { key: "performance_mode", label: "性能模式", kind: "switch", hint: "兼容性较低，出现异常时关闭" },
      { key: "cgroup_memcg", label: "启用内存限制", kind: "switch" },
      { key: "memcg_limit", label: "内存上限", kind: "text", placeholder: "100M", hint: "例如 100M、512M" },
      { key: "taskset_cpu", label: "绑定 CPU 核心", kind: "switch" },
      { key: "allow_cpu", label: "允许核心", kind: "text", placeholder: "0-7", hint: "例如 0-3 或 0,2,4-7" },
      { key: "cgroup_blkio", label: "启用磁盘 I/O 权重", kind: "switch" },
      { key: "weight", label: "I/O 权重", kind: "text", placeholder: "900", hint: "留空使用默认值" },
    ],
  },
  {
    id: "updates",
    title: "更新与下载",
    description: "GitHub 镜像、规则集与核心通道",
    fields: [
      { key: "use_ghproxy", label: "GitHub 镜像", kind: "switch" },
      { key: "url_ghproxy", label: "镜像地址", kind: "text", inputMode: "url", placeholder: "https://ghfast.top" },
      { key: "ruleset_prefix", label: "规则集前缀", kind: "text", inputMode: "url", placeholder: "留空使用官方地址" },
      { key: "mihomo_stable", label: "mihomo 通道", kind: "select", options: [
        { value: "enable", label: "稳定版" },
        { value: "disable", label: "预发行版" },
      ] },
      { key: "githubtoken", label: "GitHub Token", kind: "password", sensitive: true, hint: "只显示设置状态，不会回填明文" },
    ],
  },
];

export const allSettingsFields = settingsGroups.flatMap((group) => group.fields);
