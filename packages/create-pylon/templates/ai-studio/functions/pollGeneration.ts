import { action, v } from "@pylonsync/functions";
import { placeholderImage } from "../lib/studio";

// pollGeneration — the background job (enqueued by `generate` via
// ctx.scheduler.runAfter). It runs OFF the request path, so slow models can take
// as long as they need without blocking or timing out the user's call. It:
//   1. starts the Replicate prediction (first run), then
//   2. re-checks it and RESCHEDULES ITSELF every few seconds until it settles,
//   3. writes the result/error to the Generation — which syncs live to the
//      owner's gallery (pending → processing → done/failed) via db.useQuery.
//
// With no REPLICATE_API_TOKEN it returns a clearly-labeled placeholder instead,
// so the whole flow (and this job) work with zero config.
//
// Not internal, but harmless if called directly: it only advances a generation's
// own prediction; results sync only to the owner (Generation is owner-read).

const REPLICATE = "https://api.replicate.com/v1";
const POLL_MS = 3000;
const MAX_ATTEMPTS = 200; // ~10 min ceiling, then give up

export default action<{ id: string; attempts?: number }, { ok: boolean }>({
  auth: "public",
  args: { id: v.id("Generation"), attempts: v.optional(v.int()) },
  async handler(ctx, args) {
    const attempts = args.attempts ?? 0;
    const gen = await ctx.runQuery<{
      id: string;
      kind: string;
      prompt: string;
      status: string;
      predictionId?: string | null;
    } | null>("_getGeneration", { id: args.id });
    if (!gen || gen.status === "done" || gen.status === "failed") return { ok: true };

    const update = (patch: Record<string, unknown>) => ctx.runMutation("_updateGeneration", { id: args.id, ...patch });
    const reschedule = () =>
      ctx.scheduler.runAfter(POLL_MS, "pollGeneration", { id: args.id, attempts: attempts + 1 });

    if (attempts > MAX_ATTEMPTS) {
      await update({ status: "failed", error: "Generation timed out." });
      return { ok: true };
    }

    const token = ctx.env.REPLICATE_API_TOKEN?.trim();

    // ── Demo mode: no token → a labeled placeholder, no provider call. ──
    if (!token) {
      await update({
        status: "done",
        demo: true,
        resultUrl: gen.kind === "image" ? placeholderImage(gen.prompt) : null,
      });
      return { ok: true };
    }

    try {
      // First run: kick off the Replicate prediction.
      if (!gen.predictionId) {
        const model = modelFor(gen.kind, ctx.env);
        const pred = await replicateCreate(model, gen.prompt, token);
        if (pred.error) {
          await update({ status: "failed", error: String(pred.error).slice(0, 300) });
          return { ok: true };
        }
        const out = pickOutput(pred.output);
        if (pred.status === "succeeded" && out) {
          await update({ status: "done", resultUrl: out });
        } else if (pred.status === "failed" || pred.status === "canceled") {
          await update({ status: "failed", error: "Provider rejected the request." });
        } else {
          await update({ status: "processing", predictionId: pred.id });
          await reschedule();
        }
        return { ok: true };
      }

      // Subsequent runs: check the in-flight prediction.
      const pred = await replicateGet(gen.predictionId, token);
      const out = pickOutput(pred.output);
      if (pred.status === "succeeded" && out) {
        await update({ status: "done", resultUrl: out });
      } else if (pred.status === "failed" || pred.status === "canceled") {
        await update({ status: "failed", error: String(pred.error || "Generation failed.").slice(0, 300) });
      } else {
        await reschedule();
      }
    } catch (err) {
      // Transient network blip → try again; otherwise the attempts cap ends it.
      await reschedule();
    }
    return { ok: true };
  },
});

// Default models (run the latest version via the model-name endpoint). Override
// any of them with env so swapping models needs no code change.
function modelFor(kind: string, env: Record<string, string>): string {
  if (kind === "image") return env.REPLICATE_IMAGE_MODEL?.trim() || "black-forest-labs/flux-schnell";
  if (kind === "video") return env.REPLICATE_VIDEO_MODEL?.trim() || "minimax/video-01";
  return env.REPLICATE_AUDIO_MODEL?.trim() || "meta/musicgen";
}

function pickOutput(output: unknown): string | null {
  if (Array.isArray(output)) return typeof output[0] === "string" ? output[0] : null;
  return typeof output === "string" ? output : null;
}

interface Prediction {
  id: string;
  status: string;
  output?: unknown;
  error?: unknown;
}

async function replicateCreate(model: string, prompt: string, token: string): Promise<Prediction> {
  const res = await fetch(`${REPLICATE}/models/${model}/predictions`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({ input: { prompt } }),
  });
  const j = (await res.json().catch(() => ({}))) as Prediction & { detail?: string };
  if (!res.ok) return { id: "", status: "failed", error: j?.detail || `Replicate ${res.status}` };
  return j;
}

async function replicateGet(predictionId: string, token: string): Promise<Prediction> {
  const res = await fetch(`${REPLICATE}/predictions/${predictionId}`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  const j = (await res.json().catch(() => ({}))) as Prediction;
  if (!res.ok) return { id: predictionId, status: "processing" }; // transient — keep polling
  return j;
}
