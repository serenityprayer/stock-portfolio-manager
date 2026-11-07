import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: false,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1420,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target:
      process.env.TAURI_ENV_PLATFORM == "windows"
        ? "chrome105"
        : "safari13",
    minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    rollupOptions: {
      output: {
        // Split long-lived vendors out of page chunks so a page edit does not
        // invalidate the 3 MB UI framework bundle, and the entry chunk stays
        // small. Order matters: antd/rc-* must be matched before generic react.
        manualChunks(id) {
          if (!id.includes("node_modules")) return undefined;
          if (/node_modules[\\/](antd|@ant-design|rc-[^\\/]+)[\\/]/.test(id)) return "antd-vendor";
          if (/node_modules[\\/](echarts|zrender|recharts|d3-)[^\\/]*[\\/]?/.test(id)) return "charts";
          if (/node_modules[\\/](react|react-dom|react-router|react-router-dom|scheduler|zustand)[\\/]?/.test(id)) return "react-vendor";
          return "vendor";
        },
      },
    },
  },
}));
