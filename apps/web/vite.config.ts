import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "path";

// Three documents rather than a router. The site is three pages of static text
// on a static host; a client-side router would add a dependency, a bundle, and
// a server rewrite rule to serve the same three URLs, and would send anyone who
// asked for the privacy policy a copy of the landing page's animations first.
export default defineConfig({
  plugins: [react()],
  build: {
    target: "chrome110",
    rollupOptions: {
      input: {
        home: resolve(__dirname, "index.html"),
        terms: resolve(__dirname, "terms/index.html"),
        privacy: resolve(__dirname, "privacy/index.html"),
      },
    },
  },
});
