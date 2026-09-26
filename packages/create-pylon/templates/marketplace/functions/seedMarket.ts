import { mutation, v } from "@pylonsync/functions";

// Demo catalog for first boot. The client bootstrap calls this twice: once as
// the "bazaar" account for DEMO[0..7] and once as the demo shopper for the
// rest, so the shopper has other people's listings to buy and a few of its
// own to receive offers on. Idempotent per caller: a seller that already has
// listings is skipped.
const DEMO: Array<{
  seller: string;
  location: string;
  title: string;
  description: string;
  price: number;
  category: string;
  condition: string;
  photo: string;
  seed: string;
}> = [
  {
    seller: "Nora Lind",
    location: "Brooklyn, NY",
    title: "Danish teak lounge chair, oatmeal wool",
    description:
      "Solid teak frame with the original joinery, oiled last spring. Cushions were reupholstered in a heavy oatmeal wool bouclé. No wobble, no cracks. Seat height 16 in, width 27 in. Pickup in Greenpoint or I can help arrange a courier.",
    price: 680,
    category: "furniture",
    condition: "good",
    photo: "lounge-chair",
    seed: "a1f3",
  },
  {
    seller: "Marcus Bell",
    location: "Dallas, TX",
    title: "1960s steel dress watch, cream dial",
    description:
      "34 mm stainless case, hand-wound movement serviced in March. Keeps about +6 s/day. Cream dial has light, even patina. Comes on a brown suede strap with a spare black leather strap.",
    price: 890,
    category: "watches",
    condition: "good",
    photo: "watch",
    seed: "b7c2",
  },
  {
    seller: "Theo Park",
    location: "Portland, OR",
    title: "Chrome 35mm SLR with 50mm f/1.7",
    description:
      "Fully mechanical body, shutter fires at every speed and sounds right. Light meter reads within a third of a stop. Lens is clean with no haze or fungus. Original leather strap. Tested with a roll of Portra; sample scans on request.",
    price: 240,
    category: "cameras",
    condition: "good",
    photo: "film-camera",
    seed: "c4e9",
  },
  {
    seller: "Ava Moreno",
    location: "Austin, TX",
    title: "Walnut direct-drive turntable",
    description:
      "Direct-drive deck in a solid walnut plinth. Speed is stable at 33 and 45, pitch control works. New belt-free motor service, new cartridge with about 40 hours on it. Dust cover not included.",
    price: 410,
    category: "audio",
    condition: "like-new",
    photo: "turntable",
    seed: "d8a1",
  },
  {
    seller: "Nora Lind",
    location: "Brooklyn, NY",
    title: "Cognac leather weekender",
    description:
      "Full-grain leather duffel with solid brass hardware and a detachable shoulder strap. Carried on about ten trips; the leather has softened and darkened evenly. Interior lining is clean. 20 x 11 x 10 in.",
    price: 295,
    category: "bags",
    condition: "good",
    photo: "weekender-bag",
    seed: "a9c4",
  },
  {
    seller: "Elise Carter",
    location: "Asheville, NC",
    title: "Speckled stoneware table lamp",
    description:
      "Hand-thrown stoneware base in a sand glaze, pleated linen shade. Rewired with a cloth cord and inline switch. 22 in tall with the shade. One tiny glaze pop near the foot, shown in photos.",
    price: 165,
    category: "lighting",
    condition: "like-new",
    photo: "ceramic-lamp",
    seed: "b3f8",
  },
  {
    seller: "Elise Carter",
    location: "Asheville, NC",
    title: "Tufted slipper chair, ivory",
    description:
      "Small button-tufted slipper chair in ivory cotton velvet on turned white legs. Frame is tight, fabric has no stains or pulls. Seat height 17 in, width 24 in. Works in a bedroom or reading corner.",
    price: 240,
    category: "furniture",
    condition: "like-new",
    photo: "slipper-chair",
    seed: "m5t1",
  },
  {
    seller: "Sam Rivera",
    location: "Denver, CO",
    title: "Two-door oak wardrobe",
    description:
      "Solid oak wardrobe with two doors over two deep drawers. Hanging rail and one shelf inside. Doors close flush, drawers run smoothly. 40 in wide, 22 in deep, 74 in tall. Comes apart at the top for moving.",
    price: 450,
    category: "furniture",
    condition: "good",
    photo: "wardrobe",
    seed: "n8w3",
  },
  {
    seller: "Sam Rivera",
    location: "Denver, CO",
    title: "Carbon road bike, 54 cm, matte black",
    description:
      "Carbon frame and fork, 54 cm, matte black. 2x11 drivetrain shifts cleanly, new chain and brake pads this season. No cracks or impact damage. About 8.4 kg as pictured.",
    price: 1250,
    category: "bikes",
    condition: "good",
    photo: "road-bike",
    seed: "e2b6",
  },
  {
    seller: "Sam Rivera",
    location: "Denver, CO",
    title: "Black travel duffel, 55 L",
    description:
      "Water-resistant nylon duffel with padded shoulder straps and a separate shoe pocket. Used on two trips. All zips and buckles work. Packs flat into its own pocket.",
    price: 70,
    category: "bags",
    condition: "like-new",
    photo: "duffel",
    seed: "p2d6",
  },
];

interface SeedMarketArgs {
  start?: number;
  end?: number;
}

interface SeedMarketResult {
  seeded: number;
}

function slugify(s: string): string {
  return s
    .toLowerCase()
    .trim()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 60);
}

export default mutation<SeedMarketArgs, SeedMarketResult>({
  // auth defaults to "user": every seeded listing is owned by the caller.
  args: {
    start: v.optional(v.number()),
    end: v.optional(v.number()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "sign in first");

    const all = (await ctx.db.list("Listing")) as Array<{ sellerId: string }>;
    if (all.some((l) => l.sellerId === ctx.auth.userId)) return { seeded: 0 };

    const start = args.start ?? 0;
    const end = args.end ?? DEMO.length;
    const slice = DEMO.slice(start, end);

    // Stagger createdAt so "newest first" follows the array order. The
    // seller id is the caller; `sellerName` is a display name only.
    const now = Date.now();
    let n = 0;
    for (const d of slice) {
      await ctx.db.insert("Listing", {
        sellerId: ctx.auth.userId,
        sellerName: d.seller,
        location: d.location,
        title: d.title,
        slug: `${slugify(d.title) || "item"}-${d.seed}`,
        description: d.description,
        price: d.price,
        category: d.category,
        condition: d.condition,
        status: "active",
        imageUrl: `/images/listings/${d.photo}.webp`,
        seed: d.seed,
        createdAt: new Date(now - ((start + n) * 23 + 4) * 60_000).toISOString(),
      });
      n++;
    }
    return { seeded: n };
  },
});
