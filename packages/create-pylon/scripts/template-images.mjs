#!/usr/bin/env node
/**
 * Generate the images the marketing templates ship, with gpt-image-2 on
 * Replicate. Writes templates/<template>/public/images/... as JPEG.
 *
 *   node scripts/template-images.mjs             # everything missing
 *   node scripts/template-images.mjs --force     # regenerate all
 *   node scripts/template-images.mjs restaurant  # one template
 *
 * Token: REPLICATE_API_TOKEN, or the file ~/.config/replicate/token.
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { homedir } from "node:os";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const TEMPLATES = resolve(HERE, "../templates");
const MODEL = process.env.REPLICATE_IMAGE_MODEL || "openai/gpt-image-2";

const STYLE = "Editorial photograph, natural light, shallow depth of field, muted warm tones, no text, no logos, no watermarks.";
const HEADSHOT = "Professional headshot, neutral studio background, soft light, looking at the camera, natural expression, shoulders up, no text.";
const UI = "A clean product screenshot of a modern web app on a laptop screen, photographed at a slight angle on a desk, soft daylight, no readable text.";

const PRODUCT = "Product photograph for a small home-goods shop: the item alone on a warm off-white linen surface, soft window light from the left, shallow depth of field, no text, no labels.";

/** Every image a template ships: path under public/, prompt, aspect. */
const IMAGES = [
  { template: "shop", file: "images/products/cedar-smoke.jpg", aspect: "4:3", prompt: `A hand-poured soy candle in a dark amber glass jar, unlit, a sprig of cedar beside it. ${PRODUCT}` },
  { template: "shop", file: "images/products/linen.jpg", aspect: "4:3", prompt: `A hand-poured soy candle in a frosted white glass jar, unlit, folded white linen behind it. ${PRODUCT}` },
  { template: "shop", file: "images/products/fig-leaf.jpg", aspect: "4:3", prompt: `A hand-poured soy candle in a pale green glass jar, unlit, a fig leaf beside it. ${PRODUCT}` },
  { template: "shop", file: "images/products/tobacco-oak.jpg", aspect: "4:3", prompt: `A hand-poured soy candle in a dark brown glass jar with a wooden lid, unlit, on an oak board. ${PRODUCT}` },
  { template: "shop", file: "images/products/sea-salt.jpg", aspect: "4:3", prompt: `A hand-poured soy candle in a sea-blue glass jar, unlit, a small piece of driftwood beside it. ${PRODUCT}` },
  { template: "shop", file: "images/products/ceramic-vessel.jpg", aspect: "4:3", prompt: `A candle in a hand-thrown speckled stoneware vessel, unlit, matte glaze. ${PRODUCT}` },
  { template: "shop", file: "images/products/travel-tin.jpg", aspect: "4:3", prompt: `A small candle in a brushed metal travel tin with the lid set beside it, unlit. ${PRODUCT}` },
  { template: "shop", file: "images/products/brass-holder.jpg", aspect: "4:3", prompt: `A small solid brass cylinder holding long matches, striker ring on the base. ${PRODUCT}` },
  { template: "shop", file: "images/products/wick-trimmer.jpg", aspect: "4:3", prompt: `A matte black steel wick trimmer lying beside a candle. ${PRODUCT}` },
  { template: "shop", file: "images/products/snuffer.jpg", aspect: "4:3", prompt: `A brass candle snuffer with a long handle and a hinged bell, resting beside a candle. ${PRODUCT}` },
  { template: "shop", file: "images/products/gift-set.jpg", aspect: "4:3", prompt: `Three small candles in a kraft gift box with the lid off, tied with cotton ribbon. ${PRODUCT}` },
  { template: "shop", file: "images/products/match-jar.jpg", aspect: "4:3", prompt: `A glass apothecary jar full of long matches with a cork lid, a striker strip on the side. ${PRODUCT}` },
  { template: "restaurant", file: "images/hero.jpg", aspect: "16:9", prompt: `Dining room of a small neighbourhood bistro at night: dark walls, a wood-fired oven glowing at the back, candlelit tables with linen, a few guests blurred, moody and warm. ${STYLE}` },
  { template: "restaurant", file: "images/hearth.jpg", aspect: "4:3", prompt: `Close-up of a wood-fired hearth in a restaurant kitchen, embers glowing, a cast-iron pan at the edge, dark background. ${STYLE}` },
  { template: "restaurant", file: "images/branzino.jpg", aspect: "4:3", prompt: `A whole wood-fired branzino on a dark ceramic plate with charred fennel, citrus and salsa verde, on a dark wooden table, candlelight. ${STYLE}` },
  { template: "restaurant", file: "images/bar.jpg", aspect: "4:3", prompt: `A small restaurant bar at night: backlit bottles, a marble counter, two stools, a bartender's hands pouring wine, dark and warm. ${STYLE}` },
  { template: "local-service", file: "images/hero.jpg", aspect: "3:4", prompt: `Interior of a bright, tidy barbershop with leather chairs and a big window, mid-morning light, a barber at work blurred. ${STYLE}` },
  { template: "creator", file: "images/headshot.jpg", aspect: "1:1", prompt: `${HEADSHOT} A woman in her thirties with a friendly, confident look, a simple dark sweater.` },
  { template: "agency", file: "images/hero.jpg", aspect: "3:4", prompt: `A small design studio at work: two people at a long oak table with laptops and printed wireframes, plants, large window. ${STYLE}` },
  { template: "agency", file: "images/team/claire-donovan.jpg", aspect: "1:1", prompt: `${HEADSHOT} A woman in her forties, short grey-streaked hair, calm and direct.` },
  { template: "agency", file: "images/team/marcus-lee.jpg", aspect: "1:1", prompt: `${HEADSHOT} A man in his thirties of East Asian descent, glasses, dark shirt, slight smile.` },
  { template: "agency", file: "images/team/dana-okafor.jpg", aspect: "1:1", prompt: `${HEADSHOT} A Black woman in her thirties, natural hair, warm expression, rust-coloured top.` },
  { template: "agency", file: "images/work/ledger.jpg", aspect: "4:3", prompt: `${UI} The app is a finance ledger with a table and a small chart in deep green and off-white.` },
  { template: "agency", file: "images/work/atlas-health.jpg", aspect: "4:3", prompt: `${UI} The app is a patient portal with appointment cards, calm blue and white.` },
  { template: "agency", file: "images/work/cohort.jpg", aspect: "4:3", prompt: `${UI} The app is a community learning platform with course cards, warm orange accents.` },
  { template: "agency", file: "images/work/vela.jpg", aspect: "4:3", prompt: `${UI} The app is a sailing-club booking tool with a map and a schedule, navy and sand tones.` },
];

const force = process.argv.includes("--force");
const only = process.argv.slice(2).filter((a) => !a.startsWith("--"));

function token() {
  if (process.env.REPLICATE_API_TOKEN) return process.env.REPLICATE_API_TOKEN;
  const f = join(homedir(), ".config/replicate/token");
  if (existsSync(f)) return readFileSync(f, "utf8").trim();
  throw new Error("No Replicate token: set REPLICATE_API_TOKEN or write ~/.config/replicate/token");
}

async function generate(tok, prompt, aspect) {
  const input = { prompt, aspect_ratio: aspect, quality: "high", output_format: "jpeg", output_compression: 85 };
  if (process.env.OPENAI_API_KEY) input.openai_api_key = process.env.OPENAI_API_KEY;
  const res = await fetch(`https://api.replicate.com/v1/models/${MODEL}/predictions`, {
    method: "POST",
    headers: { Authorization: `Bearer ${tok}`, "Content-Type": "application/json", Prefer: "wait=60" },
    body: JSON.stringify({ input }),
  });
  if (!res.ok) throw new Error(`replicate ${res.status}: ${(await res.text()).slice(0, 400)}`);
  let prediction = await res.json();
  for (let i = 0; i < 240 && prediction.status !== "succeeded" && prediction.status !== "failed" && prediction.status !== "canceled"; i++) {
    await new Promise((r) => setTimeout(r, 2000));
    const poll = await fetch(`https://api.replicate.com/v1/predictions/${prediction.id}`, { headers: { Authorization: `Bearer ${tok}` } });
    prediction = await poll.json();
  }
  if (prediction.status !== "succeeded") throw new Error(`prediction ${prediction.status}: ${prediction.error ?? ""}`);
  const url = Array.isArray(prediction.output) ? prediction.output[0] : prediction.output;
  const img = await fetch(url);
  if (!img.ok) throw new Error(`download ${img.status}`);
  return new Uint8Array(await img.arrayBuffer());
}

const tok = token();
let failed = 0;
for (const item of IMAGES) {
  if (only.length && !only.includes(item.template)) continue;
  const out = join(TEMPLATES, item.template, "public", item.file);
  if (existsSync(out) && !force) {
    console.log(`= ${item.template}/${item.file} (exists)`);
    continue;
  }
  try {
    const bytes = await generate(tok, item.prompt, item.aspect);
    mkdirSync(dirname(out), { recursive: true });
    writeFileSync(out, bytes);
    console.log(`✓ ${item.template}/${item.file} ${(bytes.byteLength / 1024).toFixed(0)} KB`);
  } catch (e) {
    failed += 1;
    console.error(`✗ ${item.template}/${item.file}: ${e.message}`);
  }
}
process.exit(failed ? 1 : 0);
