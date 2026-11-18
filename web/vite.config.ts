import { fileURLToPath } from 'node:url'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// The kit package is served from wherever a host puts it, so every asset path is relative.
export default defineConfig({
  base: './',
  plugins: [react()],
  resolve: {
    // @kition/kit-editor may be linked from kition-sdk (KITION_LINK=1); one React for the page.
    dedupe: ['react', 'react-dom'],
  },
  optimizeDeps: { exclude: ['@kition/kit-editor', '@kition/kit-host', '@kition/kit-ui-protocol', '@kition/kit-dev-host'] },
  worker: { format: 'es' },
  server: {
    port: 5187,
    fs: { allow: [fileURLToPath(new URL('..', import.meta.url)), fileURLToPath(new URL('../../kition-sdk', import.meta.url)), fileURLToPath(new URL('../../kition-tessera', import.meta.url))] },
  },
  build: {
    outDir: 'dist',
    rollupOptions: { input: { index: fileURLToPath(new URL('index.html', import.meta.url)), dev: fileURLToPath(new URL('dev.html', import.meta.url)) } },
  },
})
