# __APP_NAME__

A generative **AI media studio** (image / audio / video) built with
[Pylon](https://pylonsync.com) — a live gallery that fills in as each generation
finishes, from one binary on one port. No Next.js, no job queue service.

Kick off a generation and a "generating…" card appears instantly, then flips to
the finished result the moment the server-side `generate` action resolves — live,
across every open tab. The provider call (and your API key) stays on the server.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321 and generate something — it works with **no config**:
image + audio return a clearly-labeled placeholder. Add an OpenAI key for real
media (below). Then **open a second tab** — your gallery stays in sync.

## Enable real generation

```bash
# .env
OPENAI_API_KEY=sk-...
# optional overrides:
OPENAI_IMAGE_MODEL=dall-e-3
OPENAI_TTS_MODEL=tts-1
```

- **Image** → OpenAI Images (`dall-e-3`), rendered from the returned URL.
- **Audio** → OpenAI text-to-speech, played from an inline `data:` URL.
- **Video** → a stubbed extension point. Wire a provider (Replicate / fal.ai /
  Runway / Luma) in `functions/generate.ts` where marked, and set its key.

## How it works

- `Generation` is an **owner-scoped** entity read with `db.useQuery` — private
  per user. Clients can't write it; only the server-side pipeline does.
- `functions/generate.ts` (a public `action`) brackets the provider call with
  two internal mutations: `_createGeneration` inserts a `pending` row (it appears
  in the gallery instantly), then `_finishGeneration` flips it to `done`/`failed`
  — and that change syncs to every open tab, so the card updates live.
- `<EnsureGuest>` lets anyone generate (and own their gallery); signing in is
  optional and carries the gallery across devices.

## Notes

- With a key, image results use the provider's **hosted URL** (small to sync,
  valid ~1h — fine for a live studio). For permanent results, request `b64_json`
  and persist via `/api/files`.
- Audio is stored as an inline `data:` URL so it's self-contained.

## Rebrand it

Brand, colors, the generation kinds, and the starter prompts all live in
**`lib/site.config.ts`**.

## Layout

```
app.ts                  Generation (owner-scoped) + User
lib/site.config.ts      brand + kinds + example prompts (edit this)
lib/studio.ts           types + the no-key placeholder generator
functions/generate.ts   public action: provider call + graceful placeholder
functions/_createGeneration.ts, _finishGeneration.ts   internal mutations
app/page.tsx            header + studio island
app/studio-client.tsx   prompt bar + kind selector + live gallery
```

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
