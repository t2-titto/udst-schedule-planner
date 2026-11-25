import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// `npm run dev` proxies /api to the Rust server. Start that with PLANNER_DEV_ORIGINS=http://localhost:5173.
export default defineConfig({
  plugins: [react()],
  server: { proxy: { '/api': { target: 'http://127.0.0.1:6767', changeOrigin: true } } },
})
