import { defineConfig, type PluginOption } from 'vite';

/** Serve the COOP/COEP headers the WASM threads (SharedArrayBuffer) require. */
function crossOriginIsolationHeaders(): PluginOption {
  return {
    name: 'cross-origin-isolation-headers',
    configureServer(server) {
      server.middlewares.use((_req, res, next) => {
        res.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
        res.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
        next();
      });
    },
    // Also apply to the preview server so production builds share memory.
    configurePreviewServer(server) {
      server.middlewares.use((_req, res, next) => {
        res.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
        res.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
        next();
      });
    },
  };
}

export default defineConfig({
  root: '.',
  plugins: [crossOriginIsolationHeaders()],
  server: {
    port: 5173,
  },
  build: {
    outDir: 'dist',
    target: 'esnext',
  },
});
