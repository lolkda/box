export const userAgentPresets = [
  "clash-verge/v2.4.5",
  "clash.meta/1.19.20",
  "Clash",
  "mihomo/1.19.20",
  "ClashforWindows/0.20.39",
];

export function isValidUserAgent(value: string, allowEmpty = false): boolean {
  if (!value.trim()) return allowEmpty;
  return value.length <= 255 && !/[\r\n\x00-\x1F\x7F]/.test(value);
}
