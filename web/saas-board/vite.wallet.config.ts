// The console's AG Grid wallet component, self-contained at a stable name
// (dist/assets/wallet-grid.js), so any host that serves it needs no other file.
import { defineConfig } from "vite";

export default defineConfig({
  define: { "process.env.NODE_ENV": JSON.stringify("production") },
  build: {
    outDir: "dist/assets",
    emptyOutDir: false,
    chunkSizeWarningLimit: 4000,
    lib: { entry: "src/wallet-grid.ts", formats: ["es"], fileName: () => "wallet-grid.js" },
  },
});
