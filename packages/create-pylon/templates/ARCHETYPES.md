# Landing-page archetype templates

Default templates Mast picks from when it builds a site for a new business. Each
is a real `create-pylon` template (landing + dashboard + a live backend), so
they're useful to every Pylon user too — Mast just adds a classifier that maps a
business description to one of these and fills its config.

## Principles
- **An archetype is a section structure + one real realtime feature, not just
  copy/colors.** Anyone can have an LLM emit a static landing page now. The edge
  is that the generated site has a live backend doing something. Every template
  ships with exactly one genuinely-realtime hook.
- **Shared foundation.** All templates share the brand/colors/seo config base
  (see the default template's `lib/site.config.ts`) and the marketing component
  kit (`WRAP`, `Eyebrow`, `SectionHead`, `FeatureGrid`, `Shot`, etc.). An
  archetype adds its own sections, config slots, and backend entities.
- **Customization = generate one `site.config.ts`** (plus optional seed rows for
  archetypes with lists, e.g. services/menu). Same contract as the default.
- **Privacy is part of the spec.** Landing pages are public. Any entity holding
  visitor PII (emails, phones) must deny client reads — writes go through a
  server action, and only aggregates (a count) or non-PII fields (busy time
  slots) are exposed to the page. Never let a marketing site leak its own
  customers' emails.

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
{ template: "waitlist" | "local-service" | "saas" | "restaurant" | "shop" | "creator" | "agency",
  siteConfig: <that template's typed config>,
  seed?: { services?: [...]; menu?: [...] }  // list rows for archetypes that have them
}
```
Ambiguous or clearly pre-revenue → `waitlist` (safe universal default). Known
operating business → the matching archetype.

---

# 1. `waitlist` — pre-launch / coming-soon  ★ build first (dogfood target)

**Who:** validating demand, "I just started," pre-revenue, or fallback when the
business type is unclear.

**Sections (single page):**
1. Hero — badge ("Coming soon"), headline, subcopy, **email capture form + live
   signup counter** ("1,247 people waiting").
2. Value props — 3 items (what it is / who it's for / why now).
3. Social proof — optional logos or quotes.
4. FAQ — optional, short.
5. Footer.

**Realtime feature:** the signup counter ticks up for everyone with the page
open. Open two tabs, submit in one, the other increments without refresh. That's
the whole "it's a real live app" proof.

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
- `joinWaitlist({ email })` — public action: validate + lowercase + dedupe +
  rate-limit, insert. Returns `{ ok, alreadyJoined? }`. Never returns other rows.
- `waitlistCount()` — query returning `count(Signup)`; the page live-subscribes
  so the number updates on every insert.
- **Privacy:** `Signup` policy denies ALL client read/write (like
  `cliAuthCodePolicy`). The page only ever sees the count, never emails.

**Dashboard:** total signups, signups-over-time chart, searchable list, CSV
export.

---

# 2. `local-service` — appointment businesses (booking)  ★ build second

**Who:** salon, barber, trades (plumber/electrician/cleaner), trainer, clinic,
studio, tutor — anyone who sells time slots.

**Sections:**
1. Hero — business name, tagline, **"Book now"** CTA, hero image, quick facts
   (hours / area / phone).
2. Services + prices — list/grid: name, duration, price.
3. **Booking** — pick service → pick a time from **live availability** → name /
   email / phone → confirm.
4. Reviews.
5. Hours + location — address, map embed, hours, contact.
6. Gallery — optional.
7. FAQ — optional. Footer.

**Realtime feature:** live slot availability. The time picker subscribes to the
day's bookings; the moment someone books a slot it greys out for everyone else —
no double-booking. Server also re-checks at insert time to close the race.

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
- `createBooking({ serviceId, startsAt, customer })` — public action: server-side
  re-check that the slot is still free (overlap query) before insert, to close
  the race the live UI already mostly prevents.
- `bookedSlotsForRange({ from, to })` — query returning only `{ startsAt, endsAt }`
  (NO customer fields); the picker subscribes and computes free slots = hours −
  booked. **Privacy:** `Booking` denies client read of full rows; only this
  PII-stripped projection is exposed. `Service` is public-read (it's menu data).

**Dashboard:** day/week calendar of bookings, confirm/cancel, manage services +
weekly hours.

---

# 3. `saas` — app / tool / digital product  (≈ the current default, renamed)

**Who:** software founders ("I'm building an app").
**Sections:** hero + dashboard preview, logo cloud, feature sections
(products), pricing tiers, testimonials, FAQ. (Already built — this is the
refactored `default` template; rename `default` → `saas`, keep `default` as an
alias.)
**Realtime feature:** the live dashboard behind "Open dashboard" (the workspace
itself).
**Config:** the existing `SiteConfig` (`lib/site.config.ts`).

---

# 4. `restaurant` — food & hospitality

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

# 5. `shop` — DTC product / small store

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
Stock is held under a per-product advisory lock at checkout so the cart can't
oversell. **Graceful degradation:** with no `STRIPE_SECRET_KEY` the order is held
as `reserved` for the owner to follow up — the store boots and demos live
inventory with zero config.

---

# 6. `creator` — personal brand / coach / consultant

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

# 7. `agency` — design / dev / marketing studio

**Who:** boutique product/design/dev/marketing studios that take on a limited
number of clients at once.
**Sections:** hero (+ live availability pill), logo cloud, services, **work /
case-study grid** (marked project-shot placeholders), process, **team** (marked
headshot placeholders), testimonials, **project inquiry form** (#contact), footer.
**Realtime feature:** scarcity — a public `Capacity` row holds the booking window
+ open project slots; the hero pill (`db.useQuery("Capacity")`) shows "N slots
open" live, and the owner booking a lead from the dashboard decrements it for
everyone instantly (same shape as `shop` inventory).
**Config highlights:** `hero`, `capacity: { label; openSlots }` (seed), `logos`,
`services`, `work: CaseStudy[]`, `process`, `team: TeamMember[]`, `testimonials?`,
`contact: { projectTypes[]; budgets[]; confirmationMessage }`.
**Entities:** `Inquiry` (PII deny-all: name/email/company/budget/message + status),
`Capacity` (public-read single row, live openSlots), `User`.
**Placeholders:** hero photo, each case-study project shot, each team headshot —
all clearly marked via the shared `ImagePlaceholder`.

---

## Build order
1. **`waitlist`** — smallest, proves the whole pipeline, already the dogfood
   target. (`Signup` + counter + `joinWaitlist`.)
2. **`local-service`** — biggest SMB market; booking is the strongest realtime
   demo. (`Service` + `Booking` + live availability.)
3. **`saas`** — rename/finish the refactored default.
4. **`restaurant`**, **`shop`**, **`creator`** — breadth; each heavily reuses the
   `booking`/`signup` engines above, so they're mostly config + a couple of
   bespoke sections.

## Reuse map (so 6 archetypes aren't 6 from-scratch builds)
- `Signup` + live counter → waitlist, creator (newsletter).
- `Booking` + live availability → local-service, restaurant (reservations),
  creator (call booking).
- `Product` grid + inventory → shop.
- Marketing component kit + BaseConfig → all.
Two real backend engines (signup, booking) + one (product) cover every archetype.
