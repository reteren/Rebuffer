/// <reference types="svelte" />
/// <reference types="vite/client" />

/** Injected by vite.config.ts from src-tauri/tauri.conf.json, so the About
 *  panel and the installer can never disagree about the version. */
declare const __APP_VERSION__: string
