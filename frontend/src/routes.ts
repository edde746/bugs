import { lazy } from "solid-js";
import type { RouteDefinition } from "@solidjs/router";
import AppShell from "~/layouts/AppShell";

const Home = lazy(() => import("~/pages/Home"));
const Onboarding = lazy(() => import("~/pages/Onboarding"));
const ProjectIssues = lazy(() => import("~/pages/ProjectIssues"));
const IssueDetail = lazy(() => import("~/pages/IssueDetail"));
const IssueEvents = lazy(() => import("~/pages/IssueEvents"));
const EventDetail = lazy(() => import("~/pages/EventDetail"));
const DirectEventDetail = lazy(() => import("~/pages/DirectEventDetail"));
const ProjectReleases = lazy(() => import("~/pages/ProjectReleases"));
const ProjectAlerts = lazy(() => import("~/pages/ProjectAlerts"));
const SettingsProjects = lazy(() => import("~/pages/SettingsProjects"));
const SettingsProjectDetail = lazy(() => import("~/pages/SettingsProjectDetail"));
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
    path: "/events/:eventId",
    component: AppShell,
    children: [
      {
        path: "/",
        component: DirectEventDetail,
      },
    ],
  },
  {
    path: "/settings/projects",
    component: AppShell,
    children: [
      {
        path: "/",
        component: SettingsProjects,
      },
      {
        path: "/:projectId",
        component: SettingsProjectDetail,
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
        path: "/issues/:issueId/events",
        component: IssueEvents,
      },
      {
        path: "/issues/:issueId/events/:eventId",
        component: EventDetail,
      },
      {
        path: "/releases",
        component: ProjectReleases,
      },
      {
        path: "/alerts",
        component: ProjectAlerts,
      },
    ],
  },
  {
    path: "/*",
    component: NotFound,
  },
];
