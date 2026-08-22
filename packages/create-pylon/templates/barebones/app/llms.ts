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
    summary: "A starter app with one entity and a live list.",
    details: [
      "Use this app to read and create items. Every change is pushed to open clients over the sync connection.",
      "Every page here is also readable as markdown: add `.md` to any path, or send `Accept: text/markdown`.",
    ],
    sections: [
      {
        title: "Pages",
        links: [
          { title: "Home", url: `${SITE}` },
        ],
      },
    ],
  };
}
