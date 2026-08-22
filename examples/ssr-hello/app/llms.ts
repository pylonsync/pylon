import type { LlmsTxt } from "@pylonsync/react";

// app/llms.ts → served at /llms.txt, in llmstxt.org format. The default export
// may be async, so a real site can enumerate its pages from the database.
//
// This is the file an agent reads first. Say what the site is and which pages
// are worth its context; skip the marketing.
const SITE = process.env.SITE_URL ?? "http://localhost:4321";

export default function llms(): LlmsTxt {
  return {
    title: "Pylon SSR example",
    summary:
      "Three server-rendered pages on one Pylon server: a home page, a dynamic route, and a gallery.",
    details: [
      "Read this app to see file-based SSR routing, layouts, and metadata without Next.js.",
      "Every page is also readable as markdown: add `.md` to any path, or send `Accept: text/markdown`.",
    ],
    sections: [
      {
        title: "Pages",
        links: [
          { title: "Home", url: `${SITE}/`, notes: "What the example covers" },
          { title: "Hello", url: `${SITE}/hello`, notes: "A second static route" },
          { title: "Gallery", url: `${SITE}/gallery`, notes: "Optimized <Image> output" },
        ],
      },
    ],
  };
}
