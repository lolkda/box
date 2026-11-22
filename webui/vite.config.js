import { defineConfig } from "vite";
import preact from "@preact/preset-vite";

export default defineConfig({
  plugins: [preact()],
  base: "./",
  build: {
    outDir: "../webroot",
    emptyOutDir: true,
    target: "es2020",
  },
});
