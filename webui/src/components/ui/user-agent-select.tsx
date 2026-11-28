import { useEffect, useState } from "react";
import { Input } from "./input";
import { Select } from "./select";
import { userAgentPresets } from "../../lib/user-agent";

const USER_AGENT_CUSTOM_VALUE = "__custom__";

interface UserAgentSelectProps {
  id: string;
  value: string;
  onChange: (value: string) => void;
  allowInherit?: boolean;
  disabled?: boolean;
  onCustomChange?: (custom: boolean) => void;
}

/**
 * 预设用原生下拉框选择；只有选择“自定义”时才挂载文本输入框。
 * 不把预设做成 datalist，避免 Android WebView 同时弹出建议层和输入法。
 */
export function UserAgentSelect({
  id,
  value,
  onChange,
  allowInherit = false,
  disabled = false,
  onCustomChange,
}: UserAgentSelectProps) {
  const [customMode, setCustomMode] = useState(() => value !== "" && !userAgentPresets.includes(value));
  const custom = customMode || (value !== "" && !userAgentPresets.includes(value));

  useEffect(() => {
    onCustomChange?.(custom);
  }, [custom, onCustomChange]);

  const options = [
    ...(allowInherit ? [{ value: "", label: "跟随全局" }] : []),
    ...userAgentPresets.map((preset) => ({ value: preset, label: preset })),
    { value: USER_AGENT_CUSTOM_VALUE, label: "自定义…" },
  ];

  const updateMode = (next: boolean) => {
    setCustomMode(next);
    onCustomChange?.(next);
  };

  return (
    <div className="space-y-2">
      <Select
        id={id}
        value={custom ? USER_AGENT_CUSTOM_VALUE : value}
        options={options}
        disabled={disabled}
        onChange={(event) => {
          const next = event.currentTarget.value;
          if (next === USER_AGENT_CUSTOM_VALUE) {
            updateMode(true);
            return;
          }
          updateMode(false);
          onChange(next);
        }}
      />
      {custom ? (
        <Input
          id={`${id}-custom`}
          aria-label="自定义 User-Agent"
          value={value}
          placeholder="输入自定义 User-Agent"
          autoFocus
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          className="font-mono text-xs"
          disabled={disabled}
          onChange={(event) => { const next = event.currentTarget.value; onChange(next); }}
        />
      ) : null}
    </div>
  );
}
