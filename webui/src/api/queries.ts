import { useCallback, useEffect, useRef } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "./client";
import type { TaskInfo, TaskKind } from "./types";

export const queryKeys = {
  status: ["status"] as const,
  subscriptions: ["subscriptions"] as const,
  subscription: (name: string) => ["subscriptions", name] as const,
  config: ["config"] as const,
  ruleProfile: ["rule-profile"] as const,
  ruleProfileList: ["rule-profile-list"] as const,
  settings: ["settings"] as const,
  logs: ["logs"] as const,
  tasks: ["tasks"] as const,
  taskLogs: (id: string) => ["tasks", id, "logs"] as const,
  coreConfig: ["core-config"] as const,
  appLists: ["app-lists"] as const,
  networkRules: ["network-rules"] as const,
};

export function useStatusQuery(options: { enabled?: boolean; live?: boolean } = {}) {
  const { enabled = true, live = false } = options;
  return useQuery({
    queryKey: queryKeys.status,
    queryFn: api.status,
    enabled,
    retry: live ? 1 : 3,
    refetchInterval: live ? 15_000 : false,
    staleTime: live ? 10_000 : 30_000,
  });
}

export function useSubscriptionsQuery() {
  return useQuery({
    queryKey: queryKeys.subscriptions,
    queryFn: api.subscriptions,
    staleTime: 10_000,
  });
}

export function useConfigQuery() {
  return useQuery({
    queryKey: queryKeys.config,
    queryFn: api.config,
    staleTime: 30_000,
  });
}

export function useRuleProfileListQuery(options: { enabled?: boolean } = {}) {
  const { enabled = true } = options;
  return useQuery({
    queryKey: queryKeys.ruleProfileList,
    queryFn: api.ruleProfiles,
    enabled,
    staleTime: 10_000,
  });
}

export function useRuleProfileQuery(options: { enabled?: boolean } = {}) {
  const { enabled = true } = options;
  return useQuery({
    queryKey: queryKeys.ruleProfile,
    queryFn: api.ruleProfile,
    enabled,
    staleTime: 10_000,
  });
}

export function useSettingsQuery() {
  return useQuery({
    queryKey: queryKeys.settings,
    queryFn: api.settings,
    staleTime: 30_000,
  });
}

export function useLogsQuery(options: { paused?: boolean; interval?: number; enabled?: boolean } = {}) {
  const { paused = false, interval = 2000, enabled = true } = options;
  return useQuery({
    queryKey: queryKeys.logs,
    queryFn: () => api.logs(200),
    enabled,
    refetchInterval: paused ? false : interval,
    staleTime: 1000,
  });
}

export function useTasksQuery(options: { enabled?: boolean } = {}) {
  const { enabled = true } = options;
  return useQuery({
    queryKey: queryKeys.tasks,
    queryFn: api.taskList,
    enabled,
    refetchInterval: (query) =>
      query.state.data?.some((task) => task.state === "running") ? 2000 : false,
    staleTime: 500,
  });
}

export function useNetworkRulesQuery() {
  return useQuery({
    queryKey: queryKeys.networkRules,
    queryFn: api.networkRules,
    staleTime: 10_000,
  });
}

export function useCoreConfigQuery(enabled = true) {
  return useQuery({
    queryKey: queryKeys.coreConfig,
    queryFn: api.coreConfig,
    enabled,
    staleTime: 10_000,
  });
}

export function useAppListsQuery() {
  return useQuery({
    queryKey: queryKeys.appLists,
    queryFn: api.appLists,
    staleTime: 10_000,
  });
}

export function useTaskLogsQuery(id: string | null) {
  return useQuery({
    queryKey: queryKeys.taskLogs(id || ""),
    queryFn: () => api.taskLogs(id || "", 300),
    enabled: Boolean(id),
    refetchInterval: 2000,
    staleTime: 500,
  });
}

export function useTaskStart() {
  const queryClient = useQueryClient();
  return useCallback(
    async (kind: TaskKind, arg?: string) => {
      const task = await api.taskStart(kind, arg);
      queryClient.setQueryData<TaskInfo[]>(queryKeys.tasks, (current) => [
        task,
        ...(current || []).filter((item) => item.id !== task.id),
      ]);
      void queryClient.invalidateQueries({ queryKey: queryKeys.status });
      return task;
    },
    [queryClient],
  );
}

export function useInvalidateOperationalQueries() {
  const queryClient = useQueryClient();
  return useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.status });
    void queryClient.invalidateQueries({ queryKey: queryKeys.subscriptions });
    void queryClient.invalidateQueries({ queryKey: queryKeys.config });
    void queryClient.invalidateQueries({ queryKey: queryKeys.ruleProfile });
    void queryClient.invalidateQueries({ queryKey: queryKeys.ruleProfileList });
    void queryClient.invalidateQueries({ queryKey: queryKeys.settings });
    void queryClient.invalidateQueries({ queryKey: queryKeys.logs });
    void queryClient.invalidateQueries({ queryKey: queryKeys.tasks });
    void queryClient.invalidateQueries({ queryKey: queryKeys.coreConfig });
    void queryClient.invalidateQueries({ queryKey: queryKeys.appLists });
    void queryClient.invalidateQueries({ queryKey: queryKeys.networkRules });
  }, [queryClient]);
}

export function useTaskCompletionWatcher(onFinished?: (task: TaskInfo) => void, enabled = true) {
  const tasksQuery = useTasksQuery({ enabled });
  const invalidate = useInvalidateOperationalQueries();
  const previous = useRef<Map<string, TaskInfo["state"]>>(new Map());

  useEffect(() => {
    const tasks = tasksQuery.data || [];
    for (const task of tasks) {
      const oldState = previous.current.get(task.id);
      if (oldState === "running" && task.state !== "running") {
        invalidate();
        onFinished?.(task);
      }
      previous.current.set(task.id, task.state);
    }
  }, [tasksQuery.data, invalidate, onFinished]);
}
