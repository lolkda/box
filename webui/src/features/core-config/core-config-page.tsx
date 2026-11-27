import { useEffect, useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Code2, RefreshCw, Save, ShieldAlert, SlidersHorizontal } from "lucide-react";
import { api } from "../../api/client";
import { queryKeys, useCoreConfigQuery, useStatusQuery, useTaskStart } from "../../api/queries";
import type { MihomoStructuredConfig } from "../../api/types";
import { StickyActionBar } from "../../components/layout/sticky-action-bar";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { Card, CardBody, CardHeader } from "../../components/ui/card";
import { EmptyState } from "../../components/ui/empty-state";
import { ErrorState } from "../../components/ui/error-state";
import { Field } from "../../components/ui/field";
import { Input, Textarea } from "../../components/ui/input";
import { PageTitle } from "../../components/ui/page-title";
import { Select } from "../../components/ui/select";
import { PageSkeleton } from "../../components/ui/skeleton";
import { Switch } from "../../components/ui/switch";
import { useToast } from "../../components/ui/toast";
import { cn, getErrorMessage } from "../../lib/utils";

const structuredFields: Array<{ key: keyof MihomoStructuredConfig; label: string; hint?: string }> = [
  { key: "mode", label: "运行模式" },
  { key: "log-level", label: "日志级别" },
  { key: "allow-lan", label: "允许局域网连接" },
  { key: "external-controller", label: "外部控制地址", hint: "例如 0.0.0.0:9090" },
  { key: "external-ui", label: "外部 UI 路径", hint: "例如 ./dashboard" },
  { key: "profile.store-selected", label: "记住选中节点" },
  { key: "profile.store-fake-ip", label: "缓存 Fake-IP" },
  { key: "unified-delay", label: "统一延迟测试" },
  { key: "tcp-concurrent", label: "TCP 并发" },
  { key: "find-process-mode", label: "进程匹配模式" },
];

const falseKeys = new Set(["allow-lan", "profile.store-selected", "profile.store-fake-ip", "unified-delay", "tcp-concurrent"]);

export function CoreConfigPage() {
  const statusQuery = useStatusQuery();
  const core = statusQuery.data?.core;
  const coreQuery = useCoreConfigQuery(core === "mihomo");
  const startTask = useTaskStart();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [tab, setTab] = useState<"structured" | "raw">("structured");
  const [form, setForm] = useState<MihomoStructuredConfig | null>(null);
  const [raw, setRaw] = useState("");
  const [saving, setSaving] = useState(false);
  const [updatingDashboard, setUpdatingDashboard] = useState(false);

  useEffect(() => {
    if (!coreQuery.data) return;
    setForm({ ...coreQuery.data.structured });
    setRaw(coreQuery.data.raw);
  }, [coreQuery.data]);

  const original = coreQuery.data;
  const dirty = Boolean(original && form && (JSON.stringify(form) !== JSON.stringify(original.structured) || (tab === "raw" && raw !== original.raw)));

  const updateField = <K extends keyof MihomoStructuredConfig>(key: K, value: MihomoStructuredConfig[K]) => {
    setForm((current) => (current ? { ...current, [key]: value } : current));
  };

  const save = async () => {
    if (!form) return;
    setSaving(true);
    try {
      if (tab === "raw") {
        if (!raw.trim()) {
          toast({ title: "YAML 不能为空", tone: "error" });
          return;
        }
        await api.applyCoreConfig({ raw });
      } else {
        await api.applyCoreConfig({ structured: form });
      }
      toast({ title: tab === "raw" ? "mihomo YAML 已保存" : "mihomo 配置已保存", tone: "success" });
      await queryClient.invalidateQueries({ queryKey: queryKeys.coreConfig });
      await queryClient.invalidateQueries({ queryKey: queryKeys.status });
    } catch (error) {
      toast({ title: "保存核心配置失败", description: getErrorMessage(error), tone: "error" });
    } finally {
      setSaving(false);
    }
  };

  const updateDashboard = async () => {
    setUpdatingDashboard(true);
    try {
      await startTask("xui");
      toast({ title: "已开始更新面板", description: "任务完成后刷新 /ui 查看 zashboard", tone: "info" });
    } catch (error) {
      toast({ title: "启动面板更新失败", description: getErrorMessage(error), tone: "error" });
    } finally {
      setUpdatingDashboard(false);
    }
  };

  const rawDirty = Boolean(original && raw !== original.raw);
  const structuredDirty = Boolean(original && form && JSON.stringify(form) !== JSON.stringify(original.structured));
  const canSave = tab === "raw" ? rawDirty : structuredDirty;

  const title = useMemo(() => (core === "mihomo" ? "mihomo 核心配置" : "核心配置"), [core]);

  if (statusQuery.isLoading) return <PageSkeleton />;
  if (core !== "mihomo") {
    return (
      <div className="space-y-4">
        <PageTitle title="核心配置" description="仅 mihomo 提供结构化配置和安全运行项" />
        <EmptyState
          title="当前核心不支持核心配置"
          description={`当前运行核心为 ${core || "未知"}。请切换到 mihomo 后再编辑 config.yaml。`}
          icon={<ShieldAlert size={22} />}
        />
      </div>
    );
  }

  if (coreQuery.isLoading) return <PageSkeleton />;
  if (coreQuery.error || !coreQuery.data || !form) {
    return <ErrorState error={coreQuery.error || new Error("配置未加载")} onRetry={() => void coreQuery.refetch()} />;
  }

  return (
    <div className="space-y-4 pb-8">
      <PageTitle
        title={title}
        description={coreQuery.data.file}
        action={
          <>
            <Button variant="secondary" size="sm" onClick={() => void updateDashboard()} loading={updatingDashboard} icon={<RefreshCw size={15} />}>
              更新面板
            </Button>
            <Badge tone={dirty ? "warning" : "neutral"}>{dirty ? "未保存" : "已同步"}</Badge>
          </>
        }
      />

      <div className="grid grid-cols-2 rounded-xl bg-surface-muted p-1">
        <button
          type="button"
          onClick={() => setTab("structured")}
          className={cn("flex h-9 items-center justify-center gap-1.5 rounded-lg text-xs font-medium transition-colors", tab === "structured" ? "bg-surface text-fg shadow-sm" : "text-fg-muted")}
        >
          <SlidersHorizontal size={14} /> 常用项
        </button>
        <button
          type="button"
          onClick={() => setTab("raw")}
          className={cn("flex h-9 items-center justify-center gap-1.5 rounded-lg text-xs font-medium transition-colors", tab === "raw" ? "bg-surface text-fg shadow-sm" : "text-fg-muted")}
        >
          <Code2 size={14} /> 高级 YAML
        </button>
      </div>

      {tab === "structured" ? (
        <Card>
          <CardHeader title="安全运行层" description="只修改常用运行字段；端口、TUN、DNS 等由服务自动管理。" />
          <CardBody className="space-y-4">
            <div className="grid gap-4 sm:grid-cols-2">
              <Field label="运行模式" htmlFor="core-mode">
                <Select
                  id="core-mode"
                  value={form.mode}
                  options={[
                    { value: "rule", label: "Rule" },
                    { value: "global", label: "Global" },
                    { value: "direct", label: "Direct" },
                  ]}
                  onChange={(event) => updateField("mode", event.currentTarget.value)}
                />
              </Field>
              <Field label="日志级别" htmlFor="core-log-level">
                <Select
                  id="core-log-level"
                  value={form["log-level"]}
                  options={[
                    { value: "silent", label: "Silent" },
                    { value: "error", label: "Error" },
                    { value: "warning", label: "Warning" },
                    { value: "info", label: "Info" },
                    { value: "debug", label: "Debug" },
                  ]}
                  onChange={(event) => updateField("log-level", event.currentTarget.value)}
                />
              </Field>
              <Field label="外部控制地址" htmlFor="core-controller">
                <Input id="core-controller" value={form["external-controller"]} onChange={(event) => updateField("external-controller", event.currentTarget.value)} />
              </Field>
              <Field label="外部 UI 路径" htmlFor="core-ui" hint="固定为 ./dashboard，由模块内置 zashboard 提供">
                <Input id="core-ui" value={form["external-ui"]} readOnly className="bg-surface-muted text-fg-muted" />
              </Field>
              <Field label="进程匹配模式" htmlFor="core-find-process" hint="实际值由设置页统一管理；保存后需重启核心">
                <Input
                  id="core-find-process"
                  value={form["find-process-mode"]}
                  readOnly
                  className="bg-surface-muted text-fg-muted"
                />
              </Field>
            </div>

            <div className="space-y-1 border-t border-border pt-3">
              {structuredFields.filter((field) => falseKeys.has(field.key)).map((field) => {
                const key = field.key;
                return (
                  <div key={key} className="flex min-h-11 items-center justify-between gap-3">
                    <div>
                      <p className="text-sm text-fg">{field.label}</p>
                      {field.hint ? <p className="mt-0.5 text-[11px] text-fg-subtle">{field.hint}</p> : null}
                    </div>
                    <Switch checked={Boolean(form[key])} onCheckedChange={(checked) => updateField(key, checked as never)} />
                  </div>
                );
              })}
            </div>
          </CardBody>
        </Card>
      ) : (
        <Card>
          <CardHeader title="完整 YAML" description="保存前做 YAML 语法校验；不会自动备份，误改可能导致服务无法启动。" />
          <CardBody>
            <Textarea
              id="core-raw"
              value={raw}
              className="min-h-[55vh] font-mono text-[11px] leading-5"
              spellCheck={false}
              onChange={(event) => setRaw(event.currentTarget.value)}
            />
          </CardBody>
        </Card>
      )}

      <StickyActionBar
        label="核心配置操作栏"
        status={dirty ? "有未保存的核心配置修改" : "核心配置已同步"}
      >
        {dirty ? (
          <Button
            variant="secondary"
            size="md"
            className="h-10 flex-1 px-3 text-xs sm:flex-none"
            onClick={() => { if (original) { setForm({ ...original.structured }); setRaw(original.raw); } }}
            disabled={saving}
          >
            撤销
          </Button>
        ) : null}
        <Button
          variant="primary"
          size="md"
          className={cn("h-10 px-3 text-xs", dirty ? "flex-1 sm:flex-none" : "w-full sm:w-auto")}
          onClick={() => void save()}
          loading={saving}
          disabled={!canSave}
          icon={<Save size={15} />}
        >
          保存核心配置
        </Button>
      </StickyActionBar>
    </div>
  );
}
