import path from 'node:path';
import basicSsl from '@vitejs/plugin-basic-ssl';
import tailwindcss from '@tailwindcss/vite';
import vue from '@vitejs/plugin-vue';
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
  base: './',
  plugins: [basicSsl(), vue(), tailwindcss(), crossOriginIsolationHeaders()],
  resolve: {
    alias: {
      '@': path.resolve(import.meta.dirname, 'src'),
    },
  },
  server: {
    host: '0.0.0.0',
    port: 5173,
  },
  preview: {
    host: '0.0.0.0',
  },
  build: {
    outDir: 'dist',
    target: 'esnext',
    rollupOptions: {
      input: {
        editor: path.resolve(import.meta.dirname, 'index.html'),
        paint: path.resolve(import.meta.dirname, 'paint/paint-demo.html'),
        paintNativeProbe: path.resolve(import.meta.dirname, 'paint/paint-native-probe.html'),
        paintMenuProbe: path.resolve(import.meta.dirname, 'paint/paint-menu-probe.html'),
        paintRecoveryProbe: path.resolve(import.meta.dirname, 'paint/paint-recovery-probe.html'),
        paintRasterProbe: path.resolve(import.meta.dirname, 'paint/paint-raster-probe.html'),
        paintStressProbe: path.resolve(import.meta.dirname, 'paint/paint-stress-probe.html'),
        paintCaptureProbe: path.resolve(import.meta.dirname, 'paint/paint-capture-probe.html'),
        paintCaptureControl: path.resolve(import.meta.dirname, 'paint/paint-capture-control.html'),
      },
    },
  },
});
