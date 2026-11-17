import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed port it can point the webview at, and it must fail
// loudly rather than silently moving if something else has the port.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // The form presets are imported from the engine crate, so the dev server
    // has to be allowed to read one level up. There is one copy of that file
    // and this is what keeps it that way.
    fs: { allow: [".."] },
  },
  build: {
    outDir: "dist",
    target: "chrome110",
    sourcemap: true,
  },
});
