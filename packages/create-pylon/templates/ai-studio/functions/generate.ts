import { action, v } from "@pylonsync/functions";
import { placeholderImage } from "../lib/studio";

// generate — turn a prompt into media. An `action` (network I/O to the provider)
// that brackets the call with two internal mutations:
//   1. _createGeneration → inserts a pending row (the gallery shows it instantly)
//   2. call the provider (or build a placeholder)
//   3. _finishGeneration → flips the row to done/failed (the gallery updates live)
//
// `auth: "public"` so a guest can generate (the studio mints a guest session).
// The provider key stays here on the server — it never reaches the browser.
//
// Image + audio call OpenAI when OPENAI_API_KEY is set; with no key they return
// a clearly-labeled placeholder so the whole flow + live gallery work with zero
// config. Video is intentionally a stub — wire your provider where marked below.
export default action<{ kind: string; prompt: string }, { id: string }>({
  auth: "public",
  args: { kind: v.string(), prompt: v.string() },
  async handler(ctx, args) {
    const kind = args.kind;
    const prompt = args.prompt.trim();
    if (!["image", "audio", "video"].includes(kind)) {
      throw ctx.error("INVALID_ARGS", "kind must be image, audio, or video.");
    }
    if (prompt.length < 2 || prompt.length > 1000) {
      throw ctx.error("INVALID_ARGS", "Enter a prompt (up to 1000 characters).");
    }

    const { id } = await ctx.runMutation<{ id: string }>("_createGeneration", { kind, prompt });
    const key = ctx.env.OPENAI_API_KEY?.trim();

    try {
      let resultUrl: string | null = null;
      let demo = false;

      if (kind === "image") {
        if (key) {
          resultUrl = await openaiImage(prompt, key, ctx.env.OPENAI_IMAGE_MODEL);
        } else {
          resultUrl = placeholderImage(prompt);
          demo = true;
        }
      } else if (kind === "audio") {
        if (key) {
          resultUrl = await openaiSpeech(prompt, key, ctx.env.OPENAI_TTS_MODEL);
        } else {
          // No key → no real audio to fake; mark it a demo (the card explains).
          demo = true;
        }
      } else {
        // ── video: extension point ──────────────────────────────────────────
        // No first-party video API here. Wire a provider (Replicate / fal.ai /
        // Runway / Luma): POST the prompt, poll for the asset, set resultUrl to
        // the returned video URL. Until then, video is a labeled placeholder.
        demo = true;
      }

      await ctx.runMutation("_finishGeneration", { id, status: "done", resultUrl, demo });
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Generation failed.";
      await ctx.runMutation("_finishGeneration", { id, status: "failed", error: msg.slice(0, 300) });
    }

    return { id };
  },
});

// OpenAI image generation. Uses response_format "url" (a hosted URL — small to
// store + sync, valid ~1h, fine for a live demo). For permanent results, request
// b64_json and persist it via /api/files. Model defaults to dall-e-3.
async function openaiImage(prompt: string, key: string, model?: string): Promise<string> {
  const res = await fetch("https://api.openai.com/v1/images/generations", {
    method: "POST",
    headers: { Authorization: `Bearer ${key}`, "content-type": "application/json" },
    body: JSON.stringify({
      model: model?.trim() || "dall-e-3",
      prompt,
      n: 1,
      size: "1024x1024",
      response_format: "url",
    }),
  });
  const j: any = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(j?.error?.message || `image generation failed (${res.status})`);
  const url = j?.data?.[0]?.url;
  const b64 = j?.data?.[0]?.b64_json;
  if (url) return url;
  if (b64) return `data:image/png;base64,${b64}`;
  throw new Error("image generation returned no result");
}

// OpenAI text-to-speech → an mp3 data: URL (self-contained, drops into <audio>).
async function openaiSpeech(prompt: string, key: string, model?: string): Promise<string> {
  const res = await fetch("https://api.openai.com/v1/audio/speech", {
    method: "POST",
    headers: { Authorization: `Bearer ${key}`, "content-type": "application/json" },
    body: JSON.stringify({
      model: model?.trim() || "tts-1",
      voice: "alloy",
      input: prompt.slice(0, 4000),
    }),
  });
  if (!res.ok) {
    const j: any = await res.json().catch(() => ({}));
    throw new Error(j?.error?.message || `speech generation failed (${res.status})`);
  }
  const buf = await res.arrayBuffer();
  const b64 = Buffer.from(buf).toString("base64");
  return `data:audio/mpeg;base64,${b64}`;
}
