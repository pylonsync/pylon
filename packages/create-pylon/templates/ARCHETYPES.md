# Landing-page archetype templates

Mast selects one of these `create-pylon` templates when it builds a site for a
new business. Each template includes a landing page, dashboard, and live
backend. Mast maps the business description to a template and fills its config.

## Principles

- **Each archetype defines a section structure and one realtime feature.** The
  generated site includes a working backend, and every template exposes one
  live interaction.
- **Shared foundation.** All templates share the brand/colors/seo config base
  (see the default template's `lib/site.config.ts`) and the marketing component
  kit (`WRAP`, `Eyebrow`, `SectionHead`, `FeatureGrid`, `Shot`, etc.). An
  archetype adds its own sections, config slots, and backend entities.
- **Customization happens in one `site.config.ts`.** Archetypes with lists,
  such as services or menus, may also include seed rows. This matches the
  default template's contract.
- **Privacy is part of the spec.** Landing pages are public. Any entity holding
  visitor PII (emails, phones) must deny client reads. Writes go through a
  server action. Pages may expose aggregates such as counts or non-PII fields
  such as busy time slots, but never customer emails.

## Shared config base
```ts
type BaseConfig = {
  brand: { name; letter; domain; email; footerBlurb; copyrightName; socials };
  colors: { brand; brandSoft; paper };
  seo: { title; description };
};
```
Each archetype below is `BaseConfig & { ...archetype slots }`.

## Mast classifier contract
Input: the business description. Output:
```ts
{ template: "waitlist" | "local-service" | "saas" | "restaurant" | "shop" | "creator" | "agency" | "marketplace" | "directory",
  siteConfig: <that template's typed config>,
  seed?: { services?: [...]; menu?: [...] }  // list rows for archetypes that have them
}
```
Use `waitlist` for ambiguous or pre-revenue businesses. Use the matching
archetype for a known operating business.

---

# 1. `waitlist`: pre-launch or coming soon

**Who:** validating demand, "I just started," pre-revenue, or fallback when the
business type is unclear.

**Sections (single page):**
1. Hero: badge ("Coming soon"), headline, subcopy, **email capture form + live
   signup counter** ("1,247 people waiting").
2. Value props: 3 items (what it is / who it's for / why now).
3. Social proof: optional logos or quotes.
4. FAQ: optional, short.
5. Footer.

**Realtime feature:** the signup counter updates on every open page. Submit in
one tab and the other increments without a refresh.

**Config (`WaitlistConfig`):**
```ts
BaseConfig & {
  hero: { badge; headline; subcopy; emailPlaceholder; ctaLabel; successMessage };
  counter: { enabled: boolean; label: string; seedCount?: number };
  valueProps: { eyebrow; headline; items: { title; body; icon? }[] };
  socialProof?: { label; logos?: string[]; quotes?: { quote; name; role }[] };
  faq?: { eyebrow; headline; items: { q; a }[] };
}
```

**Backend (`app.ts`):**
```ts
Signup = entity("Signup", {
  email: field.string().unique(),   // unique → dedupe
  createdAt: field.datetime(),
}, { indexes: [{ name: "by_email", fields: ["email"], unique: true }] });
```
- `joinWaitlist({ email })`: public action: validate + lowercase + dedupe +
  rate-limit, insert. Returns `{ ok, alreadyJoined? }`. Never returns other rows.
- `waitlistCount()`: query returning `count(Signup)`; the page live-subscribes
  so the number updates on every insert.
- **Privacy:** `Signup` policy denies ALL client read/write (like
  `cliAuthCodePolicy`). The page only ever sees the count, never emails.

**Dashboard:** total signups, signups-over-time chart, searchable list, CSV
export.

---

# 2. `local-service`: appointment businesses

**Who:** salon, barber, trades (plumber/electrician/cleaner), trainer, clinic,
studio, tutor — anyone who sells time slots.

**Sections:**
1. Hero: business name, tagline, **"Book now"** CTA, hero image, quick facts
   (hours / area / phone).
2. Services + prices: list/grid: name, duration, price.
3. **Booking**: pick service → pick a time from **live availability** → name /
   email / phone → confirm.
4. Reviews.
5. Hours + location: address, map embed, hours, contact.
6. Gallery: optional.
7. FAQ: optional. Footer.

**Realtime feature:** live slot availability. The time picker subscribes to the
day's bookings, so a booked slot greys out for everyone else. The server also
rechecks availability at insert time to prevent a race.

**Config (`LocalServiceConfig`):**
```ts
BaseConfig & {
  hero: { tagline; headline; subcopy; ctaLabel; heroImage?;
          quickFacts: { hours; area; phone } };
  services: { eyebrow; headline;
              items: { slug; name; durationMin; price; description? }[] };
  booking: { enabled; headline; slotMinutes;       // e.g. 30
             hours: { [day in 0..6]: { open; close } | null };  // weekly
             leadTimeHours?; confirmationMessage };
  reviews?: { eyebrow; headline; items: { quote; name; rating? }[] };
  location: { address; mapEmbedUrl?; hoursText; phone; email };
  gallery?: { images: string[] };
  faq?: { eyebrow; headline; items: { q; a }[] };
}
```

**Backend (`app.ts`):**
```ts
Service = entity("Service", {
  slug; name; durationMin: field.int(); priceCents: field.int();
  description: field.string().optional(); active: field.bool();
});
Booking = entity("Booking", {
  serviceId: field.id("Service");
  startsAt: field.datetime(); endsAt: field.datetime();
  customerName; customerEmail; customerPhone: field.string().optional();
  status: field.string();   // "confirmed" | "cancelled"
  createdAt: field.datetime();
}, { indexes: [{ name: "by_start", fields: ["startsAt"] }] });
```
- `createBooking({ serviceId, startsAt, customer })`: public action: server-side
  re-check that the slot is still free (overlap query) before insert, to close
  the race the live UI already mostly prevents.
- `bookedSlotsForRange({ from, to })`: query returning only `{ startsAt, endsAt }`
  (NO customer fields); the picker subscribes and computes free slots = hours −
  booked. **Privacy:** `Booking` denies client read of full rows; only this
  PII-stripped projection is exposed. `Service` is public-read (it's menu data).

**Dashboard:** day/week calendar of bookings, confirm/cancel, manage services +
weekly hours.

---

# 3. `saas`: app, tool, or digital product

**Who:** software founders ("I'm building an app").
**Sections:** hero + dashboard preview, logo cloud, feature sections
(products), pricing tiers, testimonials, FAQ. (Already built; this is the
refactored `default` template; rename `default` → `saas`, keep `default` as an
alias.)
**Realtime feature:** the live dashboard behind "Open dashboard" (the workspace
itself).
**Config:** the existing `SiteConfig` (`lib/site.config.ts`).

---

# 4. `restaurant`: food and hospitality

**Who:** restaurant, cafe, bar, food truck, bakery.
**Sections:** hero (name + "Reserve" / "Order"), **menu** (sections → items with
price/description), hours + location + map, reservations or order-ahead, gallery,
reviews, footer.
**Realtime feature:** live table/reservation availability (same engine as
local-service booking) OR live order-ahead status board.
**Config highlights:** `menu: { sections: { name; items: { name; price; desc?;
tags? }[] }[] }`, `reservations` (reuse booking shape), `location`, `hours`.
**Entities:** `MenuItem` (public read), `Reservation` (PII-private, like
`Booking`).

---

# 5. `shop`: DTC product or small store

**Who:** product brands, makers, single-product or small-catalog sellers.
**Sections:** hero, featured products / product grid, value props, reviews,
shipping & returns, footer; cart.
**Realtime feature:** live inventory (stock count updates as orders land) + live
cart count.
**Config highlights:** `products: { items: { slug; name; priceCents; image;
description; stock? }[] }`, `valueProps`, `policies` (shipping/returns text).
**Entities:** `Product` (public read, live stock), `Order` (PII-private; cart
lines share an `orderGroupId`).
**Checkout:** real **Stripe Checkout** via `@pylonsync/stripe`'s `stripeRequest`
+ `verifyStripeSignature` (one-time `price_data` line items; signed webhook at
`/api/webhooks/stripeWebhook` settles paid / releases held stock on expiry).
Stock is held under a per-product advisory lock at checkout so the cart cannot
oversell. Without `STRIPE_SECRET_KEY`, the order remains `reserved` for the
owner to follow up. This keeps the live-inventory demo usable without Stripe.

---

# 6. `creator`: personal brand, coach, or consultant

**Who:** solo creators, coaches, consultants, freelancers, newsletter authors.
**Sections:** hero (you), about, offerings/services, portfolio or testimonials,
**lead magnet / newsletter signup**, book-a-call, links, footer.
**Realtime feature:** live newsletter signup count (reuse the `waitlist`
`Signup` + counter) and/or live call-booking availability (reuse `local-service`
booking).
**Config highlights:** `profile: { name; tagline; photo; bio }`, `offerings`,
`portfolio?`, `newsletter` (reuse waitlist counter), `booking?` (reuse
local-service).
**Entities:** `Signup` and/or `Booking`, reused from the other archetypes.

---

# 7. `agency`: design, development, or marketing studio

**Who:** boutique product/design/dev/marketing studios that take on a limited
number of clients at once.
**Sections:** hero (+ live availability pill), logo cloud, services, **work /
case-study grid** (marked project-shot placeholders), process, **team** (marked
headshot placeholders), testimonials, **project inquiry form** (#contact), footer.
**Realtime feature:** a public `Capacity` row holds the booking window
+ open project slots; the hero pill (`db.useQuery("Capacity")`) shows "N slots
open" live, and the owner booking a lead from the dashboard decrements it for
everyone instantly (same shape as `shop` inventory).
**Config highlights:** `hero`, `capacity: { label; openSlots }` (seed), `logos`,
`services`, `work: CaseStudy[]`, `process`, `team: TeamMember[]`, `testimonials?`,
`contact: { projectTypes[]; budgets[]; confirmationMessage }`.
**Entities:** `Inquiry` (PII deny-all: name/email/company/budget/message + status),
`Capacity` (public-read single row, live openSlots), `User`.
**Placeholders:** the hero photo, each case-study project shot, and each team
headshot use the shared `ImagePlaceholder`.

---

# 8. `marketplace`: two-sided buy and sell platform

**Who:** local/vertical marketplaces, classifieds, gear resale, any buyer↔seller
platform.
**Sections:** SSR browse grid + category facets (`/`), listing detail (`/listing/:slug`),
sell form (`/sell`, sign-in gated), per-user inbox (`/me`).
**Realtime feature:** a live "just listed" ticker + live offers on a listing +
your `/me` inbox, all via `db.useQuery` over public `Listing`/`Offer` (and
private `Watch`). List in one tab → it appears in another instantly.
**Entities:** `User`, `Listing` (`sellerId: field.owner()`, slug, price,
category, condition, status, seed-gradient photo), `Offer` (`buyerId:
field.owner()`, amount, status), `Watch` (private). Public-read listings/offers,
owner-scoped writes; accept-marks-sold-and-declines-siblings in
`respondToOffer`. It is multi-user through email/password, unlike the
single-owner archetypes above.
**Note:** ported from `examples/market`. Rebrand it in
`app/layout.tsx` and `functions/seedMarket.ts`. Listing photos are generated
gradients; swap for real `<img>` + `/api/files` upload for production.

---

# 9. `directory`: curated, searchable listing site

**Who:** "best X" lists, tool/company/local directories, awesome-lists with a UI.
**Sections:** hero + a live faceted-search browse (`#browse`), a `/submit` page,
a `/dashboard` moderation queue.
**Realtime feature:** Pylon FTS and facets. `db.useSearch`
re-runs on every keystroke AND on every write, so it doubles as the live layer;
public `upvote` bumps `Listing.votes` and the count ticks up across all tabs.
**Entities:** `Listing` (public-read, no PII; `search: { text, facets, sortable }`
→ FTS5 + facet shadow tables) + `Submission` (deny-all PII: submitter
name/email + proposed entry, status) + `User`. submitListing/upvote public;
submissionsForOwner/approveSubmission/rejectSubmission owner-gated;
approveSubmission copies ONLY public fields into a Listing (PII never published).
**Key API:** `db.useSearch<T>(entity, { query, filters, facets, sort:[field,dir],
page, pageSize })` → `{ hits, facetCounts, total, loading }` (reference:
examples/store/client/Catalog.tsx).

---

# 10. `ai-chat`: streaming AI assistant

**Who:** any LLM chat product / internal assistant.
**Sections:** a full-screen chat with a conversation sidebar, streaming thread,
composer (`/`), optional `/login`.
**Realtime feature:** token streaming from the built-in `POST /api/ai/stream`
(SSE; the API key never leaves the server), PLUS owner-scoped `Conversation` +
`Message` synced across the user's tabs via `db.useQuery`.
**Entities:** `Conversation` + `Message` (both `userId: field.owner()`,
owner-scoped read/write, private per user; messages written with optimistic
`db.insert`, no custom functions) + `User`. Multi-user (guest or signed-in).
**Config:** `PYLON_AI_PROVIDER` + `PYLON_AI_API_KEY` + `PYLON_AI_MODEL` enable
replies; without them the app boots and shows a friendly "configure AI" notice.
**SSE contract:** `data: {"choices":[{"delta":{"content":"…"}}]}` … `data:
[DONE]`; 503 `AI_NOT_CONFIGURED` / 429 `RATE_LIMITED` shims handled in the client.

---

# 11. `ai-studio`: generative media studio

**Who:** AI image/audio/video generators, creative tools.
**Sections:** a prompt bar + medium selector over a live gallery (`/`), optional
`/login`.
**Realtime feature:** the generation gallery. The `generate` action inserts a
`pending` Generation (card appears instantly), runs the provider call
server-side, then flips the row to `done`/`failed`; `db.useQuery` syncs that to
every open tab so the card updates live.
**Entities:** `Generation` (owner-scoped READ, `allowInsert:"false"`; only the
server pipeline writes it) + `User`. Multi-user (guest or signed-in).
**Functions:** `generate` (public action) brackets the provider call with
internal `_createGeneration` / `_finishGeneration` mutations. Image uses OpenAI
Images, audio uses OpenAI TTS when `OPENAI_API_KEY` is set, and video is a
labeled extension point. Without a key, the app returns a clearly labeled
placeholder so the flow and gallery remain testable. Keys and media stay
server-side.
**Note:** image uses the provider's hosted URL (small to sync, ~1h TTL);
persist via `/api/files` for permanence. lib/studio.ts holds the placeholder gen.

---

## Build order
1. **`waitlist`:** smallest template and current dogfood target (`Signup`,
   counter, and `joinWaitlist`).
2. **`local-service`:** booking template with `Service`, `Booking`, and live
   availability.
3. **`saas`:** rename and finish the refactored default.
4. **`restaurant`**, **`shop`**, **`creator`:** each heavily reuses the
   `booking`/`signup` engines above, so they're mostly config + a couple of
   bespoke sections.

## Reuse map (so 6 archetypes aren't 6 from-scratch builds)
- `Signup` + live counter → waitlist, creator (newsletter).
- `Booking` + live availability → local-service, restaurant (reservations),
  creator (call booking).
- `Product` grid + inventory → shop.
- Marketing component kit + BaseConfig → all.
The signup, booking, and product engines cover every archetype above.
