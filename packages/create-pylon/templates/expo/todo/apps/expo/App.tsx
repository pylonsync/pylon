import { useEffect, useState } from "react";
import {
	View,
	Text,
	TextInput,
	Pressable,
	FlatList,
	ActivityIndicator,
	StyleSheet,
	Platform,
	Alert,
} from "react-native";
import { StatusBar } from "expo-status-bar";
import { init, db, callFn } from "@pylonsync/react-native";

const PYLON_BASE_URL =
	process.env.EXPO_PUBLIC_PYLON_BASE_URL ??
	(Platform.OS === "android" ? "http://10.0.2.2:4321" : "http://localhost:4321");

type Todo = {
	id: string;
	title: string;
	done: boolean;
	createdAt: string;
	position?: number;
};

let initPromise: Promise<void> | null = null;
function ensureInit() {
	if (!initPromise) {
		initPromise = init({
			baseUrl: PYLON_BASE_URL,
			appName: "__APP_NAME_SNAKE__",
		});
	}
	return initPromise;
}

export default function App() {
	const [ready, setReady] = useState(false);
	useEffect(() => {
		ensureInit().then(() => setReady(true));
	}, []);

	if (!ready) {
		return (
			<View style={[styles.screen, styles.center]}>
				<ActivityIndicator />
			</View>
		);
	}
	return <TodoApp />;
}

function TodoApp() {
	const { data: todos, loading } = db.useQuery<Todo>("Todo", {});
	const [draft, setDraft] = useState("");
	const [busy, setBusy] = useState(false);
	const [editingId, setEditingId] = useState<string | null>(null);
	const [editingDraft, setEditingDraft] = useState("");

	async function add() {
		const trimmed = draft.trim();
		if (!trimmed) return;
		setBusy(true);
		try {
			await callFn("addTodo", { title: trimmed });
			setDraft("");
		} catch (e) {
			Alert.alert("Add failed", String(e));
		} finally {
			setBusy(false);
		}
	}

	async function toggle(t: Todo) {
		try {
			await callFn("toggleTodo", { id: t.id, done: !t.done });
		} catch (e) {
			Alert.alert("Toggle failed", String(e));
		}
	}

	async function remove(t: Todo) {
		try {
			await callFn("deleteTodo", { id: t.id });
		} catch (e) {
			Alert.alert("Delete failed", String(e));
		}
	}

	async function commitEdit(t: Todo) {
		const trimmed = editingDraft.trim();
		if (!trimmed || trimmed === t.title) {
			setEditingId(null);
			setEditingDraft("");
			return;
		}
		try {
			await callFn("editTodo", { id: t.id, title: trimmed });
		} catch (e) {
			Alert.alert("Rename failed", String(e));
		} finally {
			setEditingId(null);
			setEditingDraft("");
		}
	}

	const sorted = (todos ?? [])
		.slice()
		.sort((a, b) => {
			const ap = typeof a.position === "number" ? a.position : Date.parse(a.createdAt) || 0;
			const bp = typeof b.position === "number" ? b.position : Date.parse(b.createdAt) || 0;
			return ap - bp;
		});

	return (
		<View style={styles.screen}>
			<StatusBar style="auto" />
			<Text style={styles.title}>__APP_NAME__</Text>
			<Text style={styles.subtitle}>
				Tap to toggle. Long-press to edit. Swipe-style buttons for delete.
			</Text>

			<View style={styles.row}>
				<TextInput
					style={styles.input}
					placeholder="What needs doing?"
					value={draft}
					onChangeText={setDraft}
					onSubmitEditing={add}
					editable={!busy}
					autoCorrect={false}
				/>
				<Pressable
					onPress={add}
					disabled={busy || !draft.trim()}
					style={({ pressed }) => [
						styles.button,
						(busy || !draft.trim()) && styles.buttonDisabled,
						pressed && styles.buttonPressed,
					]}
				>
					<Text style={styles.buttonLabel}>Add</Text>
				</Pressable>
			</View>

			{loading ? (
				<ActivityIndicator style={{ marginTop: 24 }} />
			) : sorted.length === 0 ? (
				<Text style={styles.empty}>No todos yet.</Text>
			) : (
				<FlatList
					data={sorted}
					keyExtractor={(t) => t.id}
					contentContainerStyle={{ paddingTop: 16 }}
					ItemSeparatorComponent={() => <View style={styles.separator} />}
					renderItem={({ item }) => (
						<Row
							todo={item}
							editing={editingId === item.id}
							editingDraft={editingDraft}
							onEditingDraftChange={setEditingDraft}
							onStartEdit={() => {
								setEditingId(item.id);
								setEditingDraft(item.title);
							}}
							onCommitEdit={() => commitEdit(item)}
							onCancelEdit={() => {
								setEditingId(null);
								setEditingDraft("");
							}}
							onToggle={() => toggle(item)}
							onDelete={() => remove(item)}
						/>
					)}
				/>
			)}
		</View>
	);
}

function Row({
	todo,
	editing,
	editingDraft,
	onEditingDraftChange,
	onStartEdit,
	onCommitEdit,
	onCancelEdit,
	onToggle,
	onDelete,
}: {
	todo: Todo;
	editing: boolean;
	editingDraft: string;
	onEditingDraftChange: (v: string) => void;
	onStartEdit: () => void;
	onCommitEdit: () => void;
	onCancelEdit: () => void;
	onToggle: () => void;
	onDelete: () => void;
}) {
	return (
		<View style={styles.item}>
			<Pressable onPress={onToggle} hitSlop={8}>
				<Text style={[styles.checkbox, todo.done && styles.checkboxDone]}>
					{todo.done ? "☑" : "☐"}
				</Text>
			</Pressable>
			{editing ? (
				<TextInput
					style={[styles.itemText, styles.itemEditInput]}
					value={editingDraft}
					onChangeText={onEditingDraftChange}
					onSubmitEditing={onCommitEdit}
					onBlur={onCommitEdit}
					autoFocus
					blurOnSubmit
				/>
			) : (
				<Pressable onLongPress={onStartEdit} style={{ flex: 1 }}>
					<Text
						style={[styles.itemText, todo.done && styles.itemTextDone]}
						numberOfLines={1}
					>
						{todo.title}
					</Text>
				</Pressable>
			)}
			{editing ? (
				<Pressable onPress={onCancelEdit} hitSlop={8}>
					<Text style={styles.actionLabel}>Cancel</Text>
				</Pressable>
			) : (
				<Pressable onPress={onDelete} hitSlop={8}>
					<Text style={[styles.actionLabel, styles.actionDelete]}>Delete</Text>
				</Pressable>
			)}
		</View>
	);
}

const styles = StyleSheet.create({
	screen: { flex: 1, paddingTop: 64, paddingHorizontal: 20, backgroundColor: "#fff" },
	center: { alignItems: "center", justifyContent: "center" },
	title: { fontSize: 28, fontWeight: "600" },
	subtitle: { color: "#666", marginTop: 4, marginBottom: 20 },
	row: { flexDirection: "row", gap: 8 },
	input: {
		flex: 1,
		borderWidth: 1,
		borderColor: "#d4d4d8",
		borderRadius: 6,
		paddingHorizontal: 12,
		paddingVertical: 8,
		fontSize: 14,
	},
	button: {
		backgroundColor: "#171717",
		borderRadius: 6,
		paddingHorizontal: 16,
		justifyContent: "center",
	},
	buttonDisabled: { opacity: 0.5 },
	buttonPressed: { opacity: 0.8 },
	buttonLabel: { color: "#fff", fontWeight: "600" },
	empty: { textAlign: "center", color: "#999", marginTop: 32 },
	item: {
		flexDirection: "row",
		alignItems: "center",
		gap: 12,
		paddingVertical: 12,
	},
	checkbox: { fontSize: 22, color: "#9ca3af" },
	checkboxDone: { color: "#16a34a" },
	itemText: { flex: 1, fontSize: 15 },
	itemTextDone: { color: "#9ca3af", textDecorationLine: "line-through" },
	itemEditInput: {
		borderBottomWidth: 1,
		borderColor: "#d4d4d8",
		paddingVertical: 4,
	},
	actionLabel: { fontSize: 13, color: "#6b7280" },
	actionDelete: { color: "#ef4444" },
	separator: { height: 1, backgroundColor: "#e5e5e5" },
});
