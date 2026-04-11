import { useParams } from "@solidjs/router";

export function useCurrentProject() {
  const params = useParams<{ project: string }>();
  return () => params.project;
}
