import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';

const panel = process.env.SYNTHPANEL_DEV_URL ?? 'https://127.0.0.1:8443';

export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    assetsDir: 'assets',
    target: 'es2022',
    sourcemap: false,
  },
  server: {
    port: 5173,
    proxy: {
      '/api': { target: panel, changeOrigin: true, secure: false },
      '/healthz': { target: panel, changeOrigin: true, secure: false },
    },
  },
});
