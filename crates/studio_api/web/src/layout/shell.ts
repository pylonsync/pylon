import { createContext, useContext } from "react";
import type { StudioRoute } from "@/layout/StudioLayout";

export interface Shell {
	/** The sidebar is not on screen: the window is narrow, or it is collapsed. */
	navHidden: boolean;
	/** Whether the top bar shows the sidebar button. */
	navToggle: boolean;
	/** Opens the sidebar sheet on a narrow window. Shows or hides the sidebar on a wide one. */
	toggleNav: () => void;
	navigate: (route: StudioRoute) => void;
}

export const ShellContext = createContext<Shell | null>(null);

export function useShell(): Shell {
	const shell = useContext(ShellContext);
	if (!shell) throw new Error("useShell must be used inside <StudioLayout>");
	return shell;
}
