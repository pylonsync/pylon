# Auction House — pylon example

A real-time auction platform with two sale formats. Demonstrates:

- **Transactional bids**: `placeBid.ts` reads the lot, validates the bid
  amount against the current price + minimum increment, checks balance, and
  writes the Bid row + updates the lot atomically. Any check failure is a
  no-op.
- **Scheduled settlement**: timed lots close automatically via a recurring
  `sweepTimedLots` scheduler that walks open lots every 2 seconds and
  closes expired ones, awarding the winner.
- **Antishill timer**: live auction lots get a sliding 12-second deadline
  that resets on every fresh bid, preventing last-second sniping.
- **Live UI**: `db.useQuery` keeps bid feeds and lot state live with
  zero app-specific WebSocket code.
- **Per-user watchlist**: optimistic `db.insert` / `db.delete` on the
  `Watch` entity, scoped by `watch_owner` policy.
- **Self-seeding**: `seedAuctionHouse.ts` populates sample auctions
  on first launch; idempotent on subsequent visits.

## Run it

```sh
cd examples/auction-house
pylon dev
```

Open http://localhost:4321 to browse auctions and bid.
Open http://localhost:4321/studio to inspect entities.

## What to read first

| File | Why |
|---|---|
| `app.ts` | Entity schema + policies |
| `functions/placeBid.ts` | Atomic bid validation and write |
| `functions/createAuction.ts` | Auction + lot creation, scheduler kickoff |
| `functions/sweepTimedLots.ts` | Recurring timed-lot close loop |
| `functions/openLot.ts` | Live auctioneer control (antishill timer) |
| `functions/endAuction.ts` | Auctioneer ends the live session |
| `client/AuctionApp.tsx` | Shell + hash routing |
| `client/LotDetail.tsx` | Timed lot — bid form, history, watchlist |
| `client/LiveRoom.tsx` | Live auction room — spotlight + bid stream |
| `client/Account.tsx` | Bid history + watchlist + wins |

## Out of scope

- Payment integration; bids debit fake `balanceCents`
- Lot image uploads; use `db.uploadFile` for those
- Bidder notifications
- Automatic daily cron bootstrap; call `/api/fn/dailyAuctionCron` once after deploy
  to start the self-perpetuating daily auction cycle)
