import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fontsourceAssets } from './scripts/fontsource-assets.mjs';

const host = process.env.TAURI_DEV_HOST;

function manualChunks(id: string) {
  if (!id.includes("node_modules")) return undefined;
  if (/[\\/]node_modules[\\/](react|react-dom|scheduler)[\\/]/.test(id)) {
    return "vendor-react";
  }
  if (id.includes("framer-motion")) {
    return "vendor-motion";
  }
  if (id.includes("lucide-react") || id.includes("react-icons")) {
    return "vendor-icons";
  }
  if (id.includes("@tauri-apps")) {
    return "vendor-tauri";
  }
  // Let Rollup split Markdown by its actual consumers. Startup speech metadata
  // uses the small AST parser; grouping all Markdown dependencies together also
  // loads React rendering, HTML sanitization and math before a chat is opened.
  if (id.includes("@dnd-kit") || id.includes("cmdk")) {
    return "vendor-interactions";
  }
  return undefined;
}

export default defineConfig(async () => ({
  plugins: [fontsourceAssets(), react(), tailwindcss()],

  clearScreen: false,

  build: {
    chunkSizeWarningLimit: 650,
    rollupOptions: {
      input: { main: 'index.html', remote: 'phone.html' },
      output: {
        manualChunks,
      },
    },
  },

  server: {
    port: 5173,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));
