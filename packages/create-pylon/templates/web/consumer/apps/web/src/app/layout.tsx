import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
	title: "__APP_NAME__",
	description: "Multi-tenant SaaS scaffold powered by Pylon",
};

export default function RootLayout({
	children,
}: {
	children: React.ReactNode;
}) {
	return (
		<html lang="en">
			<body className="antialiased min-h-screen bg-white dark:bg-neutral-950 text-neutral-900 dark:text-neutral-100">
				{children}
			</body>
		</html>
	);
}
