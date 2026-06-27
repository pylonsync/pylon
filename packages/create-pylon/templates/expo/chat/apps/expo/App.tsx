import { useEffect, useRef, useState } from "react";
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
	KeyboardAvoidingView,
	SafeAreaView,
} from "react-native";
import { StatusBar } from "expo-status-bar";
import { init, db, callFn } from "@pylonsync/react-native";

const PYLON_BASE_URL =
	process.env.EXPO_PUBLIC_PYLON_BASE_URL ??
	(Platform.OS === "android" ? "http://10.0.2.2:4321" : "http://localhost:4321");

type Room = {
	id: string;
	slug: string;
	name: string;
	createdAt: string;
};

type Message = {
	id: string;
	roomId: string;
	authorId: string;
	authorName: string;
	body: string;
	createdAt: string;
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
	return <Chat />;
}

function Chat() {
	// Live subscriptions — the sync engine pushes diffs over WebSocket, so new
	// rooms/messages from any device re-render this component without polling.
	const { data: rooms = [] } = db.useQuery<Room>("Room", {
		orderBy: { createdAt: "asc" },
	});
	const [activeId, setActiveId] = useState<string | null>(null);
	useEffect(() => {
		if (!activeId && rooms.length > 0) setActiveId(rooms[0].id);
	}, [rooms, activeId]);

	const active = rooms.find((r) => r.id === activeId) ?? null;

	const { data: messages = [] } = db.useQuery<Message>("Message", {
		where: activeId ? { roomId: activeId } : undefined,
		orderBy: { createdAt: "asc" },
		limit: 200,
	});

	const [draft, setDraft] = useState("");
	const [sending, setSending] = useState(false);
	const [authorName, setAuthorName] = useState("anonymous");
	const listRef = useRef<FlatList<Message>>(null);

	useEffect(() => {
		if (messages.length > 0) {
			listRef.current?.scrollToEnd({ animated: true });
		}
	}, [messages.length, activeId]);

	if (!active) {
		return (
			<RoomCreate
				authorName={authorName}
				onAuthorNameChange={setAuthorName}
				onCreated={(r) => setActiveId(r.id)}
			/>
		);
	}

	async function send() {
		const body = draft.trim();
		if (!body) return;
		setDraft("");
		setSending(true);
		try {
			await callFn("sendMessage", {
				roomId: active.id,
				body,
				authorName,
			});
		} catch (e) {
			setDraft(body);
			Alert.alert("Send failed", String(e));
		} finally {
			setSending(false);
		}
	}

	async function createRoom() {
		const name = window?.prompt?.("Room name?") ?? null;
		if (!name) return;
		const slug = name
			.toLowerCase()
			.replace(/[^a-z0-9]+/g, "-")
			.replace(/^-|-$/g, "");
		try {
			const r = (await callFn("createRoom", { slug, name })) as Room;
			setActiveId(r.id);
		} catch (e) {
			Alert.alert("Create failed", String(e));
		}
	}

	return (
		<SafeAreaView style={styles.screen}>
			<StatusBar style="auto" />
			<KeyboardAvoidingView
				style={styles.flex}
				behavior={Platform.OS === "ios" ? "padding" : undefined}
				keyboardVerticalOffset={64}
			>
				<View style={styles.header}>
					<View>
						<Text style={styles.title}>{active.name}</Text>
						<Text style={styles.handle}>#{active.slug}</Text>
					</View>
					{rooms.length > 1 && (
						<Pressable
							onPress={() => {
								const idx = rooms.findIndex((r) => r.id === active.id);
								setActiveId(rooms[(idx + 1) % rooms.length].id);
							}}
						>
							<Text style={styles.switchBtn}>Next room →</Text>
						</Pressable>
					)}
				</View>

				<View style={styles.namebar}>
					<Text style={styles.namelabel}>You:</Text>
					<TextInput
						style={styles.nameinput}
						value={authorName}
						onChangeText={setAuthorName}
						autoCapitalize="none"
					/>
				</View>

				<FlatList
					ref={listRef}
					data={messages}
					keyExtractor={(m) => m.id}
					contentContainerStyle={{ padding: 16 }}
					ListEmptyComponent={() => (
						<Text style={styles.empty}>No messages yet. Say hi.</Text>
					)}
					renderItem={({ item }) => (
						<View style={styles.msg}>
							<View style={styles.msgHead}>
								<Text style={styles.msgName}>{item.authorName}</Text>
								<Text style={styles.msgTime}>
									{new Date(item.createdAt).toLocaleTimeString(undefined, {
										hour: "numeric",
										minute: "2-digit",
									})}
								</Text>
							</View>
							<Text style={styles.msgBody}>{item.body}</Text>
						</View>
					)}
				/>

				<View style={styles.composer}>
					<TextInput
						style={styles.composerInput}
						value={draft}
						onChangeText={setDraft}
						placeholder={`Message ${active.name}…`}
						multiline
						editable={!sending}
					/>
					<Pressable
						onPress={send}
						disabled={sending || !draft.trim()}
						style={({ pressed }) => [
							styles.button,
							(sending || !draft.trim()) && styles.buttonDisabled,
							pressed && styles.buttonPressed,
						]}
					>
						<Text style={styles.buttonLabel}>Send</Text>
					</Pressable>
				</View>

				<Pressable onPress={createRoom} style={styles.newRoomBar}>
					<Text style={styles.newRoomLabel}>+ New room</Text>
				</Pressable>
			</KeyboardAvoidingView>
		</SafeAreaView>
	);
}

function RoomCreate({
	authorName,
	onAuthorNameChange,
	onCreated,
}: {
	authorName: string;
	onAuthorNameChange: (s: string) => void;
	onCreated: (r: Room) => void;
}) {
	const [name, setName] = useState("General");
	const [slug, setSlug] = useState("general");
	const [creating, setCreating] = useState(false);

	async function create() {
		setCreating(true);
		try {
			const r = (await callFn("createRoom", {
				slug: slug.toLowerCase(),
				name,
			})) as Room;
			onCreated(r);
		} catch (e) {
			Alert.alert("Create failed", String(e));
		} finally {
			setCreating(false);
		}
	}

	return (
		<SafeAreaView style={styles.screen}>
			<StatusBar style="auto" />
			<View style={styles.content}>
				<Text style={styles.title}>__APP_NAME__</Text>
				<Text style={styles.subtitle}>No rooms yet — create one.</Text>
				<TextInput
					style={styles.input}
					placeholder="Your display name"
					value={authorName}
					onChangeText={onAuthorNameChange}
				/>
				<TextInput
					style={styles.input}
					placeholder="Room name"
					value={name}
					onChangeText={setName}
				/>
				<TextInput
					style={styles.input}
					placeholder="Slug"
					value={slug}
					onChangeText={(s) => setSlug(s.toLowerCase())}
					autoCapitalize="none"
					autoCorrect={false}
				/>
				<Pressable
					onPress={create}
					disabled={creating || !name.trim() || !slug.trim()}
					style={({ pressed }) => [
						styles.button,
						(creating || !name.trim() || !slug.trim()) && styles.buttonDisabled,
						pressed && styles.buttonPressed,
					]}
				>
					<Text style={styles.buttonLabel}>
						{creating ? "Creating…" : "Create room"}
					</Text>
				</Pressable>
			</View>
		</SafeAreaView>
	);
}

const styles = StyleSheet.create({
	screen: { flex: 1, backgroundColor: "#fff" },
	flex: { flex: 1 },
	center: { alignItems: "center", justifyContent: "center" },
	content: { padding: 20, gap: 12 },
	title: { fontSize: 24, fontWeight: "600" },
	subtitle: { color: "#666" },
	handle: { fontFamily: "Menlo", fontSize: 12, color: "#999" },
	header: {
		paddingHorizontal: 20,
		paddingTop: 12,
		paddingBottom: 8,
		flexDirection: "row",
		justifyContent: "space-between",
		alignItems: "flex-end",
		borderBottomWidth: 1,
		borderColor: "#e5e5e5",
	},
	switchBtn: { color: "#3b82f6", fontSize: 13 },
	namebar: {
		flexDirection: "row",
		alignItems: "center",
		gap: 6,
		paddingHorizontal: 20,
		paddingVertical: 6,
		backgroundColor: "#fafafa",
	},
	namelabel: { fontSize: 12, color: "#666" },
	nameinput: {
		flex: 1,
		borderBottomWidth: 1,
		borderColor: "#d4d4d8",
		fontSize: 13,
		paddingVertical: 2,
	},
	input: {
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
		paddingVertical: 12,
		alignItems: "center",
	},
	buttonDisabled: { opacity: 0.5 },
	buttonPressed: { opacity: 0.8 },
	buttonLabel: { color: "#fff", fontWeight: "600", fontSize: 13 },
	empty: { textAlign: "center", color: "#999", marginTop: 32 },
	msg: { marginBottom: 16 },
	msgHead: { flexDirection: "row", alignItems: "baseline", gap: 6 },
	msgName: { fontSize: 13, fontWeight: "500" },
	msgTime: { fontSize: 11, color: "#999" },
	msgBody: { fontSize: 14, marginTop: 2 },
	composer: {
		flexDirection: "row",
		gap: 8,
		padding: 12,
		borderTopWidth: 1,
		borderColor: "#e5e5e5",
	},
	composerInput: {
		flex: 1,
		minHeight: 36,
		maxHeight: 120,
		borderWidth: 1,
		borderColor: "#d4d4d8",
		borderRadius: 6,
		paddingHorizontal: 12,
		paddingVertical: 8,
		fontSize: 14,
	},
	newRoomBar: {
		paddingVertical: 10,
		alignItems: "center",
		borderTopWidth: 1,
		borderColor: "#e5e5e5",
	},
	newRoomLabel: { color: "#3b82f6", fontSize: 13 },
});
