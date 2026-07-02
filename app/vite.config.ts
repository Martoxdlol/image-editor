import path from 'node:path'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig, type ViteDevServer, type PreviewServer } from 'vite'

// COOP/COEP so SharedArrayBuffer/wasm threads can be probed (spec §12.3)
function crossOriginIsolation() {
  const setHeaders = (server: ViteDevServer | PreviewServer) => {
    server.middlewares.use((_req, res, next) => {
      res.setHeader('Cross-Origin-Opener-Policy', 'same-origin')
      res.setHeader('Cross-Origin-Embedder-Policy', 'require-corp')
      next()
    })
  }
  return {
    name: 'cross-origin-isolation',
    configureServer: setHeaders,
    configurePreviewServer: setHeaders,
  }
}

export default defineConfig({
  plugins: [react(), tailwindcss(), crossOriginIsolation()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  worker: {
    format: 'es',
  },
  build: {
    target: 'esnext',
  },
})
