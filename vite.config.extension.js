import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import svgr from "vite-plugin-svgr";
import { crx } from "@crxjs/vite-plugin";
import manifest from "./extension/manifest.json";

// https://vitejs.dev/config/
export default defineConfig({
  build: {
    outDir: "dist-extension",
    // ES2020 → required for BigInt literals (`32n`, `0n`) which
    // dirplayer-js-api uses to pack u64 ptr+len values from the xtra
    // wasm boundary. Vite's default target downgrades to safari13 which
    // doesn't support BigInt; bumping the floor to es2020 matches the
    // browser baseline every Chromium/Firefox/Safari extension API
    // already requires (chrome67+/firefox68+/safari14+/edge79+).
    target: "es2020",
  },
  plugins: [
    react(),
    svgr({
      svgrOptions: {
        icon: true,
        // ...svgr options (https://react-svgr.com/docs/options/)
      },
    }),
    // Build Chrome Extension
    crx({ manifest }),
  ],
});
