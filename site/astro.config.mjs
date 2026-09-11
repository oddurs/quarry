import { defineConfig } from "astro/config";

import react from "@astrojs/react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  site: "https://oddurs.github.io",
  base: "/quarry",
  build: { inlineStylesheets: "always" },
  integrations: [react()],

  vite: {
    plugins: [tailwindcss()],
  },
});