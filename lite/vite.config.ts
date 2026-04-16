import { defineConfig } from "vite";
import solidPlugin from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";
import path from "path";

export default defineConfig({
  plugins: [solidPlugin(), tailwindcss()],
  resolve: {
    alias: {
      // Share components, types, and i18n from the main app
      "@components": path.resolve(__dirname, "../src/components"),
      "@shared": path.resolve(__dirname, "../src"),
    },
  },
  server: {
    port: 1421,
    strictPort: true,
  },
  build: {
    target: "esnext",
    outDir: "dist",
  },
});
