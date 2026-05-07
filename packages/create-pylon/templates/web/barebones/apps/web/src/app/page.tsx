import { pylon } from "@/lib/pylon";
import { WidgetList } from "./components/WidgetList";

// Force dynamic — every render reads the live widget list from Pylon.
export const dynamic = "force-dynamic";

type Widget = {
	id: string;
	name: string;
	count: number;
	createdAt: string;
};

export default async function HomePage() {
	const widgets = await pylon
		.json<Widget[]>("/api/fn/listWidgets", {
			method: "POST",
			body: "{}",
			headers: { "Content-Type": "application/json" },
		})
		.catch(() => [] as Widget[]);

	return (
		<main className="mx-auto max-w-2xl px-6 py-12 space-y-8">
			<header className="space-y-2">
				<h1 className="text-3xl font-semibold tracking-tight">__APP_NAME__</h1>
				<p className="text-sm text-neutral-500 dark:text-neutral-400">
					A bare-bones Pylon app. Edit{" "}
					<code className="font-mono text-xs">apps/api/schema.ts</code> to
					change the data model,{" "}
					<code className="font-mono text-xs">apps/api/functions/</code> to
					add handlers, or{" "}
					<code className="font-mono text-xs">
						apps/web/src/app/components/WidgetList.tsx
					</code>{" "}
					for the UI.
				</p>
			</header>

			<WidgetList initialWidgets={widgets} />
		</main>
	);
}
