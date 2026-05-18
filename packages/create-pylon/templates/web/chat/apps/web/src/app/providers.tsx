"use client";

import { useEffect, useState } from "react";
import { init } from "@pylonsync/react";

/**
 * Boots the Pylon sync engine on first mount. Every `db.useQuery`
 * downstream renders against the same engine instance, so a single
 * top-level Providers wrapper is enough — no need to thread a client
 * through context.
 *
 * `baseUrl` is the same-origin Next path because next.config.ts
 * proxies /api/* to the Pylon binary on PYLON_TARGET. Browser →
 * same-origin → Next server → Pylon: no CORS, no extra DNS, the
 * cookie travels naturally.
 */
export function Providers({ children }: { children: React.ReactNode }) {
	const [ready, setReady] = useState(false);
	useEffect(() => {
		init({ baseUrl: "", appName: "__APP_NAME_SNAKE__" });
		setReady(true);
	}, []);
	if (!ready) return null;
	return <>{children}</>;
}
