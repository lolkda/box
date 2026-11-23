import { useEffect, useRef } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "./client";
import type { TaskInfo } from "./types";

export const queryKeys = {
  status: ["status"] as const,
  subscriptions: ["subscriptions"] as const,
  subscription: (name: string) => ["subscriptions", name] as const,
  config: ["config"] as const,
  settings: ["settings"] as const,
  logs: ["logs"] as const,
  tasks: ["tasks"] as const,
  taskLogs: (id: string) => ["tasks", id, "logs"] as const,
};

export function useStatusQuery() {
  return useQuery({
    queryKey: queryKeys.status,
    queryFn: api.status,
    refetchInterval: 5000,
    staleTime: 2000,
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

export function useSettingsQuery() {
  return useQuery({
    queryKey: queryKeys.settings,
    queryFn: api.settings,
    staleTime: 30_000,
  });
}

export function useLogsQuery(paused = false) {
  return useQuery({
    queryKey: queryKeys.logs,
    queryFn: () => api.logs(200),
    refetchInterval: paused ? false : 2000,
    staleTime: 1000,
  });
}

export function useTasksQuery() {
  return useQuery({
    queryKey: queryKeys.tasks,
    queryFn: api.taskList,
    refetchInterval: 1500,
    staleTime: 500,
  });
}

export function useTaskLogsQuery(id: string | null) {
  return useQuery({
    queryKey: queryKeys.taskLogs(id || ""),
    queryFn: () => api.taskLogs(id || "", 300),
    enabled: Boolean(id),
    refetchInterval: 1500,
    staleTime: 500,
  });
}

export function useInvalidateOperationalQueries() {
  const queryClient = useQueryClient();
  return () => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.status });
    void queryClient.invalidateQueries({ queryKey: queryKeys.subscriptions });
    void queryClient.invalidateQueries({ queryKey: queryKeys.config });
    void queryClient.invalidateQueries({ queryKey: queryKeys.settings });
    void queryClient.invalidateQueries({ queryKey: queryKeys.logs });
    void queryClient.invalidateQueries({ queryKey: queryKeys.tasks });
  };
}

export function useTaskCompletionWatcher(onFinished?: (task: TaskInfo) => void) {
  const tasksQuery = useTasksQuery();
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
