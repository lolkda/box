import { useState, type KeyboardEvent } from "react";
import { PackageSearch, Plus, X } from "lucide-react";
import { Button } from "./button";
import { Input } from "./input";
import { uniquePush } from "../../lib/utils";

export function TagInput({
  values,
  onChange,
  placeholder = "输入后按回车添加",
  addLabel = "添加",
  onPickPackages,
  mono = false,
  id,
}: {
  values: string[];
  onChange: (values: string[]) => void;
  placeholder?: string;
  addLabel?: string;
  onPickPackages?: () => void;
  mono?: boolean;
  id?: string;
}) {
  const [draft, setDraft] = useState("");

  const add = () => {
    const next = uniquePush(values, draft);
    if (next !== values) onChange(next);
    setDraft("");
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      event.preventDefault();
      add();
    }
    if (event.key === "Backspace" && !draft && values.length) {
      onChange(values.slice(0, -1));
    }
  };

  return (
    <div className="space-y-2">
      <div className="flex gap-2">
        <Input
          id={id}
          name={id}
          value={draft}
          placeholder={placeholder}
          aria-label={id ? undefined : placeholder}
          onChange={(event) => setDraft(event.currentTarget.value)}
          onKeyDown={onKeyDown}
          className={mono ? "font-mono text-xs" : undefined}
        />
        <Button size="icon" variant="secondary" onClick={add} aria-label={addLabel} disabled={!draft.trim()}>
          <Plus size={17} />
        </Button>
        {onPickPackages ? (
          <Button size="icon" variant="secondary" onClick={onPickPackages} aria-label="选择已安装应用">
            <PackageSearch size={17} />
          </Button>
        ) : null}
      </div>
      {values.length ? (
        <div className="flex flex-wrap gap-1.5">
          {values.map((value) => (
            <span
              key={value}
              className={`inline-flex max-w-full items-center gap-1 rounded-lg border border-border bg-surface-muted px-2 py-1 text-[11px] text-fg ${
                mono ? "font-mono" : ""
              }`}
            >
              <span className="truncate">{value}</span>
              <button
                type="button"
                onClick={() => onChange(values.filter((item) => item !== value))}
                className="rounded-full p-0.5 text-fg-muted hover:bg-border hover:text-fg"
                aria-label={`移除 ${value}`}
              >
                <X size={12} />
              </button>
            </span>
          ))}
        </div>
      ) : (
        <p className="text-[11px] text-fg-subtle">暂无条目</p>
      )}
    </div>
  );
}
