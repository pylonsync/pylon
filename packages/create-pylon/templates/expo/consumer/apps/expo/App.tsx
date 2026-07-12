import { useEffect, useMemo, useState } from "react";
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
import { SafeAreaProvider, SafeAreaView } from "react-native-safe-area-context";
import { StatusBar } from "expo-status-bar";
import { init, db, callFn } from "@pylonsync/react-native";
import AsyncStorage from "@react-native-async-storage/async-storage";

const PYLON_BASE_URL =
	process.env.EXPO_PUBLIC_PYLON_BASE_URL ??
	(Platform.OS === "android" ? "http://10.0.2.2:4321" : "http://localhost:4321");

const PROFILE_KEY = "__APP_NAME_SNAKE___profile_id";

type Profile = {
	id: string;
	userId: string;
	handle: string;
	displayName: string;
	bio?: string | null;
	createdAt: string;
};

type Post = {
	id: string;
	authorId: string;
	body: string;
	createdAt: string;
};

type Like = {
	id: string;
	postId: string;
	profileId: string;
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
	return (
		<SafeAreaProvider>
			<AppContent />
		</SafeAreaProvider>
	);
}

function AppContent() {
	const [ready, setReady] = useState(false);
	const [profileId, setProfileId] = useState<string | null>(null);
	useEffect(() => {
		(async () => {
			await ensureInit();
			const stored = await AsyncStorage.getItem(PROFILE_KEY);
			setProfileId(stored);
			setReady(true);
		})();
	}, []);
	if (!ready) {
		return (
			<View style={[styles.screen, styles.center]}>
				<ActivityIndicator />
			</View>
		);
	}
	return (
		<Root
			profileId={profileId}
			onProfileChange={async (id) => {
				if (id) await AsyncStorage.setItem(PROFILE_KEY, id);
				else await AsyncStorage.removeItem(PROFILE_KEY);
				setProfileId(id);
			}}
		/>
	);
}

function Root({
	profileId,
	onProfileChange,
}: {
	profileId: string | null;
	onProfileChange: (id: string | null) => void;
}) {
	const { data: profiles = [] } = db.useQuery<Profile>("Profile", {});
	const me = useMemo(
		() => (profileId ? (profiles.find((p) => p.id === profileId) ?? null) : null),
		[profileId, profiles],
	);

	if (!me) {
		return (
			<ProfileSetup
				existingHandles={profiles.map((p) => p.handle)}
				onSaved={(p) => onProfileChange(p.id)}
			/>
		);
	}
	return <Feed me={me} profiles={profiles} />;
}

function Feed({ me, profiles }: { me: Profile; profiles: Profile[] }) {
	const { data: posts = [] } = db.useQuery<Post>("Post", {
		orderBy: { createdAt: "desc" },
		limit: 100,
	});
	const { data: likes = [] } = db.useQuery<Like>("Like", {});

	const profilesById = useMemo(() => {
		const map = new Map<string, Profile>();
		for (const p of profiles) map.set(p.id, p);
		return map;
	}, [profiles]);

	const items = useMemo(
		() =>
			posts.map((post) => {
				const author = profilesById.get(post.authorId) ?? null;
				const postLikes = likes.filter((l) => l.postId === post.id);
				const likedByMe = postLikes.some((l) => l.profileId === me.id);
				return { post, author, likeCount: postLikes.length, likedByMe };
			}),
		[posts, profilesById, likes, me.id],
	);

	const [draft, setDraft] = useState("");
	const [posting, setPosting] = useState(false);

	async function post() {
		const body = draft.trim();
		if (!body) return;
		setDraft("");
		setPosting(true);
		try {
			await callFn("createPost", { body });
		} catch (e) {
			Alert.alert("Post failed", String(e));
			setDraft(body);
		} finally {
			setPosting(false);
		}
	}

	return (
		<SafeAreaView style={styles.screen}>
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

			{items.length === 0 ? (
				<Text style={styles.empty}>No posts yet.</Text>
			) : (
				<FlatList
					data={items}
					keyExtractor={(it) => it.post.id}
					contentContainerStyle={{ paddingTop: 8, paddingBottom: 32 }}
					ItemSeparatorComponent={() => <View style={styles.separator} />}
					renderItem={({ item }) => <Row item={item} myId={me.id} />}
				/>
			)}
		</SafeAreaView>
	);
}

function Row({
	item,
	myId,
}: {
	item: {
		post: Post;
		author: Profile | null;
		likeCount: number;
		likedByMe: boolean;
	};
	myId: string;
}) {
	async function toggleLike() {
		try {
			await callFn("toggleLike", { postId: item.post.id });
		} catch (e) {
			Alert.alert("Like failed", String(e));
		}
	}
	async function remove() {
		try {
			await callFn("deletePost", { id: item.post.id });
		} catch (e) {
			Alert.alert("Delete failed", String(e));
		}
	}
	return (
		<View style={styles.post}>
			<View style={styles.postHead}>
				<Text style={styles.postName}>
					{item.author?.displayName ?? "Unknown"}
				</Text>
				<Text style={styles.postHandle}>@{item.author?.handle ?? "?"}</Text>
			</View>
			<Text style={styles.postBody}>{item.post.body}</Text>
			<View style={styles.postFoot}>
				<Pressable onPress={toggleLike}>
					<Text
						style={[styles.likeBtn, item.likedByMe && styles.likeBtnActive]}
					>
						{item.likedByMe ? "♥" : "♡"} {item.likeCount}
					</Text>
				</Pressable>
				{item.author?.id === myId && (
					<Pressable onPress={remove}>
						<Text style={styles.deleteBtn}>Delete</Text>
					</Pressable>
				)}
			</View>
		</View>
	);
}

function ProfileSetup({
	existingHandles,
	onSaved,
}: {
	existingHandles: string[];
	onSaved: (p: Profile) => void;
}) {
	const [handle, setHandle] = useState("");
	const [displayName, setDisplayName] = useState("");
	const [bio, setBio] = useState("");
	const [saving, setSaving] = useState(false);

	async function save() {
		const lower = handle.trim().toLowerCase();
		if (existingHandles.includes(lower)) {
			Alert.alert("Handle taken", `@${lower} is already in use.`);
			return;
		}
		setSaving(true);
		try {
			const profile = (await callFn("upsertProfile", {
				handle: lower,
				displayName: displayName.trim(),
				bio: bio.trim(),
			})) as Profile;
			onSaved(profile);
		} catch (e) {
			Alert.alert("Save failed", String(e));
		} finally {
			setSaving(false);
		}
	}

	return (
		<SafeAreaView style={styles.screen}>
			<StatusBar style="auto" />
			<View style={styles.content}>
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
		</SafeAreaView>
	);
}

const styles = StyleSheet.create({
	screen: { flex: 1, paddingTop: 64, paddingHorizontal: 20, backgroundColor: "#fff" },
	center: { alignItems: "center", justifyContent: "center" },
	content: { padding: 20, gap: 12 },
	title: { fontSize: 28, fontWeight: "600" },
	subtitle: { color: "#666", marginBottom: 12 },
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
