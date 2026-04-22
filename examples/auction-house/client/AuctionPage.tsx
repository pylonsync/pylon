import { init, db, useFn } from "@statecraft/react";

init({ baseUrl: "http://localhost:4321" });

export function AuctionList() {
  const { data: listings, loading } = useFn<unknown, Listing[]>("activeListings", {});

  if (loading) return <div>Loading...</div>;
  return (
    <ul>
      {(listings ?? []).map((l) => (
        <li key={l.id}>
          <a href={`/listing/${l.id}`}>{l.title}</a>
          <span> — current bid: ${(l.currentCents / 100).toFixed(2)}</span>
          <span> — ends {new Date(l.endsAt).toLocaleString()}</span>
        </li>
      ))}
    </ul>
  );
}

export function ListingDetail({ listingId }: { listingId: string }) {
  const { data, loading, refetch } = useFn<{ listingId: string }, ListingDetailResult>(
    "listingDetail",
    { listingId },
    { refetchIntervalMs: 2000 },
  );
  const { mutate: placeBid, loading: bidding, error } = db.useMutation("placeBid");

  if (loading || !data) return <div>Loading...</div>;
  const { listing, seller, bids } = data;

  return (
    <div>
      <h1>{listing.title}</h1>
      <p>{listing.description}</p>
      <p>Seller: {seller?.displayName}</p>
      <p>Current bid: ${(listing.currentCents / 100).toFixed(2)}</p>
      <p>Ends: {new Date(listing.endsAt).toLocaleString()}</p>

      <button
        disabled={bidding}
        onClick={async () => {
          await placeBid({
            listingId: listing.id,
            amountCents: listing.currentCents + 100,
          });
          refetch();
        }}
      >
        Bid ${((listing.currentCents + 100) / 100).toFixed(2)}
      </button>
      {error && <p style={{ color: "red" }}>{error.message}</p>}

      <h3>Recent bids</h3>
      <ul>
        {bids.map((b) => (
          <li key={b.id}>
            ${(b.amountCents / 100).toFixed(2)} at {new Date(b.createdAt).toLocaleTimeString()}
          </li>
        ))}
      </ul>
    </div>
  );
}

interface Listing {
  id: string;
  title: string;
  description: string;
  currentCents: number;
  endsAt: string;
}

interface Bid {
  id: string;
  amountCents: number;
  createdAt: string;
}

interface ListingDetailResult {
  listing: Listing & { sellerId: string };
  seller: { displayName: string } | null;
  bids: Bid[];
}
