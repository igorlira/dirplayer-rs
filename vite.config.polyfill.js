import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import svgr from "vite-plugin-svgr";
import embedResources from "./polyfill/vite-plugin-embed-resources.js";
import path from "path";

/**
 * Plugin to inject CSS into the JS bundle
 */
function injectCss() {
  return {
    name: "inject-css",
    apply: "build",
    enforce: "post",
    generateBundle(options, bundle) {
      // Find the CSS file
      let cssContent = "";
      const cssFiles = [];
      for (const fileName in bundle) {
        if (fileName.endsWith(".css")) {
          cssContent += bundle[fileName].source;
          cssFiles.push(fileName);
        }
      }

      // Remove CSS files from bundle
      for (const fileName of cssFiles) {
        delete bundle[fileName];
      }

      // Inject CSS into JS files
      if (cssContent) {
        const cssInjection = `(function(){var style=document.createElement("style");style.textContent=${JSON.stringify(cssContent)};document.head.appendChild(style);})();`;
        for (const fileName in bundle) {
          if (fileName.endsWith(".js") && bundle[fileName].type === "chunk") {
            bundle[fileName].code = cssInjection + bundle[fileName].code;
          }
        }
      }
    },
  };
}

// https://vitejs.dev/config/
export default defineConfig({
  // Don't copy public folder for library build
  publicDir: false,
  // Replace Node.js globals for browser compatibility
  define: {
    "process.env.NODE_ENV": JSON.stringify("production"),
  },
  build: {
    outDir: "dist-polyfill",
    // Empty the output directory before build
    emptyOutDir: true,
    lib: {
      entry: path.resolve(__dirname, "polyfill/src/standalone.tsx"),
      name: "DirPlayerPolyfill",
      formats: ["iife"],
      fileName: () => "dirplayer-polyfill.js",
    },
    rollupOptions: {
      output: {
        // Ensure everything is bundled into a single file
        inlineDynamicImports: true,
      },
    },
  },
  plugins: [
    react(),
    svgr({
      svgrOptions: {
        icon: true,
      },
    }),
    embedResources({
      wasmPath: "vm-rust/pkg/vm_rust_bg.wasm",
      fontPath: "public/charmap-system.png",
    }),
    injectCss(),
  ],
});
