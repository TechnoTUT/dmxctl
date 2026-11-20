import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'
import { viteSingleFile } from 'vite-plugin-singlefile'

// 成果物は1枚のHTMLにする。crates/backend/build.rs が DMXCTL_WEB_OUT を指定して実行し、include_str! で埋め込む
export default defineConfig({
  plugins: [react(), tailwindcss(), viteSingleFile()],
  build: { outDir: process.env.DMXCTL_WEB_OUT ?? 'dist', emptyOutDir: true, rollupOptions: { input: 'index.html' } },
  // 開発中は dmxctl serve のAPIへ中継する
  server: { proxy: { '/api': process.env.DMXCTL_WEB_TARGET ?? 'http://127.0.0.1:8080' } },
})
