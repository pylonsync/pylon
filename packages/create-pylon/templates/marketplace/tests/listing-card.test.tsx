import { afterEach, expect, mock, test } from "bun:test";
import React from "react";
import { cleanup, render, screen } from "@testing-library/react";
import type { Listing } from "../client/market";

mock.module("@pylonsync/react", () => ({
  Link: ({ href, children, seed: _seed, ...rest }: Record<string, unknown>) => (
    <a href={href as string} {...rest}>
      {children as React.ReactNode}
    </a>
  ),
  db: { useQuery: () => ({ data: [], loading: false }) },
  init: () => {},
  configureClient: () => {},
  callFn: async () => ({}),
  setSessionToken: async () => {},
  storageKey: (key: string) => key,
}));
const { ListingCard } = await import("../client/ListingCard");

afterEach(cleanup);

const listing: Listing = {
  id: "1",
  sellerId: "s1",
  sellerName: "Nora Lind",
  location: "Brooklyn, NY",
  title: "Danish teak lounge chair",
  slug: "danish-teak-lounge-chair-a1f3",
  description: "",
  price: 680,
  category: "furniture",
  condition: "good",
  status: "active",
  imageUrl: "/images/listings/lounge-chair.webp",
  seed: "a1f3",
  createdAt: new Date().toISOString(),
};

test("card shows price, title, location, and the live offer count", () => {
  render(<ListingCard listing={listing} pendingOffers={2} />);
  expect(screen.getByText("$680")).toBeDefined();
  expect(screen.getByText("Danish teak lounge chair")).toBeDefined();
  expect(screen.getByText("Brooklyn, NY · Good")).toBeDefined();
  expect(screen.getByText("2 offers")).toBeDefined();
  const link = screen.getByRole("link");
  expect(link.getAttribute("href")).toBe("/listing/danish-teak-lounge-chair-a1f3");
});

test("a sold card shows Sold instead of the offer count", () => {
  render(
    <ListingCard listing={{ ...listing, id: "2", status: "sold" }} pendingOffers={3} />,
  );
  expect(screen.getAllByText("Sold").length).toBe(1);
  expect(screen.queryByText("3 offers")).toBeNull();
});

test("a listing that arrived live is marked Just listed", () => {
  render(<ListingCard listing={{ ...listing, id: "3" }} fresh />);
  expect(screen.getByText("Just listed")).toBeDefined();
});
