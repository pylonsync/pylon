# Store — faceted search showcase

10,000-SKU demo storefront built on Pylon's native faceted search. One
`search:` declaration on the `Product` entity unlocks the whole flow:
BM25 text ranking, live facet counts with bitmap-backed intersection,
filter expressions, sort by any declared field.

Writes to `Product` update FTS5 and the facet bitmaps in the same
transaction. New products change facet counts and results immediately,
without a separate search service or dual-write path.

## What this example demonstrates

- **`db.useSearch<T>(entity, spec)`**: the client hook
- **`search: { text, facets, sortable }`** in `app.ts` — the schema
  declaration that creates `_fts_Product` + `_facet_bitmap` shadow
  tables on first push
- **Live reactivity**: inserting a new row updates every open
  browser's facet counts within a frame
- **Sort across full result set**: price-desc pagination surfaces
  the highest-priced items on page 0, even when matches
  span 10k+ rows
- **Policy enforcement**: search is refused on entities whose read
  policy depends on per-row data (would leak aggregates)

## Run it

```bash
cd examples/store
bun install
pylon dev              # backend :4321

# second terminal
cd examples/store/web
bun install
bun run dev            # UI :5179
```

The first load seeds 10,000 products via the `seedCatalog` function.
Subsequent loads skip seeding.

## Files

- `app.ts`: `Product` entity + `search:` declaration + public-read
  policy
- `functions/seedCatalog.ts`: idempotent bulk seeder
- `client/StoreApp.tsx`: the UI (~300 lines, no external search libs)
- `web/`: minimal Vite host

## Performance

On a 10k catalog with 10 facets × 10 values each, query and facet
calculation takes about 5–15ms on a laptop. Bitmap intersection provides
that speed without a dual-write path.

For production-scale (1M+ rows), Roaring bitmaps stay sublinear on
memory and the query plan is unchanged. See the codex design notes in
`crates/storage/src/search.rs` for the architecture.
