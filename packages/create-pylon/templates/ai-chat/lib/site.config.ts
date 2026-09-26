// Brand-specific copy and settings. The layout, the chat UI, the server
// functions (system prompt, model allowlist, limits), and the scaffolder read
// this file. Replace the values and keep the shape.
//
// Colors are applied as CSS variables on <html> in app/layout.tsx.

/* ----------------------------- types ----------------------------- */

export type Social = { label: string; href: string; path: string };

export type BaseConfig = {
  brand: {
    name: string;
    letter: string;
    domain: string;
    email: string;
    footerBlurb: string;
    copyrightName: string;
    socials: Social[];
  };
  colors: { brand: string; brandSoft: string; paper: string };
  seo: { title: string; description: string };
};

// A selectable model. `id` is sent to the provider; `label` and `note` are
// shown in the picker. Only ids listed here are accepted by the `respond`
// function, and app.ts passes the same list to `llm({ allowedModels })`.
export type ChatModel = { id: string; label: string; note: string };

export type Suggestion = { title: string; prompt: string };

export type ChatConfig = BaseConfig & {
  chat: {
    // Sent as the system prompt on every request.
    systemPrompt: string;
    emptyHeadline: string;
    inputPlaceholder: string;
    // Starter prompts on the empty state. Picking one sends `prompt`.
    suggestions: Suggestion[];
    models: ChatModel[];
    defaultModel: string;
    // Upper bound on tokens per reply.
    maxOutputTokens: number;
    // Messages longer than this are refused by the server.
    maxInputChars: number;
    // Replies per rolling hour. `guest` and `user` are per person;
    // `allGuests` caps every guest session together, since anyone can start a
    // new guest session.
    repliesPerHour: { guest: number; user: number; allGuests: number };
    // Model ids a guest session may use. Accounts may use every model.
    guestModels: string[];
  };
};

/* ----------------------------- config ---------------------------- */

export const siteConfig: ChatConfig = {
  brand: {
    name: "Lumen",
    letter: "L",
    domain: "lumen.chat",
    email: "hello@lumen.example",
    footerBlurb: "A streaming AI assistant built on Pylon.",
    copyrightName: "Lumen",
    socials: [
      {
        label: "X",
        href: "https://x.com",
        path: "M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z",
      },
    ],
  },

  colors: { brand: "#1d5c4d", brandSoft: "#e3eeea", paper: "#f4f1ea" },

  seo: {
    title: "Lumen, a streaming AI assistant",
    description:
      "Ask questions and read answers as they stream. Conversations sync across your tabs and devices, and your API key stays on the server.",
  },

  chat: {
    systemPrompt:
      "You are Lumen, a helpful AI assistant. Answer clearly and get to the point. Use Markdown: short paragraphs, lists where they help, and fenced code blocks with a language tag for code.",
    emptyHeadline: "What can I help with?",
    inputPlaceholder: "Message Lumen",
    suggestions: [
      {
        title: "Explain a concept",
        prompt: "Explain how WebSockets differ from Server-Sent Events, and when to pick each.",
      },
      {
        title: "Write code",
        prompt: "Write a TypeScript function that retries a fetch with exponential backoff and jitter.",
      },
      {
        title: "Edit my writing",
        prompt: "Rewrite this to be shorter and clearer: \"We are reaching out to let you know that we have made some changes to our pricing.\"",
      },
      {
        title: "Plan something",
        prompt: "Plan a 3-day trip to Lisbon for someone who likes food markets and walking.",
      },
    ],
    // Verify the ids for your provider before shipping. Every id here is
    // accepted by the server; remove the ones you do not want to pay for.
    models: [
      { id: "claude-sonnet-4-6", label: "Claude Sonnet 4.6", note: "Balanced speed and quality" },
      { id: "claude-opus-4-8", label: "Claude Opus 4.8", note: "Best for hard problems" },
      { id: "claude-haiku-4-5", label: "Claude Haiku 4.5", note: "Fastest replies" },
    ],
    defaultModel: "claude-sonnet-4-6",
    maxOutputTokens: 4096,
    maxInputChars: 12000,
    repliesPerHour: { guest: 20, user: 100, allGuests: 300 },
    guestModels: ["claude-sonnet-4-6", "claude-haiku-4-5"],
  },
};
