# Auction House — statecraft example

A timed-auction marketplace. Demonstrates:

- **Transactional bids** — `placeBid.ts` reads the listing, validates the bid
  amount, debits checks, and updates the winning bid in one atomic mutation.
  If any check fails, nothing is written.
- **Scheduled settlement** — `createListing.ts` calls
  `ctx.scheduler.runAt(endsAt, "settleListing", ...)`. When the auction ends,
  the scheduler invokes `settleListing.ts`, which transfers funds from
  winner to seller atomically.
- **Live UI** — the React `ListingDetail` polls every 2 seconds via `useFn`
  with `refetchIntervalMs`. (Swap for `useShard` if you need sub-second.)
- **Policies** — only authenticated users can bid; only admins can call
  `settleListing` directly (the scheduler runs as system).

## Run it

```sh
cd examples/auction-house
statecraft dev
```

Open http://localhost:4321/studio to inspect entities.

## What to read first

| File | Why |
|---|---|
| `statecraft.manifest.json` | The data model and what's exposed |
| `functions/placeBid.ts` | The bidding transaction — every check inline |
| `functions/settleListing.ts` | Atomic balance transfer |
| `functions/createListing.ts` | Scheduling future work |
| `client/AuctionPage.tsx` | React with `useFn` and `db.useMutation` |

## Things this example does NOT do

- No payment integration (bids debit fake `balanceCents`)
- No image uploads on listings (use `db.uploadFile` for that)
- No anti-sniping extension (could be added in `placeBid` by extending
  `endsAt` and rescheduling settlement)
- No bidder notifications
