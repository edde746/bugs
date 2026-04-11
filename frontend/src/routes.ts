import { lazy } from "solid-js";
import type { RouteDefinition } from "@solidjs/router";
import AppShell from "~/layouts/AppShell";

const Home = lazy(() => import("~/pages/Home"));
const Onboarding = lazy(() => import("~/pages/Onboarding"));
const ProjectIssues = lazy(() => import("~/pages/ProjectIssues"));
const IssueDetail = lazy(() => import("~/pages/IssueDetail"));
const EventDetail = lazy(() => import("~/pages/EventDetail"));
const SettingsProjects = lazy(() => import("~/pages/SettingsProjects"));
const NotFound = lazy(() => import("~/pages/NotFound"));

export const routes: RouteDefinition[] = [
  {
    path: "/",
    component: Home,
  },
  {
    path: "/onboarding",
    component: Onboarding,
  },
  {
    path: "/settings/projects",
    component: AppShell,
    children: [
      {
        path: "/",
        component: SettingsProjects,
      },
    ],
  },
  {
    path: "/:project",
    component: AppShell,
    children: [
      {
        path: "/issues",
        component: ProjectIssues,
      },
      {
        path: "/issues/:issueId",
        component: IssueDetail,
      },
      {
        path: "/issues/:issueId/events/:eventId",
        component: EventDetail,
      },
    ],
  },
  {
    path: "/*",
    component: NotFound,
  },
];
