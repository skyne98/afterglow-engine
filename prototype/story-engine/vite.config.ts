import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

/**
 * The Bun server runs on :8787. Vite serves the Vue app on :5173 and proxies
 * the JSON API and the WebSocket to the server, so the browser talks to a
 * single origin.
 */
export default defineConfig({
  root: "web",
  plugins: [vue()],
  server: {
    host: true,
    port: 5173,
    proxy: {
      "/api": "http://127.0.0.1:8787",
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "esnext",
  },
});
