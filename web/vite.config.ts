import path from "node:path";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

// The build is committed to ../assets/dashboard and embedded in the wh
// binary, so file names are fixed (no hashes) and the Rust server lists
// each one. Fonts ship latin only.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": path.resolve(import.meta.dirname, "src") } },
  base: "/",
  build: {
    outDir: "../assets/dashboard",
    emptyOutDir: true,
    assetsInlineLimit: 0,
    modulePreload: false,
    cssCodeSplit: false,
    rollupOptions: {
      output: {
        entryFileNames: "app.js",
        chunkFileNames: "app-[name].js",
        assetFileNames: (asset) =>
          asset.names?.some((name) => name.endsWith(".css"))
            ? "app.css"
            : "fonts/[name][extname]",
        codeSplitting: false,
      },
    },
  },
  server: {
    // `npm run dev` proxies the API to a running `wh dash` (WH_DASH_URL).
    proxy: process.env.WH_DASH_URL
      ? {
          "/api": process.env.WH_DASH_URL,
          "/session": process.env.WH_DASH_URL,
        }
      : undefined,
  },
});
