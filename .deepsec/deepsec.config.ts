import { defineConfig } from "deepsec/config";

export default defineConfig({
  projects: [
    { id: "pylon", root: ".." },
    // <deepsec:projects-insert-above>
  ],
});
