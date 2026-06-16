# __APP_NAME__

A generative **AI media studio** (image / audio / video) built with
[Pylon](https://pylonsync.com) — a live gallery that fills in as each generation
finishes, from one binary on one port. No Next.js, no separate job service.

Kick off a generation and a card appears instantly, then flips to the finished
result the moment the generation completes — live, across every open tab. The
generation runs in a **background job** (so even a minutes-long video never
blocks your request), and the provider call + API token stay on the server.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321 and generate something — it works with **no config**
(a clearly-labeled placeholder). Add a Replicate token for real media (below).
Then **open a second tab** — your gallery stays in sync.

## Enable real generation (Replicate)

```bash
# .env  — get a token at https://replicate.com/account/api-tokens
REPLICATE_API_TOKEN=r8_...
# optional model overrides (defaults shown):
REPLICATE_IMAGE_MODEL=black-forest-labs/flux-schnell
REPLICATE_AUDIO_MODEL=meta/musicgen
REPLICATE_VIDEO_MODEL=minimax/video-01
```

One provider, all three media. Models run via Replicate's model-name endpoint
(latest version — no version hashes to maintain). Results are the provider's
hosted URLs.

## How it works (background jobs + realtime)

- `Generation` is an **owner-scoped** entity read with `db.useQuery` — private
  per user. Clients can't write it; only the server-side pipeline does.
- `functions/generate.ts` (a `mutation`) inserts a `pending` row and enqueues a
  job with `ctx.scheduler.runAfter` — then returns. No network I/O on the
  request path, so slow models can't time it out.
- `functions/pollGeneration.ts` (a scheduled `action`) starts the Replicate
  prediction, then **reschedules itself** every few seconds until it settles,
  writing the result via the internal `_updateGeneration` mutation. Each write
  syncs to the owner's gallery — `pending → processing → done` — live.

## Notes

- Without a token, image generations return an SVG placeholder; audio/video
  cards show a "add a token" note. The whole flow (and the background job) still
  runs, so you can see the realtime gallery with zero config.
- Results are Replicate's hosted URLs (fine for a live studio). For permanent
  storage, download the asset in the job and persist via `/api/files`.
- Video is genuinely wired (Replicate has text-to-video models) — it just takes
  longer, which is exactly why the work runs in a background job.

## Rebrand it

Brand, colors, the generation kinds, and the starter prompts all live in
**`lib/site.config.ts`**.

## Layout

```
app.ts                          Generation (owner-scoped) + User
lib/site.config.ts              brand + kinds + example prompts (edit this)
lib/studio.ts                   types + the no-token placeholder generator
functions/generate.ts           mutation: insert pending + enqueue the job
functions/pollGeneration.ts     scheduled action: Replicate call + self-poll
functions/_getGeneration.ts, _updateGeneration.ts   internal read/write
app/page.tsx                    header + studio island
app/studio-client.tsx           prompt bar + kind selector + live gallery
```

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
