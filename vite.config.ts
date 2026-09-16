import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

// The version the About panel shows comes from tauri.conf.json, which is the
// file the installer's own version is read from. Typing it into the UI as well
// is how an About line ends up claiming a version that was shipped months ago.
const appVersion: string = JSON.parse(
  readFileSync(resolve(__dirname, 'src-tauri/tauri.conf.json'), 'utf-8'),
).version

// Two entry points: the Alt+V popup and the settings window.
// Both are built as static pages and loaded by separate Tauri WebViews.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  define: {
    __APP_VERSION__: JSON.stringify(appVersion),
  },
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: {
    target: 'chrome105',
    rollupOptions: {
      input: {
        popup: resolve(__dirname, 'index.html'),
        settings: resolve(__dirname, 'settings.html'),
        traymenu: resolve(__dirname, 'traymenu.html'),
      },
    },
  },
})
