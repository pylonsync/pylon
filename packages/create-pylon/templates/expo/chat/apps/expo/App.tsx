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
import { init, callFn } from "@pylonsync/react-native";

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
	const [rooms, setRooms] = useState<Room[]>([]);
	const [active, setActive] = useState<Room | null>(null);
	const [authorName, setAuthorName] = useState("anonymous");

	useEffect(() => {
		ensureInit().then(async () => {
			try {
				const list = await callFn<Room[]>("listRooms", {});
				setRooms(list);
				setActive(list[0] ?? null);
			} catch (e) {
				Alert.alert("Load failed", String(e));
			}
			setReady(true);
		});
	}, []);

	if (!ready) {
		return (
			<View style={[styles.screen, styles.center]}>
				<ActivityIndicator />
			</View>
		);
	}

	if (!active) {
		return (
			<RoomCreate
				authorName={authorName}
				onAuthorNameChange={setAuthorName}
				onCreated={(r) => {
					setRooms((prev) => [...prev, r]);
					setActive(r);
				}}
			/>
		);
	}

	return (
		<RoomView
			room={active}
			rooms={rooms}
			onSwitch={setActive}
			authorName={authorName}
			onAuthorNameChange={setAuthorName}
			onCreate={(r) => {
				setRooms((prev) => [...prev, r]);
				setActive(r);
			}}
		/>
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
			const r = await callFn<Room>("createRoom", {
				slug: slug.toLowerCase(),
				name,
			});
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

function RoomView({
	room,
	rooms,
	onSwitch,
	authorName,
	onAuthorNameChange,
	onCreate,
}: {
	room: Room;
	rooms: Room[];
	onSwitch: (r: Room) => void;
	authorName: string;
	onAuthorNameChange: (s: string) => void;
	onCreate: (r: Room) => void;
}) {
	const [messages, setMessages] = useState<Message[]>([]);
	const [draft, setDraft] = useState("");
	const [sending, setSending] = useState(false);
	const listRef = useRef<FlatList<Message>>(null);

	useEffect(() => {
		void load();
		const t = setInterval(load, 1500);
		return () => clearInterval(t);
		async function load() {
			try {
				const m = await callFn<Message[]>("roomMessages", { roomId: room.id });
				setMessages(m);
			} catch {
				// ignore — will retry on next tick
			}
		}
	}, [room.id]);

	useEffect(() => {
		if (messages.length > 0) {
			listRef.current?.scrollToEnd({ animated: true });
		}
	}, [messages.length]);

	async function send() {
		const body = draft.trim();
		if (!body) return;
		setSending(true);
		setDraft("");
		try {
			const msg = await callFn<Message>("sendMessage", {
				roomId: room.id,
				body,
				authorName,
			});
			setMessages((prev) => [...prev, msg]);
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
			const r = await callFn<Room>("createRoom", { slug, name });
			onCreate(r);
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
						<Text style={styles.title}>{room.name}</Text>
						<Text style={styles.handle}>#{room.slug}</Text>
					</View>
					{rooms.length > 1 && (
						<Pressable
							onPress={() => {
								// Cycle to next room — simplest "switch room" UX without a sidebar.
								const idx = rooms.findIndex((r) => r.id === room.id);
								onSwitch(rooms[(idx + 1) % rooms.length]);
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
						onChangeText={onAuthorNameChange}
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
						placeholder={`Message ${room.name}…`}
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
			</KeyboardAvoidingView>
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
});
