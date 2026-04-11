import type { RouteSectionProps } from "@solidjs/router";
import Sidebar from "./Sidebar";
import SearchBar from "~/components/search/SearchBar";

export default function AppShell(props: RouteSectionProps) {
  return (
    <div class="app-shell">
      <Sidebar />
      <div class="app-shell__content">
        <header class="app-shell__header">
          <SearchBar />
        </header>
        <main class="app-shell__main">
          {props.children}
        </main>
      </div>
    </div>
  );
}
