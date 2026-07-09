import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';

export default defineConfig({
  plugins: [vue()],
  build: {
    outDir: '../web',
    emptyOutDir: true,
    chunkSizeWarningLimit: 800,
    target: 'es2022',
    rollupOptions: {
      output: {
        manualChunks: {
          'chart': ['chart.js', 'chartjs-adapter-date-fns'],
          'xterm': ['@xterm/xterm', '@xterm/addon-fit'],
          'novnc': ['@novnc/novnc'],
          'vendor': ['vue', 'vue-router', 'pinia', 'vue-i18n'],
        },
      },
    },
  },
  optimizeDeps: {
    esbuildOptions: {
      target: 'es2022',
    },
  },
  server: {
    port: 5173,
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:8080',
        changeOrigin: true,
        ws: true,
      },
    },
  },
});
