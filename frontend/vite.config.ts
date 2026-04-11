import { defineConfig } from "vite";
import solidPlugin from "vite-plugin-solid";
import Icons from "unplugin-icons/vite";
import path from "node:path";

export default defineConfig({
  plugins: [solidPlugin(), Icons({ compiler: "solid" })],
  resolve: { alias: { "~": path.resolve(__dirname, "./src") } },
  build: {
    target: "es2022",
    outDir: "dist",
    rollupOptions: {
      output: {
        manualChunks(id: string) {
          if (
            id.includes("node_modules/solid-js") ||
            id.includes("node_modules/@solidjs/router")
          ) {
            return "solid-vendor";
          }
          if (id.includes("node_modules/@tanstack")) {
            return "query";
          }
        },
      },
    },
  },
  server: {
    port: 3000,
    proxy: { "/api": { target: "http://localhost:9000", changeOrigin: true } },
  },
});
