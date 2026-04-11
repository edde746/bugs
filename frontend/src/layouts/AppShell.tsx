import type { RouteSectionProps } from "@solidjs/router";
import Sidebar from "./Sidebar";

export default function AppShell(props: RouteSectionProps) {
  return (
    <div class="flex h-screen overflow-hidden">
      <Sidebar />
      <main class="flex-1 overflow-y-auto bg-[var(--color-surface-0)]">
        {props.children}
      </main>
    </div>
  );
}
