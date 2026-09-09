import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://oddurs.github.io",
  base: "/quarry",
  build: { inlineStylesheets: "always" },
});
