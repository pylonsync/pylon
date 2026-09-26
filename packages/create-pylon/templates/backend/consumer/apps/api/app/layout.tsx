import React from "react";

interface LayoutProps {
	children: React.ReactNode;
}

const css = `
:root { --ink: #111; --muted: #6b6b6b; --surface: #fff; --line: #ececec; }
@media (prefers-color-scheme: dark) {
  :root { --ink: #f5f5f5; --muted: #9a9a9a; --surface: #000; --line: #262626; }
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--surface); color: var(--ink);
  font: 16px/1.5 -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", sans-serif; }
`;

/**
 * The document shell for the backend's one web page. The server also
 * serves public/ (the demo photos) at the site root.
 */
export default function RootLayout({ children }: LayoutProps) {
	return (
		<html lang="en">
			<head>
				<meta charSet="utf-8" />
				<meta name="viewport" content="width=device-width, initial-scale=1" />
				<meta name="color-scheme" content="light dark" />
				<style dangerouslySetInnerHTML={{ __html: css }} />
			</head>
			<body>{children}</body>
		</html>
	);
}
