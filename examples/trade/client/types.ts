export type Ticker = {
  id: string;
  symbol: string;
  name: string;
  sector: string;
  price: number;
  openPrice: number;
  dayHigh: number;
  dayLow: number;
  volume: number;
  updatedAt: string;
  barT?: string | null;
  barOpen?: number | null;
  barHigh?: number | null;
  barLow?: number | null;
  barVolume?: number | null;
  barTrades?: number | null;
};

export type Bar = {
  id: string;
  symbol: string;
  t: string;
  open: number;
  high: number;
  low: number;
  close: number;
  volume: number;
  trades: number;
};

export type Fill = {
  id: string;
  slot: number;
  seq: number;
  symbol: string;
  price: number;
  qty: number;
  side: string;
  at: string;
};

export type Market = {
  id: string;
  key: string;
  seq: number;
  trades: number;
  volume: number;
  notional: number;
  updatedAt: string;
};

export type Watch = {
  id: string;
  userId: string;
  symbol: string;
  addedAt: string;
};

/** A Ticker with its change from the open. */
export type Quote = Ticker & { change: number; pct: number };

/** One second of a symbol's trading, keyed by epoch ms. */
export type Candle = {
  t: number;
  open: number;
  high: number;
  low: number;
  close: number;
  volume: number;
  trades: number;
};
