import { useEffect, type RefObject } from "react";

/**
 * Close a panel that is not modal when Escape is pressed while focus is
 * outside it. The panel handles Escape itself while focus is inside, and an
 * open popup or dialog handles its own Escape.
 */
export function useEscapeOutside(
	open: boolean,
	panel: RefObject<HTMLElement | null>,
	onClose: () => void,
): void {
	useEffect(() => {
		if (!open) return;
		const onKey = (e: KeyboardEvent) => {
			if (e.key !== "Escape" || e.defaultPrevented) return;
			const active = document.activeElement;
			if (active && panel.current?.contains(active)) return;
			if (active?.closest("[data-open]")) return;
			onClose();
		};
		document.addEventListener("keydown", onKey);
		return () => document.removeEventListener("keydown", onKey);
	}, [open, panel, onClose]);
}
