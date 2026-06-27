// Single source of truth for everything brand-specific on this AI chat app —
// edit this one file to rebrand; the layout + chat UI read from here. The
// create-pylon scaffolder and Mast target this file.
//
// Colors live here (applied as CSS variables on <html> in app/layout.tsx).
// Fictional demo copy — replace the values, keep the shape.

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

// A selectable model. `id` is what's sent to /api/ai/stream; `provider` is just
// a label for the picker. Which models are actually accepted is gated server-side
// by PYLON_AI_MODELS_ALLOWED (+ the configured provider / gateway) — see README.
export type ChatModel = { id: string; label: string; provider: string };

export type ChatConfig = BaseConfig & {
  chat: {
    assistantName: string;
    // Prepended as a system message on every request — sets the assistant's
    // persona. Edit to retune the assistant without touching code.
    systemPrompt: string;
    emptyHeadline: string;
    emptySubcopy: string;
    inputPlaceholder: string;
    // Starter prompts shown on the empty state; clicking one sends it.
    suggestions: string[];
    // The model picker. Curate to your setup: a single provider's models, or —
    // with an OpenRouter-style gateway (PYLON_AI_PROVIDER=custom) — models across
    // providers in one list. The selected id is sent as `model` per request.
    models: ChatModel[];
    defaultModel: string;
  };
};

/* ----------------------------- config ---------------------------- */

export const siteConfig: ChatConfig = {
  brand: {
    name: "Lumen",
    letter: "L",
    domain: "lumen.chat",
    email: "hello@lumen.example",
    footerBlurb: "A fast, streaming AI assistant built on Pylon.",
    copyrightName: "Lumen",
    socials: [
      {
        label: "X",
        href: "https://x.com",
        path: "M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z",
      },
    ],
  },

  colors: { brand: "#6d28d9", brandSoft: "#ede9fe", paper: "#faf9fc" },

  seo: {
    title: "Lumen — a fast streaming AI assistant.",
    description:
      "A streaming AI chat app built on Pylon. Conversations sync across your tabs and devices in realtime; your API key never leaves the server.",
  },

  chat: {
    assistantName: "Lumen",
    systemPrompt:
      "You are Lumen, a friendly, concise AI assistant. Answer clearly and get to the point. Use Markdown when it helps.",
    emptyHeadline: "How can I help?",
    emptySubcopy: "Ask anything. Your conversations are saved and stay in sync across your tabs.",
    inputPlaceholder: "Message Lumen…",
    suggestions: [
      "Explain WebSockets like I'm five.",
      "Draft a friendly out-of-office reply.",
      "Give me 5 dinner ideas using chicken and rice.",
      "What's the difference between SSR and SSG?",
    ],
    // Current Claude models (verify the ids for your provider before shipping —
    // model names move fast). Whichever you keep here must be in
    // PYLON_AI_MODELS_ALLOWED for switching to work (see .env.example). To offer
    // models from OTHER providers (OpenAI, Google, …) in the same list, route
    // through an OpenRouter-style gateway (PYLON_AI_PROVIDER=custom +
    // PYLON_AI_BASE_URL) and use that gateway's slugs here.
    models: [
      { id: "claude-sonnet-4-6", label: "Claude Sonnet 4.6", provider: "Anthropic" },
      { id: "claude-opus-4-8", label: "Claude Opus 4.8", provider: "Anthropic" },
      { id: "claude-haiku-4-5", label: "Claude Haiku 4.5", provider: "Anthropic" },
    ],
    defaultModel: "claude-sonnet-4-6",
  },
};
