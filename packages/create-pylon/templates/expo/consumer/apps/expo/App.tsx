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
import { init, callFn } from "@pylonsync/react-native";

const PYLON_BASE_URL =
	process.env.EXPO_PUBLIC_PYLON_BASE_URL ??
	(Platform.OS === "android" ? "http://10.0.2.2:4321" : "http://localhost:4321");

type Profile = {
	id: string;
	userId: string;
	handle: string;
	displayName: string;
	bio?: string | null;
	createdAt: string;
};

type FeedItem = {
	id: string;
	body: string;
	createdAt: string;
	author: { id: string; handle: string; displayName: string } | null;
	likeCount: number;
	likedByMe: boolean;
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
	const [me, setMe] = useState<Profile | null>(null);

	useEffect(() => {
		ensureInit().then(async () => {
			try {
				const profile = await callFn<Profile | null>("myProfile", {});
				setMe(profile);
			} catch {
				setMe(null);
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
	if (!me) {
		return <ProfileSetup onSaved={setMe} />;
	}
	return <Feed me={me} />;
}

function ProfileSetup({ onSaved }: { onSaved: (p: Profile) => void }) {
	const [handle, setHandle] = useState("");
	const [displayName, setDisplayName] = useState("");
	const [bio, setBio] = useState("");
	const [saving, setSaving] = useState(false);

	async function save() {
		setSaving(true);
		try {
			const p = await callFn<Profile>("upsertProfile", {
				handle: handle.trim().toLowerCase(),
				displayName: displayName.trim(),
				bio: bio.trim(),
			});
			onSaved(p);
		} catch (e) {
			Alert.alert("Save failed", String(e));
		} finally {
			setSaving(false);
		}
	}

	return (
		<View style={styles.screen}>
			<StatusBar style="auto" />
			<Text style={styles.title}>__APP_NAME__</Text>
			<Text style={styles.subtitle}>Set up your profile</Text>
			<TextInput
				style={styles.input}
				placeholder="handle (lowercase, 2–20)"
				autoCapitalize="none"
				autoCorrect={false}
				value={handle}
				onChangeText={setHandle}
			/>
			<TextInput
				style={styles.input}
				placeholder="Display name"
				value={displayName}
				onChangeText={setDisplayName}
			/>
			<TextInput
				style={styles.input}
				placeholder="Bio (optional)"
				value={bio}
				onChangeText={setBio}
			/>
			<Pressable
				onPress={save}
				disabled={saving || !handle.trim() || !displayName.trim()}
				style={({ pressed }) => [
					styles.button,
					(saving || !handle.trim() || !displayName.trim()) && styles.buttonDisabled,
					pressed && styles.buttonPressed,
				]}
			>
				<Text style={styles.buttonLabel}>{saving ? "Saving…" : "Save"}</Text>
			</Pressable>
		</View>
	);
}

function Feed({ me }: { me: Profile }) {
	const [feed, setFeed] = useState<FeedItem[]>([]);
	const [loading, setLoading] = useState(true);
	const [draft, setDraft] = useState("");
	const [posting, setPosting] = useState(false);

	useEffect(() => {
		void load();
	}, []);

	async function load() {
		setLoading(true);
		try {
			setFeed(await callFn<FeedItem[]>("feed", {}));
		} catch (e) {
			Alert.alert("Load failed", String(e));
		} finally {
			setLoading(false);
		}
	}

	async function post() {
		const body = draft.trim();
		if (!body) return;
		setPosting(true);
		try {
			const item = await callFn<FeedItem>("createPost", { body });
			setFeed((prev) => [item, ...prev]);
			setDraft("");
		} catch (e) {
			Alert.alert("Post failed", String(e));
		} finally {
			setPosting(false);
		}
	}

	async function toggleLike(item: FeedItem) {
		setFeed((prev) =>
			prev.map((p) =>
				p.id === item.id
					? {
							...p,
							likedByMe: !p.likedByMe,
							likeCount: p.likeCount + (p.likedByMe ? -1 : 1),
						}
					: p,
			),
		);
		try {
			const result = await callFn<{ liked: boolean; likeCount: number }>(
				"toggleLike",
				{ postId: item.id },
			);
			setFeed((prev) =>
				prev.map((p) =>
					p.id === item.id
						? { ...p, likedByMe: result.liked, likeCount: result.likeCount }
						: p,
				),
			);
		} catch {
			setFeed((prev) =>
				prev.map((p) =>
					p.id === item.id
						? { ...p, likedByMe: item.likedByMe, likeCount: item.likeCount }
						: p,
				),
			);
		}
	}

	async function remove(item: FeedItem) {
		setFeed((prev) => prev.filter((p) => p.id !== item.id));
		try {
			await callFn("deletePost", { id: item.id });
		} catch {
			setFeed((prev) => [...prev, item]);
		}
	}

	return (
		<View style={styles.screen}>
			<StatusBar style="auto" />
			<View style={styles.headerRow}>
				<View>
					<Text style={styles.title}>__APP_NAME__</Text>
					<Text style={styles.handle}>@{me.handle}</Text>
				</View>
			</View>

			<View style={styles.composerCard}>
				<TextInput
					style={styles.composerInput}
					placeholder="What's on your mind?"
					value={draft}
					onChangeText={setDraft}
					multiline
					maxLength={1000}
				/>
				<View style={styles.composerFoot}>
					<Text style={styles.counter}>{draft.length}/1000</Text>
					<Pressable
						onPress={post}
						disabled={posting || !draft.trim()}
						style={({ pressed }) => [
							styles.buttonSmall,
							(posting || !draft.trim()) && styles.buttonDisabled,
							pressed && styles.buttonPressed,
						]}
					>
						<Text style={styles.buttonLabel}>{posting ? "Posting…" : "Post"}</Text>
					</Pressable>
				</View>
			</View>

			{loading ? (
				<ActivityIndicator style={{ marginTop: 24 }} />
			) : feed.length === 0 ? (
				<Text style={styles.empty}>No posts yet.</Text>
			) : (
				<FlatList
					data={feed}
					keyExtractor={(p) => p.id}
					contentContainerStyle={{ paddingTop: 8, paddingBottom: 32 }}
					ItemSeparatorComponent={() => <View style={styles.separator} />}
					renderItem={({ item }) => (
						<View style={styles.post}>
							<View style={styles.postHead}>
								<Text style={styles.postName}>
									{item.author?.displayName ?? "Unknown"}
								</Text>
								<Text style={styles.postHandle}>
									@{item.author?.handle ?? "?"}
								</Text>
							</View>
							<Text style={styles.postBody}>{item.body}</Text>
							<View style={styles.postFoot}>
								<Pressable onPress={() => toggleLike(item)}>
									<Text
										style={[
											styles.likeBtn,
											item.likedByMe && styles.likeBtnActive,
										]}
									>
										{item.likedByMe ? "♥" : "♡"} {item.likeCount}
									</Text>
								</Pressable>
								{item.author?.id === me.id && (
									<Pressable onPress={() => remove(item)}>
										<Text style={styles.deleteBtn}>Delete</Text>
									</Pressable>
								)}
							</View>
						</View>
					)}
				/>
			)}
		</View>
	);
}

const styles = StyleSheet.create({
	screen: { flex: 1, paddingTop: 64, paddingHorizontal: 20, backgroundColor: "#fff" },
	center: { alignItems: "center", justifyContent: "center" },
	title: { fontSize: 28, fontWeight: "600" },
	subtitle: { color: "#666", marginTop: 4, marginBottom: 20 },
	handle: { fontFamily: "Menlo", fontSize: 12, color: "#999" },
	headerRow: { flexDirection: "row", justifyContent: "space-between", marginBottom: 16 },
	input: {
		borderWidth: 1,
		borderColor: "#d4d4d8",
		borderRadius: 6,
		paddingHorizontal: 12,
		paddingVertical: 8,
		fontSize: 14,
		marginBottom: 8,
	},
	composerCard: {
		borderWidth: 1,
		borderColor: "#e5e5e5",
		borderRadius: 8,
		padding: 12,
		marginBottom: 12,
	},
	composerInput: { minHeight: 70, fontSize: 14, textAlignVertical: "top" },
	composerFoot: {
		flexDirection: "row",
		justifyContent: "space-between",
		alignItems: "center",
		marginTop: 6,
	},
	counter: { color: "#999", fontSize: 12 },
	button: {
		backgroundColor: "#171717",
		borderRadius: 6,
		paddingHorizontal: 16,
		paddingVertical: 12,
		justifyContent: "center",
		alignItems: "center",
	},
	buttonSmall: {
		backgroundColor: "#171717",
		borderRadius: 6,
		paddingHorizontal: 12,
		paddingVertical: 6,
	},
	buttonDisabled: { opacity: 0.5 },
	buttonPressed: { opacity: 0.8 },
	buttonLabel: { color: "#fff", fontWeight: "600", fontSize: 13 },
	empty: { textAlign: "center", color: "#999", marginTop: 32 },
	separator: { height: 1, backgroundColor: "#e5e5e5" },
	post: { paddingVertical: 12 },
	postHead: { flexDirection: "row", alignItems: "baseline", gap: 6 },
	postName: { fontSize: 14, fontWeight: "500" },
	postHandle: { fontFamily: "Menlo", fontSize: 12, color: "#999" },
	postBody: { fontSize: 15, marginTop: 4, lineHeight: 20 },
	postFoot: { flexDirection: "row", gap: 16, marginTop: 8 },
	likeBtn: { fontSize: 13, color: "#666" },
	likeBtnActive: { color: "#ec4899" },
	deleteBtn: { fontSize: 13, color: "#ef4444" },
});
