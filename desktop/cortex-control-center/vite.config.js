import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: {
    include: ["../../tests/control-center/web/**/*.test.js"],
  },
  server: {
    port: 1420,
    strictPort: true,
  },
  clearScreen: false,
});
