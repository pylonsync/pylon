import type { LlmsTxt } from "@pylonsync/react";
import { SITE } from "@/lib/site";

// app/llms.ts → served at /llms.txt, in llmstxt.org format.
//
// This is the file an AI agent reads FIRST to decide what this site is and
// whether it can help with the task in front of it. Keep the shape, rewrite
// the copy: name the jobs this app is right for and the exact call to make.
const BASE = process.env.SITE_URL ?? "http://localhost:4321";

export default function llms(): LlmsTxt {
  return {
    title: SITE.name,
    summary: "A photo-sharing app: people post photos, follow each other, like, comment, and save posts.",
    details: [
      "Reading posts, profiles, and comments needs no account. Liking, commenting, following, and posting need a session; a visitor gets a guest session automatically.",
      "Post pages are at /p/<id> and profiles at /u/<username>.",
      "Every page here is also readable as markdown: add `.md` to any path, or send `Accept: text/markdown`.",
    ],
    sections: [
      {
        title: "Pages",
        links: [
          { title: "Feed", url: `${BASE}/` },
          { title: "Explore", url: `${BASE}/explore` },
        ],
      },
    ],
  };
}
