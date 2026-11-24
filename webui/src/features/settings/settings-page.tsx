import { useEffect, useState } from "react";
import { Controller, useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { useBlocker } from "react-router-dom";
import { useQueryClient } from "@tanstack/react-query";
import { Eye, EyeOff, KeyRound, RotateCcw, Save, X } from "lucide-react";
import { api } from "../../api/client";
import { queryKeys, useSettingsQuery } from "../../api/queries";
import { Accordion, AccordionItem } from "../../components/ui/accordion";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { ConfirmDialog } from "../../components/ui/confirm-dialog";
import { ErrorState } from "../../components/ui/error-state";
import { Field } from "../../components/ui/field";
import { Input } from "../../components/ui/input";
import { PageTitle } from "../../components/ui/page-title";
import { PackagePicker } from "../../components/ui/package-picker";
import { Select } from "../../components/ui/select";
import { PageSkeleton } from "../../components/ui/skeleton";
import { Switch } from "../../components/ui/switch";
import { TagInput } from "../../components/ui/tag-input";
import { useToast } from "../../components/ui/toast";
import { allSettingsFields, settingsGroups, type SettingField } from "../../lib/settings";
import { cn, getErrorMessage } from "../../lib/utils";

interface SettingsFormValues {
  settings: Record<string, string>;
  arrays: Record<string, string[]>;
  token: string;
}

const settingsSchema = z.object({
  settings: z.record(z.string()).superRefine((settings, ctx) => {
    for (const field of allSettingsFields) {
      if (field.kind === "array" || field.kind === "package-array" || field.kind === "password") continue;
      const value = settings[field.key] ?? "";
      if (field.kind === "select" && field.options && !field.options.some((option) => option.value === value)) {
        ctx.addIssue({ code: z.ZodIssueCode.custom, path: [field.key], message: `${field.label}的值不合法` });
      }
      if (field.kind === "number" && !/^\d+$/.test(value)) {
        ctx.addIssue({ code: z.ZodIssueCode.custom, path: [field.key], message: `${field.label}必须是数字` });
      }
      if ((field.key === "tproxy_port" || field.key === "redir_port") && /^\d+$/.test(value)) {
        const port = Number(value);
        if (port < 1 || port > 65535) ctx.addIssue({ code: z.ZodIssueCode.custom, path: [field.key], message: "端口必须在 1-65535 之间" });
      }
      if (field.key === "memcg_limit" && !/^\d+[KMGkmg]?$/.test(value)) {
        ctx.addIssue({ code: z.ZodIssueCode.custom, path: [field.key], message: "格式示例：100M、512M" });
      }
      if (field.key === "allow_cpu" && !/^\d+(-\d+)?(,\d+(-\d+)?)*$/.test(value)) {
        ctx.addIssue({ code: z.ZodIssueCode.custom, path: [field.key], message: "格式示例：0-7 或 0,2,4-7" });
      }
      if (field.key === "weight" && value && !/^\d+$/.test(value)) {
        ctx.addIssue({ code: z.ZodIssueCode.custom, path: [field.key], message: "权重必须是数字" });
      }
      if (field.key === "interva_update" && value.trim().split(/\s+/).length !== 5) {
        ctx.addIssue({ code: z.ZodIssueCode.custom, path: [field.key], message: "cron 表达式需要 5 个字段" });
      }
      if (field.key === "subscription_user_agent" && (!value.trim() || value.length > 255 || /[\r\n\x00-\x1F\x7F]/.test(value))) {
        ctx.addIssue({ code: z.ZodIssueCode.custom, path: [field.key], message: "User-Agent 不能为空、不能包含换行或控制字符，最多 255 个字符" });
      }
      if ((field.key === "url_ghproxy" || field.key === "ruleset_prefix" || field.key === "cn_ip_url" || field.key === "cn_ipv6_url") && value && !/^https?:\/\//i.test(value)) {
        ctx.addIssue({ code: z.ZodIssueCode.custom, path: [field.key], message: `${field.label}必须是 http(s) URL` });
      }
    }
  }),
  arrays: z.record(z.array(z.string())).superRefine((arrays, ctx) => {
    for (const field of allSettingsFields) {
      if (field.kind !== "array") continue;
      const values = arrays[field.key] || [];
      values.forEach((value, index) => {
        if (field.key === "force_proxy_cidrs" && !/^(\d{1,3}\.){3}\d{1,3}\/\d{1,2}$/.test(value)) {
          ctx.addIssue({ code: z.ZodIssueCode.custom, path: [field.key, index], message: `CIDR 格式不合法: ${value}` });
        }
        if (field.key === "force_proxy_cidrs6" && !value.includes(":")) {
          ctx.addIssue({ code: z.ZodIssueCode.custom, path: [field.key, index], message: `IPv6 网段不合法: ${value}` });
        }
        if ((field.key === "macs_list" || field.key === "wifi_bssids_list") && !/^([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}$/.test(value)) {
          ctx.addIssue({ code: z.ZodIssueCode.custom, path: [field.key, index], message: `MAC 地址格式不合法: ${value}` });
        }
      });
    }
  }),
  token: z.string(),
});

function buildValues(data: Awaited<ReturnType<typeof api.settings>>): SettingsFormValues {
  const settings: Record<string, string> = {};
  const arrays: Record<string, string[]> = {};
  for (const [key, value] of Object.entries(data)) {
    if (key === "sensitive" || typeof value !== "string" && !Array.isArray(value)) continue;
    if (Array.isArray(value)) arrays[key] = [...value];
    else settings[key] = value;
  }
  return { settings, arrays, token: "" };
}

function errorFor(errors: Record<string, unknown> | undefined, key: string): string | undefined {
  const value = errors?.[key];
  if (value && typeof value === "object" && "message" in value) return String((value as { message?: unknown }).message);
  return undefined;
}

function SettingsFieldRow({
  field,
  control,
  register,
  error,
  tokenSet,
  tokenPreview,
  onPickPackages,
}: {
  field: SettingField;
  control: ReturnType<typeof useForm<SettingsFormValues>>["control"];
  register: ReturnType<typeof useForm<SettingsFormValues>>["register"];
  error?: string;
  tokenSet?: boolean;
  tokenPreview?: string;
  onPickPackages?: () => void;
}) {
  const [showToken, setShowToken] = useState(false);

  if (field.kind === "switch") {
    return (
      <Controller
        control={control}
        name={`settings.${field.key}`}
        render={({ field: controlled }) => (
          <div className="flex min-h-11 items-center justify-between gap-3 py-1">
            <div className="min-w-0">
              <p className="text-sm text-fg">{field.label}</p>
              {field.hint ? <p className="mt-0.5 text-[11px] leading-4 text-fg-subtle">{field.hint}</p> : null}
            </div>
            <Switch checked={controlled.value === "true"} onCheckedChange={(checked) => controlled.onChange(checked ? "true" : "false")} />
          </div>
        )}
      />
    );
  }

  if (field.kind === "array" || field.kind === "package-array") {
    return (
      <Field label={field.label} htmlFor={`setting-${field.key}`} hint={field.hint} error={error}>
        <Controller
          control={control}
          name={`arrays.${field.key}`}
          render={({ field: controlled }) => (
            <TagInput
              id={`setting-${field.key}`}
              values={controlled.value || []}
              onChange={controlled.onChange}
              placeholder={field.placeholder}
              onPickPackages={field.kind === "package-array" ? onPickPackages : undefined}
            />
          )}
        />
      </Field>
    );
  }

  if (field.kind === "select") {
    return (
      <Field label={field.label} htmlFor={`setting-${field.key}`} hint={field.hint} error={error}>
        <Controller
          control={control}
          name={`settings.${field.key}`}
          render={({ field: controlled }) => (
            <Select
              id={`setting-${field.key}`}
              value={controlled.value || ""}
              options={field.options || []}
              onChange={(event) => { const value = event.currentTarget.value; controlled.onChange(value); }}
            />
          )}
        />
      </Field>
    );
  }

  if (field.kind === "password") {
    return (
      <Field label={field.label} htmlFor={`setting-${field.key}`} hint={field.hint} error={error}>
        <Controller
          control={control}
          name="token"
          render={({ field: controlled }) => (
            <div className="space-y-2">
              <div className="flex gap-2">
                <div className="relative flex-1">
                  <Input
                    id="setting-githubtoken"
                    type={showToken ? "text" : "password"}
                    value={controlled.value}
                    placeholder={tokenSet ? `已设置（${tokenPreview}），留空保持不变` : "未设置"}
                    autoCapitalize="none"
                    autoCorrect="off"
                    spellCheck={false}
                    className="pr-10 font-mono text-xs"
                    onChange={(event) => { const value = event.currentTarget.value; controlled.onChange(value); }}
                  />
                  <button
                    type="button"
                    onClick={() => setShowToken((current) => !current)}
                    className="absolute right-2 top-1/2 -translate-y-1/2 rounded-md p-1 text-fg-muted hover:bg-surface-muted"
                    aria-label={showToken ? "隐藏 Token" : "显示 Token"}
                  >
                    {showToken ? <EyeOff size={16} /> : <Eye size={16} />}
                  </button>
                </div>
                <Button
                  variant="danger-outline"
                  size="icon"
                  aria-label="清除 Token"
                  onClick={() => controlled.onChange("")}
                >
                  <X size={16} />
                </Button>
              </div>
              <div className="flex items-center gap-2 text-[11px] text-fg-muted">
                <KeyRound size={13} />
                <span>{tokenSet ? "GitHub Token 已设置" : "GitHub Token 未设置"}</span>
                <Badge tone={tokenSet ? "success" : "neutral"}>{tokenSet ? "已配置" : "未配置"}</Badge>
              </div>
            </div>
          )}
        />
      </Field>
    );
  }

  const presetListId = field.presets?.length ? `setting-${field.key}-presets` : undefined;
  return (
    <Field label={field.label} htmlFor={`setting-${field.key}`} hint={field.hint} error={error}>
      {presetListId ? (
        <datalist id={presetListId}>
          {field.presets?.map((preset) => <option value={preset} key={preset} />)}
        </datalist>
      ) : null}
      <Input
        id={`setting-${field.key}`}
        type="text"
        list={presetListId}
        inputMode={field.inputMode || (field.kind === "number" ? "numeric" : "text")}
        placeholder={field.placeholder}
        className={field.key === "interva_update" || field.key === "allow_cpu" || field.key === "url_ghproxy" || field.key === "ruleset_prefix" || field.key === "subscription_user_agent" ? "font-mono text-xs" : undefined}
        {...register(`settings.${field.key}`)}
      />
    </Field>
  );
}

export function SettingsPage() {
  const settingsQuery = useSettingsQuery();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [saving, setSaving] = useState(false);
  const [packagePickerFor, setPackagePickerFor] = useState<string | null>(null);
  const [tokenSet, setTokenSet] = useState(false);
  const [tokenPreview, setTokenPreview] = useState("");

  const form = useForm<SettingsFormValues>({
    resolver: zodResolver(settingsSchema),
    defaultValues: { settings: {}, arrays: {}, token: "" },
    mode: "onBlur",
  });
  const { control, register, handleSubmit, reset, formState, setValue, getValues, watch } = form;
  const dirty = formState.isDirty;
  const blocker = useBlocker(dirty || saving);

  useEffect(() => {
    if (!settingsQuery.data) return;
    reset(buildValues(settingsQuery.data));
    setTokenSet(settingsQuery.data.sensitive.githubtoken.set);
    setTokenPreview(settingsQuery.data.sensitive.githubtoken.preview);
  }, [settingsQuery.data, reset]);

  useEffect(() => {
    const handler = (event: BeforeUnloadEvent) => {
      if (!dirty) return;
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", handler);
    return () => window.removeEventListener("beforeunload", handler);
  }, [dirty]);

  const formSettings = watch("settings") || {};
  const settingsErrors = (formState.errors.settings || {}) as Record<string, unknown>;
  const arrayErrors = (formState.errors.arrays || {}) as Record<string, unknown>;

  const onSubmit = async (values: SettingsFormValues) => {
    setSaving(true);
    try {
      const token = formState.dirtyFields.token ? values.token : undefined;
      await api.applySettings(values.settings, values.arrays, token);
      toast({ title: "设置已保存", tone: "success" });
      await queryClient.invalidateQueries({ queryKey: queryKeys.settings });
      await queryClient.invalidateQueries({ queryKey: queryKeys.status });
      reset(values);
    } catch (error) {
      toast({ title: "保存设置失败", description: getErrorMessage(error), tone: "error" });
    } finally {
      setSaving(false);
    }
  };

  const tokenDirty = Boolean(formState.dirtyFields.token);
  const tokenSetValue = tokenDirty ? Boolean(getValues("token")) : tokenSet;

  return (
    <div className={cn("space-y-4", dirty && "pb-20")}>
      <PageTitle
        title="设置"
        description="全部可编辑设置项；保存后写入 settings.json"
        action={
          <Badge tone={dirty ? "warning" : "neutral"}>{dirty ? "未保存" : "已同步"}</Badge>
        }
      />

      {settingsQuery.isLoading ? (
        <PageSkeleton />
      ) : settingsQuery.error ? (
        <ErrorState error={settingsQuery.error} onRetry={() => void settingsQuery.refetch()} />
      ) : !settingsQuery.data ? (
        <PageSkeleton />
      ) : (
        <form onSubmit={handleSubmit(onSubmit)} className="space-y-3">
          <Accordion defaultValue={["basic", "network"]}>
            {settingsGroups.map((group) => (
              <AccordionItem key={group.id} value={group.id} title={group.title} description={group.description}>
                <div className="space-y-4">
                  {group.fields.filter((field) => !field.showWhen || formSettings[field.showWhen.key] === field.showWhen.equals).map((field) => (
                    <SettingsFieldRow
                      key={field.key}
                      field={field}
                      control={control}
                      register={register}
                      error={errorFor(settingsErrors, field.key) || errorFor(arrayErrors, field.key)}
                      tokenSet={tokenSetValue}
                      tokenPreview={tokenPreview}
                      onPickPackages={() => setPackagePickerFor(field.key)}
                    />
                  ))}
                </div>
              </AccordionItem>
            ))}
          </Accordion>

          <div className="rounded-2xl border border-border bg-surface p-4 text-[11px] leading-5 text-fg-muted">
            <p className="font-medium text-fg">内部路径与常量</p>
            <p className="mt-1">box_dir、内部路径、config 路径、bin_list 等内部键不在页面开放，避免误改导致模块无法启动。</p>
          </div>

          {dirty ? (
            <div className="fixed inset-x-0 bottom-[calc(4.25rem+var(--safe-bottom))] z-30 border-t border-border bg-surface/95 px-4 py-3 shadow-[0_-8px_24px_rgba(0,0,0,0.08)] backdrop-blur-xl">
              <div className="mx-auto flex max-w-3xl items-center justify-between gap-3">
                <p className="text-xs text-fg-muted">有未保存的设置修改</p>
                <div className="flex gap-2">
                  <Button variant="ghost" size="sm" icon={<RotateCcw size={14} />} onClick={() => { reset(); }}>
                    撤销
                  </Button>
                  <Button variant="primary" size="sm" type="submit" loading={saving} icon={<Save size={14} />}>
                    保存更改
                  </Button>
                </div>
              </div>
            </div>
          ) : null}
        </form>
      )}

      <PackagePicker
        open={packagePickerFor !== null}
        onOpenChange={(open) => { if (!open) setPackagePickerFor(null); }}
        values={packagePickerFor ? getValues(`arrays.${packagePickerFor}`) || [] : []}
        onChange={(values) => {
          if (!packagePickerFor) return;
          setValue(`arrays.${packagePickerFor}`, values, { shouldDirty: true });
          setPackagePickerFor(null);
        }}
      />

      <ConfirmDialog
        open={blocker.state === "blocked"}
        onOpenChange={(open) => { if (!open) blocker.reset?.(); }}
        title="放弃未保存的修改？"
        description="离开设置页会丢失当前修改。"
        confirmLabel="放弃修改"
        destructive
        onConfirm={() => blocker.proceed?.()}
      />
    </div>
  );
}
