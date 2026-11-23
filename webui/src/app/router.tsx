import { lazy, Suspense, type ReactNode } from "react";
import { createHashRouter, Navigate } from "react-router-dom";
import { AppShell, RouteError } from "./shell";
import { PageSkeleton } from "../components/ui/skeleton";

const OverviewPage = lazy(() => import("../features/overview/overview-page").then((module) => ({ default: module.OverviewPage })));
const SubscriptionsPage = lazy(() => import("../features/subscriptions/subscriptions-page").then((module) => ({ default: module.SubscriptionsPage })));
const NodeSubscriptionsPage = lazy(() => import("../features/subscriptions/node-subscriptions-page").then((module) => ({ default: module.NodeSubscriptionsPage })));
const RuleProfilePage = lazy(() => import("../features/subscriptions/rule-profile-page").then((module) => ({ default: module.RuleProfilePage })));
const GroupsPage = lazy(() => import("../features/groups/groups-page").then((module) => ({ default: module.GroupsPage })));
const RulesPage = lazy(() => import("../features/rules/rules-page").then((module) => ({ default: module.RulesPage })));
const MorePage = lazy(() => import("../features/more/more-page").then((module) => ({ default: module.MorePage })));
const SettingsPage = lazy(() => import("../features/settings/settings-page").then((module) => ({ default: module.SettingsPage })));
const ProvidersPage = lazy(() => import("../features/providers/providers-page").then((module) => ({ default: module.ProvidersPage })));
const LogsPage = lazy(() => import("../features/logs/logs-page").then((module) => ({ default: module.LogsPage })));
const TasksPage = lazy(() => import("../features/tasks/tasks-page").then((module) => ({ default: module.TasksPage })));
const DangerPage = lazy(() => import("../features/danger/danger-page").then((module) => ({ default: module.DangerPage })));
const CoreConfigPage = lazy(() => import("../features/core-config/core-config-page").then((module) => ({ default: module.CoreConfigPage })));
const AppsNetworkPage = lazy(() => import("../features/apps-network/apps-network-page").then((module) => ({ default: module.AppsNetworkPage })));

function withSuspense(node: ReactNode): ReactNode {
  return <Suspense fallback={<PageSkeleton />}>{node}</Suspense>;
}

export const router = createHashRouter([
  {
    path: "/",
    element: <AppShell />,
    errorElement: <RouteError />,
    children: [
      { index: true, element: withSuspense(<OverviewPage />) },
      {
        path: "subscriptions",
        element: withSuspense(<SubscriptionsPage />),
        children: [
          { index: true, element: withSuspense(<NodeSubscriptionsPage />) },
          { path: "rule-profile", element: withSuspense(<RuleProfilePage />) },
        ],
      },
      { path: "groups", element: withSuspense(<GroupsPage />) },
      { path: "rules", element: withSuspense(<RulesPage />) },
      { path: "more", element: withSuspense(<MorePage />) },
      { path: "settings", element: withSuspense(<SettingsPage />) },
      { path: "providers", element: withSuspense(<ProvidersPage />) },
      { path: "logs", element: withSuspense(<LogsPage />) },
      { path: "tasks", element: withSuspense(<TasksPage />) },
      { path: "danger", element: withSuspense(<DangerPage />) },
      { path: "core-config", element: withSuspense(<CoreConfigPage />) },
      { path: "apps-network", element: withSuspense(<AppsNetworkPage />) },
      { path: "*", element: <Navigate to="/" replace /> },
    ],
  },
]);
