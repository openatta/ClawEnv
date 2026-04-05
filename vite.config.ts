import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [solid(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/core/**", "**/tauri/**", "**/cli/**"],
    },
  },
  build: {
    target: "esnext",
    outDir: "dist",
  },
});
