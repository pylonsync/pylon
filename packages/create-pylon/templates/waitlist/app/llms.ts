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
    summary: "A coming-soon page with email capture and a live signup counter.",
    details: [
      "Use this app to join the waitlist with an email address. The owner dashboard lists the signups.",
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
