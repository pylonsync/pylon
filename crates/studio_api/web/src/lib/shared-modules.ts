import * as React from "react";
import * as ReactDOM from "react-dom";
import * as ReactDOMClient from "react-dom/client";
import * as JsxRuntime from "react/jsx-runtime";

import { MODULES_GLOBAL, type SharedSpecifier } from "@/lib/module-shims";

/**
 * The modules that Studio shares with extensions, keyed by specifier.
 *
 * `react/jsx-dev-runtime` maps `jsxDEV` to the production `jsx`: a bundler in
 * development mode compiles JSX to `jsxDEV` calls, and the production build
 * of React that Studio ships has no `jsxDEV`.
 */
export function sharedModules(): Record<SharedSpecifier, object> {
	return {
		react: React,
		"react-dom": ReactDOM,
		"react-dom/client": ReactDOMClient,
		"react/jsx-runtime": JsxRuntime,
		"react/jsx-dev-runtime": {
			Fragment: JsxRuntime.Fragment,
			jsxDEV: (type: React.ElementType, props: unknown, key?: React.Key) =>
				JsxRuntime.jsx(type, props, key),
		},
	};
}

/** Put the shared modules on the global that the shims read. */
export function provideSharedModules(): void {
	(globalThis as Record<string, unknown>)[MODULES_GLOBAL] = sharedModules();
}
