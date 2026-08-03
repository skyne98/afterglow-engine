import { defineConfig } from 'vite';

export default defineConfig({
  root: '.',
  server: {
    port: 5173,
    // Serve baked static assets from the public dir (Vite handles this).
  },
  build: {
    outDir: 'dist',
    target: 'esnext',
  },
});
