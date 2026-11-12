// The board is served by saas-web at money.agentrix.website/board/. `npm run dev` serves it on
// 18184 and proxies the feed, the agent and the kit's assets to saas-web on 18183, as the money
// host (money.localhost), so the dev server and the real one see the same origin rules.
import { defineConfig } from "vite";

const saasWeb = {
  target: "http://127.0.0.1:18183",
  changeOrigin: false,
  headers: { host: "money.localhost:18183" },
};

export default defineConfig({
  base: "/board/",
  server: {
    port: 18184,
    strictPort: true,
    host: "127.0.0.1",
    proxy: {
      "/feed.json": saasWeb,
      "/board/agent": saasWeb,
      "/board/config.json": saasWeb,
      "/static": saasWeb,
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    chunkSizeWarningLimit: 8000,
  },
});
