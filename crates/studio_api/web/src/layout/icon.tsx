import { createElement, type ComponentType } from "react";

import { resolveIcon } from "@/layout/icons";

/**
 * An icon by its name in studio.config.ts. A name that is not in the icon
 * table shows `fallback`.
 */
export function Icon({
	name,
	fallback,
	className,
}: {
	name: string | undefined;
	fallback?: ComponentType<{ className?: string }>;
	className?: string;
}) {
	return createElement(resolveIcon(name, fallback), { className });
}
