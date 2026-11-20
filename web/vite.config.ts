import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'
import { viteSingleFile } from 'vite-plugin-singlefile'

// 成果物は1枚のHTMLにして、バックエンドが include_str! で埋め込む
export default defineConfig({
  plugins: [react(), viteSingleFile()],
  build: { outDir: '../crates/backend/src/web', emptyOutDir: false, rollupOptions: { input: 'index.html' } },
  // 開発中は dmxctl serve のAPIへ中継する
  server: { proxy: { '/api': process.env.DMXCTL_WEB_TARGET ?? 'http://127.0.0.1:8080' } },
})
