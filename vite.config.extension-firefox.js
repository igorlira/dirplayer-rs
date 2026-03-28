import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import svgr from "vite-plugin-svgr";

// https://vitejs.dev/config/
export default defineConfig({
  build: {
    outDir: "dist-extension-firefox",
    emptyOutDir: true,
    rollupOptions: {
      input: "extension/src/content-script.tsx",
      output: {
        entryFileNames: "content-script.js",
        assetFileNames: "content-script.[ext]",
        format: "iife",
        inlineDynamicImports: true,
      },
    },
    // Content scripts can't use ES modules
    target: "firefox109",
    // Extract CSS to a separate file (injected via manifest, not JS)
    // This avoids the document.head is null error at document_start
    cssCodeSplit: false,
  },
  // Don't copy public/ files into the output
  publicDir: false,
  plugins: [
    react(),
    svgr({
      svgrOptions: {
        icon: true,
      },
    }),
  ],
});
