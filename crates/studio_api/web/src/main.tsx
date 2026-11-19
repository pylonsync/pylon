import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import App from "./App.tsx";
import { provideSharedModules } from "./lib/shared-modules";
import { colorScheme } from "./lib/theme";

// Extensions load after the first render and import these modules.
provideSharedModules();

// Put the saved color scheme on <html> before the first paint.
document.documentElement.setAttribute(
	"data-color-scheme",
	colorScheme.get().value,
);

createRoot(document.getElementById("root")!).render(
	<StrictMode>
		<App />
	</StrictMode>,
);
