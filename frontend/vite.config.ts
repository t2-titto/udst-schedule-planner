import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// `npm run dev` proxies /api to the Rust server. Start that with PLANNER_DEV_ORIGINS=http://localhost:5173.
export default defineConfig({
  plugins: [react()],
  server: { proxy: { '/api': { target: 'http://127.0.0.1:6767', changeOrigin: true } } },
  build: {
    // Terser (vs. the default esbuild minifier) also mangles top-level/property names and
    // strips comments, so the shipped bundle is harder to read than a default Vite build.
    // It's still plain JS that must run in the browser, so this only raises the effort to
    // read it back, not a real barrier -- the license is what actually forbids reuse.
    minify: 'terser',
    terserOptions: {
      compress: { passes: 2 },
      mangle: { toplevel: true },
      format: { comments: false },
    },
  },
})
