import { useCallback, useEffect, useRef, useState } from "react";

import { errorText } from "@/lib/format";

export interface QueryOptions {
	/** Load again at this interval, in milliseconds. 0 turns polling off. */
	pollMs?: number;
	/** Set to false to make no request. */
	enabled?: boolean;
}

export interface QueryResult<T> {
	/** The last good response. It stays while a later request runs or fails. */
	data: T | null;
	/** The message of the last request, when that request failed. */
	error: string | null;
	/** A request for the current key runs and has no answer yet. */
	loading: boolean;
	/** Load again now. The Promise settles when the request does. */
	reload: () => Promise<void>;
	/**
	 * The number of the request that produced `data`. Request numbers grow
	 * in the order requests start, across every query in the page.
	 */
	dataRequest: number;
}

let requestCounter = 0;

/**
 * Whether a response replaces the shown state. A response for another key
 * always does. For the same key, only a request that started after the one
 * shown does.
 */
export function responseReplaces(
	shown: { key: string | null; request: number },
	response: { key: string; request: number },
): boolean {
	return shown.key !== response.key || response.request > shown.request;
}

/** The number the next request will get. A request started later gets a larger one. */
export function nextRequestNumber(): number {
	return requestCounter + 1;
}

/**
 * Load data for a key, and load again when the key changes.
 *
 * `key` names the request: the same key means the same data. `fetcher` makes
 * the request. A response that arrives for an older key is dropped, and so is
 * a response to a request that started before the one whose data is shown.
 */
export function useQuery<T>(
	key: string,
	fetcher: () => Promise<T>,
	{ pollMs = 0, enabled = true }: QueryOptions = {},
): QueryResult<T> {
	const [state, setState] = useState<{
		key: string | null;
		data: T | null;
		error: string | null;
		request: number;
	}>({ key: null, data: null, error: null, request: 0 });
	const [manual, setManual] = useState(0);

	const latest = useRef({ key, fetcher });
	useEffect(() => {
		latest.current = { key, fetcher };
	});

	const run = useCallback(async (forKey: string) => {
		const request = ++requestCounter;
		try {
			const data = await latest.current.fetcher();
			if (latest.current.key !== forKey) return;
			setState((prev) =>
				responseReplaces(prev, { key: forKey, request })
					? { key: forKey, data, error: null, request }
					: prev,
			);
		} catch (err) {
			if (latest.current.key !== forKey) return;
			setState((prev) =>
				responseReplaces(prev, { key: forKey, request })
					? { key: forKey, data: prev.data, error: errorText(err), request: prev.request }
					: prev,
			);
		}
	}, []);

	useEffect(() => {
		if (!enabled) return;
		void run(key);
		if (!pollMs) return;
		const timer = window.setInterval(() => void run(key), pollMs);
		return () => window.clearInterval(timer);
	}, [key, enabled, pollMs, run]);

	const reload = useCallback(async () => {
		setManual((n) => n + 1);
		try {
			await run(latest.current.key);
		} finally {
			setManual((n) => n - 1);
		}
	}, [run]);

	return {
		data: state.data,
		error: state.error,
		loading: enabled && (state.key !== key || manual > 0),
		reload,
		dataRequest: state.request,
	};
}
