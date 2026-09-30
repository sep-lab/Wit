import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// https://vite.dev/config/
//
// Tauri note: `npx tauri dev` shells out to this dev server (fixed port,
// strict, so Tauri's webview always finds it) and `npx tauri build` runs
// `vite build` before bundling. No network code lives here — this is only
// the local asset pipeline (ADR-0008: no telemetry, no auto-updater).
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  // Under Vitest, force Svelte's package.json "browser" export condition
  // — otherwise `import { mount } from "svelte"` resolves to the
  // server-side render build (`svelte/index-server.js`, whose `mount()`
  // always throws) rather than the client build a jsdom component test
  // needs (review round 1, non-blocking #9: this is what the new
  // component-mount tests need to actually mount anything).
  resolve: process.env.VITEST ? { conditions: ["browser"] } : undefined,
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // Tauri's own minimum-supported webview versions (Tauri 2 docs).
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
    css: false,
  },
});
