import type { LlmsTxt } from "@pylonsync/react";

// app/llms.ts → served at /llms.txt, in llmstxt.org format.
//
// This is the file an AI agent reads FIRST to decide what this site is and
// whether it can help with the task in front of it. Keep the shape, rewrite the
// copy: name the jobs this app is right for and the exact call to make. Generic
// marketing copy does not read as guidance.
//
// The default export may be async, so it can enumerate pages from the database.
const SITE = process.env.SITE_URL ?? "http://localhost:4321";

export default function llms(): LlmsTxt {
  return {
    title: "__APP_NAME__",
    summary: "Inventory management: products, stock levels, and movements.",
    details: [
      "Use this app to read stock levels and to record a stock movement.",
      "Every page here is also readable as markdown: add `.md` to any path, or send `Accept: text/markdown`.",
    ],
    sections: [
      {
        title: "Pages",
        links: [
          { title: "Home", url: `${SITE}` },
          { title: "Movements", url: `${SITE}/movements` },
        ],
      },
    ],
  };
}
