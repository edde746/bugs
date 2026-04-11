import { useNavigate } from "@solidjs/router";
import { createQuery } from "@tanstack/solid-query";
import { createEffect } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { Project } from "~/lib/sentry-types";
import LoadingSkeleton from "~/components/ui/LoadingSkeleton";

export default function Home() {
  const navigate = useNavigate();

  const projectsQuery = createQuery(() => ({
    queryKey: queryKeys.projects.all(),
    queryFn: () => api.get<Project[]>("/internal/projects"),
  }));

  createEffect(() => {
    const projects = projectsQuery.data;
    if (projects !== undefined) {
      if (projects.length > 0) {
        navigate(`/${projects[0]!.slug}/issues`, { replace: true });
      } else {
        navigate("/onboarding", { replace: true });
      }
    }
  });

  return (
    <div class="center-page">
      <LoadingSkeleton rows={3} />
    </div>
  );
}
