import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  use: { baseURL: "http://127.0.0.1:1420", headless: true, viewport: { width: 1280, height: 900 } },
  webServer: { command: "npm run dev -- --host 127.0.0.1", url: "http://127.0.0.1:1420", reuseExistingServer: !process.env.CI },
});
