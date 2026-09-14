import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "path";

// Two entry points, two windows. The overlay is a separate document so it can
// stay framework-free and load nothing it does not need — its paint time is
// stage 1 of the latency budget.
export default defineConfig({
  // Only the Hub uses React; the overlay entry imports none of it, so nothing
  // of the runtime reaches that bundle.
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  build: {
    target: "chrome110",
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        overlay: resolve(__dirname, "overlay.html"),
        prompt: resolve(__dirname, "prompt.html"),
      },
    },
  },
});
