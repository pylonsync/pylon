"use client";

import { createContext } from "react";
import type { MatchedRoute } from "./match";

export interface RouterContextValue {
	pathname: string;
	search: string;
	hash: string;
	query: Record<string, string>;
	params: Record<string, string>;
	/** The matched chain — root layout to leaf page. */
	matches: MatchedRoute[];
	push: (href: string) => void;
	replace: (href: string) => void;
	back: () => void;
}

export const RouterContext = createContext<RouterContextValue | null>(null);

/**
 * Used internally so a layout's `<Outlet />` knows which depth in the
 * match chain to render next. Apps don't touch this directly.
 */
export const RouterDepthContext = createContext<number>(0);
