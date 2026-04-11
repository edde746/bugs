import type { RouteSectionProps } from "@solidjs/router";
import Sidebar from "./Sidebar";
import SearchBar from "~/components/search/SearchBar";

export default function AppShell(props: RouteSectionProps) {
  return (
    <div class="flex h-screen overflow-hidden">
      <Sidebar />
      <div class="flex flex-1 flex-col overflow-hidden">
        <header class="flex h-14 items-center border-b border-[var(--color-border)] bg-[var(--color-surface-0)] px-6">
          <SearchBar />
        </header>
        <main class="flex-1 overflow-y-auto bg-[var(--color-surface-0)]">
          {props.children}
        </main>
      </div>
    </div>
  );
}
