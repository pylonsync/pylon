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

const FEED = "Phone photo shared on a photo app, candid and natural, true-to-life colors, no text, no logos, no watermarks, no borders.";
const AVATAR = "Casual profile photo for a photo-sharing app, head and shoulders, natural light, relaxed expression, simple background, no text.";
const PRODUCT = "Product photograph for a small home-goods shop: the item alone on a warm off-white linen surface, soft window light from the left, shallow depth of field, no text, no labels.";

/** Every image a template ships: path under public/, prompt, aspect. */
const IMAGES = [
  { template: "consumer", file: "images/avatars/mara.bakes.jpg", aspect: "1:1", quality: "medium", prompt: `${AVATAR} A woman in her thirties with blond hair tied back, flour on her apron, in a bright kitchen.` },
  { template: "consumer", file: "images/avatars/theo.climbs.jpg", aspect: "1:1", quality: "medium", prompt: `${AVATAR} A Black man in his twenties in a climbing gym, chalk on his hands, smiling.` },
  { template: "consumer", file: "images/avatars/ines.clay.jpg", aspect: "1:1", quality: "medium", prompt: `${AVATAR} A Spanish woman in her forties with dark curly hair in a pottery studio, clay on her fingers.` },
  { template: "consumer", file: "images/avatars/kenji.rides.jpg", aspect: "1:1", quality: "medium", prompt: `${AVATAR} A Japanese man in his thirties in a cycling cap outdoors, road in soft focus behind.` },
  { template: "consumer", file: "images/avatars/lou.streets.jpg", aspect: "1:1", quality: "medium", prompt: `${AVATAR} A woman in her twenties with short red hair holding a film camera on a city street.` },
  { template: "consumer", file: "images/avatars/sam.grows.jpg", aspect: "1:1", quality: "medium", prompt: `${AVATAR} A Nigerian-British man in his fifties with a grey beard in a vegetable garden, wearing a wool hat.` },
  { template: "consumer", file: "images/avatars/rosa.roasts.jpg", aspect: "1:1", quality: "medium", prompt: `${AVATAR} A Latina woman in her thirties in a coffee roastery, dark hair in a bandana.` },
  { template: "consumer", file: "images/avatars/biscuit.corgi.jpg", aspect: "1:1", quality: "medium", prompt: `${AVATAR} A red and white Pembroke Welsh corgi looking at the camera on a rug, tongue out.` },
  { template: "consumer", file: "images/avatars/nora.h.jpg", aspect: "1:1", quality: "medium", prompt: `${AVATAR} A woman in her thirties with glasses and a dark bob haircut, in a bright home office.` },
  { template: "consumer", file: "images/avatars/ari.wanders.jpg", aspect: "1:1", quality: "medium", prompt: `${AVATAR} A man in his thirties with a short beard and a backpack strap on his shoulder, at a train station.` },
  { template: "consumer", file: "images/avatars/dev.draws.jpg", aspect: "1:1", quality: "medium", prompt: `${AVATAR} An Indian man in his forties with glasses and a sketchbook, in an architecture studio.` },
  { template: "consumer", file: "images/avatars/june.surfs.jpg", aspect: "1:1", quality: "medium", prompt: `${AVATAR} A Japanese-American woman in her twenties in a wetsuit on a cold beach, wet hair.` },
  { template: "consumer", file: "images/posts/mara-loaf.jpg", aspect: "3:4", prompt: `A rustic sourdough loaf with a dramatic ear, on a linen cloth, morning window light. ${FEED}` },
  { template: "consumer", file: "images/posts/june-dawn.jpg", aspect: "3:4", prompt: `A surfer paddling out on glassy water at dawn, pink sky, cold Oregon coast. ${FEED}` },
  { template: "consumer", file: "images/posts/biscuit-couch.jpg", aspect: "1:1", prompt: `A corgi lying on a grey couch looking alert, afternoon sunlight. ${FEED}` },
  { template: "consumer", file: "images/posts/rosa-roaster.jpg", aspect: "1:1", prompt: `Coffee beans tumbling in the cooling tray of a small drum roaster. ${FEED}` },
  { template: "consumer", file: "images/posts/ines-glaze.jpg", aspect: "3:4", prompt: `A row of handmade stoneware mugs with a speckled oat glaze on a wooden shelf. ${FEED}` },
  { template: "consumer", file: "images/posts/lou-crosswalk.jpg", aspect: "3:4", prompt: `A Brooklyn street corner at golden hour, people crossing, shot on 35mm film with grain. ${FEED}` },
  { template: "consumer", file: "images/posts/dev-sketch.jpg", aspect: "1:1", prompt: `An open sketchbook with an ink drawing of a library staircase, a pen beside it. ${FEED}` },
  { template: "consumer", file: "images/posts/kenji-gravel.jpg", aspect: "3:4", prompt: `A gravel bike leaning on a fence on a dirt road through golden hills. ${FEED}` },
  { template: "consumer", file: "images/posts/sam-tomatoes.jpg", aspect: "1:1", prompt: `A basket of freshly picked red and yellow cherry tomatoes on an allotment bench. ${FEED}` },
  { template: "consumer", file: "images/posts/theo-boulder.jpg", aspect: "3:4", prompt: `A climber reaching for a hold on an indoor bouldering wall with blue holds. ${FEED}` },
  { template: "consumer", file: "images/posts/ari-train.jpg", aspect: "3:4", prompt: `View from a sleeper train window at sunrise over the Venetian lagoon. ${FEED}` },
  { template: "consumer", file: "images/posts/nora-desk.jpg", aspect: "1:1", prompt: `A tidy home desk with a laptop, a lamp, and a corgi sleeping on the rug below. ${FEED}` },
  { template: "consumer", file: "images/posts/mara-buns.jpg", aspect: "1:1", prompt: `Freshly baked cardamom buns on a baking tray, pearl sugar on top. ${FEED}` },
  { template: "consumer", file: "images/posts/june-board.jpg", aspect: "1:1", prompt: `A surfboard freshly waxed leaning against a van, overcast beach behind. ${FEED}` },
  { template: "consumer", file: "images/posts/rosa-latte.jpg", aspect: "1:1", prompt: `A latte with a rosetta in a ceramic cup, top-down, on a wooden counter. ${FEED}` },
  { template: "consumer", file: "images/posts/ines-wheel.jpg", aspect: "3:4", prompt: `Hands trimming the foot of a bowl on a pottery wheel, clay shavings. ${FEED}` },
  { template: "consumer", file: "images/posts/lou-diner.jpg", aspect: "1:1", prompt: `Diner counter seats with a coffee cup and a slice of pie, shot on film. ${FEED}` },
  { template: "consumer", file: "images/posts/biscuit-beach.jpg", aspect: "3:4", prompt: `A corgi running on a sandy beach, ears flapping, waves behind. ${FEED}` },
  { template: "consumer", file: "images/posts/dev-model.jpg", aspect: "1:1", prompt: `A white card architectural model of a small community building on a cutting mat. ${FEED}` },
  { template: "consumer", file: "images/posts/kenji-coffee.jpg", aspect: "1:1", prompt: `An espresso cup on a café table outdoors with a road bike leaning behind. ${FEED}` },
  { template: "consumer", file: "images/posts/sam-shed.jpg", aspect: "3:4", prompt: `Seed trays with green seedlings on a shelf inside a wooden garden shed. ${FEED}` },
  { template: "consumer", file: "images/posts/theo-chalk.jpg", aspect: "1:1", prompt: `A chalk bag, finger tape, and worn climbing shoes on a gym mat. ${FEED}` },
  { template: "consumer", file: "images/posts/ari-market.jpg", aspect: "1:1", prompt: `A market stall piled with lemons and blood oranges in Palermo, morning light. ${FEED}` },
  { template: "consumer", file: "images/posts/nora-sketches.jpg", aspect: "1:1", prompt: `Paper wireframe sketches for a mobile app spread on a desk with sticky notes. ${FEED}` },
  { template: "consumer", file: "images/posts/mara-starter.jpg", aspect: "3:4", prompt: `A glass jar of bubbly sourdough starter with a rubber band marking the rise. ${FEED}` },
  { template: "consumer", file: "images/posts/june-van.jpg", aspect: "1:1", prompt: `An old camper van parked on a bluff above the ocean, side door open. ${FEED}` },
  { template: "consumer", file: "images/posts/ines-studio.jpg", aspect: "3:4", prompt: `Pottery studio shelves full of bisque-fired bowls and glazed pieces. ${FEED}` },
  { template: "consumer", file: "images/posts/lou-rain.jpg", aspect: "3:4", prompt: `A rainy elevated subway platform at night with reflections, shot on film. ${FEED}` },
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

async function generate(tok, prompt, aspect, quality = "high") {
  const input = { prompt, aspect_ratio: aspect, quality, output_format: "jpeg", output_compression: 85 };
  if (process.env.OPENAI_API_KEY) input.openai_api_key = process.env.OPENAI_API_KEY;
  let prediction;
  for (let attempt = 0; ; attempt++) {
    const res = await fetch(`https://api.replicate.com/v1/models/${MODEL}/predictions`, {
      method: "POST",
      headers: { Authorization: `Bearer ${tok}`, "Content-Type": "application/json", Prefer: "wait=60" },
      body: JSON.stringify({ input }),
    });
    if (res.status === 429 && attempt < 12) {
      // Accounts under $5 of credit are limited to one request every ten seconds.
      const body = await res.json().catch(() => ({}));
      await new Promise((r) => setTimeout(r, ((body.retry_after ?? 10) + 1) * 1000));
      continue;
    }
    if (!res.ok) throw new Error(`replicate ${res.status}: ${(await res.text()).slice(0, 400)}`);
    prediction = await res.json();
    break;
  }
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
    const bytes = await generate(tok, item.prompt, item.aspect, item.quality);
    mkdirSync(dirname(out), { recursive: true });
    writeFileSync(out, bytes);
    console.log(`✓ ${item.template}/${item.file} ${(bytes.byteLength / 1024).toFixed(0)} KB`);
  } catch (e) {
    failed += 1;
    console.error(`✗ ${item.template}/${item.file}: ${e.message}`);
  }
}
process.exit(failed ? 1 : 0);
