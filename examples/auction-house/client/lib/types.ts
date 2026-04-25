export type AuctionKind = "timed" | "live";
export type AuctionStatus = "draft" | "scheduled" | "running" | "ended";

export type Auction = {
  id: string;
  title: string;
  description: string;
  kind: AuctionKind;
  status: AuctionStatus;
  creatorId: string;
  startsAt: string;
  endsAt: string;
  currentLotId?: string | null;
  bannerColor?: string | null;
  createdAt: string;
};

export type LotStatus = "pending" | "running" | "sold" | "passed";

export type Lot = {
  id: string;
  auctionId: string;
  position: number;
  title: string;
  description: string;
  imageColor?: string | null;
  startingCents: number;
  currentCents: number;
  minIncrementCents: number;
  bidCount: number;
  status: LotStatus;
  endsAt?: string | null;
  winningBidId?: string | null;
  winnerId?: string | null;
  soldAt?: string | null;
  createdAt: string;
};

export type Bid = {
  id: string;
  auctionId: string;
  lotId: string;
  bidderId: string;
  bidderName: string;
  amountCents: number;
  createdAt: string;
};

export type Watch = {
  id: string;
  userId: string;
  lotId: string;
  addedAt: string;
};

export type AuthUser = {
  id: string;
  email?: string;
  displayName?: string;
  balanceCents?: number;
} | null;
