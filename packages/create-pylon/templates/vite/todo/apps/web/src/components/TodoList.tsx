import { useState, useTransition, useRef, useEffect } from "react";
import { Button, Input } from "@__APP_NAME_KEBAB__/ui";
import { db } from "@pylonsync/react";
import {
	DndContext,
	closestCenter,
	KeyboardSensor,
	PointerSensor,
	useSensor,
	useSensors,
	type DragEndEvent,
} from "@dnd-kit/core";
import {
	arrayMove,
	SortableContext,
	sortableKeyboardCoordinates,
	useSortable,
	verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";

type Todo = {
	id: string;
	title: string;
	done: boolean;
	createdAt: string;
	position?: number;
};

// Live query — db.useQuery subscribes to Todo and re-renders on every
// server-pushed change. Compare to a Next.js Server Component which
// fetches once at request time and goes stale. The order matches
// listTodos: explicit `position` first, falling back to createdAt
// for legacy rows.
function useTodos(): Todo[] {
	const { data } = db.useQuery<Todo>("Todo");
	const rows = data ?? [];
	return [...rows].sort((a, b) => {
		const ap =
			typeof a.position === "number" ? a.position : Date.parse(a.createdAt) || 0;
		const bp =
			typeof b.position === "number" ? b.position : Date.parse(b.createdAt) || 0;
		return ap - bp;
	});
}

export function TodoList() {
	const todos = useTodos();
	const [title, setTitle] = useState("");
	const [pending, startTransition] = useTransition();
	const sensors = useSensors(
		useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
		useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
	);

	function add() {
		if (!title.trim()) return;
		const newTitle = title;
		setTitle("");
		startTransition(async () => {
			await db.fn<Todo>("addTodo", { title: newTitle }).catch(() => null);
		});
	}

	function toggle(t: Todo) {
		startTransition(async () => {
			await db
				.fn("toggleTodo", { id: t.id, done: !t.done })
				.catch(() => null);
		});
	}

	function remove(t: Todo) {
		startTransition(async () => {
			await db.fn("deleteTodo", { id: t.id }).catch(() => null);
		});
	}

	function rename(t: Todo, newTitle: string) {
		const trimmed = newTitle.trim();
		if (!trimmed || trimmed === t.title) return;
		startTransition(async () => {
			await db.fn("editTodo", { id: t.id, title: trimmed }).catch(() => null);
		});
	}

	function onDragEnd(e: DragEndEvent) {
		const { active, over } = e;
		if (!over || active.id === over.id) return;
		const oldIndex = todos.findIndex((t) => t.id === active.id);
		const newIndex = todos.findIndex((t) => t.id === over.id);
		if (oldIndex < 0 || newIndex < 0) return;
		const reordered = arrayMove(todos, oldIndex, newIndex);
		const prev = reordered[newIndex - 1];
		const next = reordered[newIndex + 1];
		const prevPos = prev?.position ?? Date.parse(prev?.createdAt ?? "") ?? 0;
		const nextPos = next?.position ?? Date.parse(next?.createdAt ?? "") ?? 0;
		let position: number;
		if (prev && next) position = (prevPos + nextPos) / 2;
		else if (prev) position = prevPos + 1024;
		else if (next) position = nextPos - 1024;
		else position = 1024;
		const movedId = String(active.id);
		startTransition(async () => {
			await db.fn("reorderTodo", { id: movedId, position }).catch(() => null);
		});
	}

	return (
		<div className="space-y-4">
			<form
				onSubmit={(e) => {
					e.preventDefault();
					add();
				}}
				className="flex gap-2"
			>
				<Input
					value={title}
					onChange={(e) => setTitle(e.target.value)}
					placeholder="What needs doing?"
					disabled={pending}
					className="flex-1"
				/>
				<Button
					type="submit"
					variant="primary"
					disabled={pending || !title.trim()}
				>
					Add
				</Button>
			</form>

			{todos.length === 0 ? (
				<p className="text-sm text-neutral-500 dark:text-neutral-400 text-center py-8">
					No todos yet. Add one above.
				</p>
			) : (
				<DndContext
					sensors={sensors}
					collisionDetection={closestCenter}
					onDragEnd={onDragEnd}
				>
					<SortableContext
						items={todos.map((t) => t.id)}
						strategy={verticalListSortingStrategy}
					>
						<ul className="divide-y divide-neutral-200 dark:divide-neutral-800 rounded-md border border-neutral-200 dark:border-neutral-800">
							{todos.map((t) => (
								<SortableRow
									key={t.id}
									todo={t}
									pending={pending}
									onToggle={() => toggle(t)}
									onRemove={() => remove(t)}
									onRename={(next) => rename(t, next)}
								/>
							))}
						</ul>
					</SortableContext>
				</DndContext>
			)}
		</div>
	);
}

function SortableRow({
	todo,
	pending,
	onToggle,
	onRemove,
	onRename,
}: {
	todo: Todo;
	pending: boolean;
	onToggle: () => void;
	onRemove: () => void;
	onRename: (next: string) => void;
}) {
	const {
		attributes,
		listeners,
		setNodeRef,
		transform,
		transition,
		isDragging,
	} = useSortable({ id: todo.id });
	const style = {
		transform: CSS.Transform.toString(transform),
		transition,
		opacity: isDragging ? 0.4 : 1,
	};
	const [editing, setEditing] = useState(false);
	const [draft, setDraft] = useState(todo.title);
	const inputRef = useRef<HTMLInputElement>(null);

	useEffect(() => {
		if (editing) {
			setDraft(todo.title);
			requestAnimationFrame(() => {
				inputRef.current?.focus();
				inputRef.current?.select();
			});
		}
	}, [editing, todo.title]);

	function commit() {
		setEditing(false);
		onRename(draft);
	}

	return (
		<li
			ref={setNodeRef}
			style={style}
			className="flex items-center gap-3 px-4 py-3 text-sm group bg-white dark:bg-neutral-950"
		>
			<button
				type="button"
				{...attributes}
				{...listeners}
				className="cursor-grab active:cursor-grabbing text-neutral-300 hover:text-neutral-500 select-none touch-none"
				aria-label="Drag to reorder"
				tabIndex={-1}
			>
				⋮⋮
			</button>
			<input
				type="checkbox"
				checked={todo.done}
				onChange={onToggle}
				disabled={pending}
				className="size-4 cursor-pointer"
				aria-label={`Mark "${todo.title}" as ${todo.done ? "not done" : "done"}`}
			/>
			{editing ? (
				<input
					ref={inputRef}
					value={draft}
					onChange={(e) => setDraft(e.target.value)}
					onBlur={commit}
					onKeyDown={(e) => {
						if (e.key === "Enter") commit();
						else if (e.key === "Escape") {
							setEditing(false);
							setDraft(todo.title);
						}
					}}
					className="flex-1 bg-transparent border-b border-neutral-300 dark:border-neutral-700 outline-none text-sm"
					aria-label="Edit title"
				/>
			) : (
				<button
					type="button"
					onDoubleClick={() => setEditing(true)}
					className={`flex-1 text-left ${todo.done ? "line-through text-neutral-400" : ""}`}
					title="Double-click to edit"
				>
					{todo.title}
				</button>
			)}
			<button
				type="button"
				onClick={() => setEditing(true)}
				className="opacity-0 group-hover:opacity-100 transition-opacity text-xs text-neutral-500 hover:text-neutral-800 dark:hover:text-neutral-200"
				aria-label={`Edit "${todo.title}"`}
			>
				Edit
			</button>
			<button
				type="button"
				onClick={onRemove}
				disabled={pending}
				className="opacity-0 group-hover:opacity-100 transition-opacity text-xs text-neutral-500 hover:text-red-500"
				aria-label={`Delete "${todo.title}"`}
			>
				Delete
			</button>
		</li>
	);
}
