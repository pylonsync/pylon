import { TodoList } from "./components/TodoList";

export function App() {
	return (
		<main className="mx-auto max-w-2xl px-6 py-12 space-y-8">
			<header className="space-y-2">
				<h1 className="text-3xl font-semibold tracking-tight">__APP_NAME__</h1>
				<p className="text-sm text-neutral-500 dark:text-neutral-400">
					Drag rows to reorder, double-click a title to edit, hover for
					delete. All mutations are optimistic with revert-on-failure.
				</p>
			</header>
			<TodoList />
		</main>
	);
}
