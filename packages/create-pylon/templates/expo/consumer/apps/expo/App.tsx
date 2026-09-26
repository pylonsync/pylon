import { useCallback, useEffect, useMemo, useState } from "react";
import {
	ActivityIndicator,
	Alert,
	FlatList,
	Image,
	Platform,
	Pressable,
	StyleSheet,
	Text,
	TextInput,
	View,
	useWindowDimensions,
} from "react-native";
import { SafeAreaProvider, SafeAreaView } from "react-native-safe-area-context";
import { StatusBar } from "expo-status-bar";
import * as ImagePicker from "expo-image-picker";
import {
	callFn,
	db,
	getMe,
	init,
	passwordLogin,
	passwordRegister,
	signOut,
	uploadFile,
} from "@pylonsync/react-native";

// The Pylon backend in apps/api. The iOS simulator reaches the Mac's
// localhost; the Android emulator reaches it at 10.0.2.2. On a device, set
// EXPO_PUBLIC_PYLON_BASE_URL to the Mac's LAN address or your deployed API.
const PYLON_BASE_URL =
	process.env.EXPO_PUBLIC_PYLON_BASE_URL ??
	(Platform.OS === "android" ? "http://10.0.2.2:4321" : "http://localhost:4321");

type Profile = {
	id: string;
	userId: string;
	handle: string;
	displayName: string;
	bio?: string | null;
	avatarUrl?: string | null;
	createdAt: string;
};

type Post = {
	id: string;
	authorId: string;
	imageUrl: string;
	caption?: string | null;
	createdAt: string;
};

type Like = {
	id: string;
	postId: string;
	profileId: string;
	createdAt: string;
};

type Comment = {
	id: string;
	postId: string;
	profileId: string;
	text: string;
	createdAt: string;
};

let initPromise: Promise<void> | null = null;
function ensureInit() {
	if (!initPromise) {
		initPromise = init({ baseUrl: PYLON_BASE_URL, appName: "__APP_NAME_SNAKE__" });
	}
	return initPromise;
}

/** Absolute URL for a stored image path (`/images/...` or `/api/files/<id>`). */
function mediaUrl(path?: string | null): string | undefined {
	if (!path) return undefined;
	return /^https?:\/\//.test(path) ? path : `${PYLON_BASE_URL}${path}`;
}

/** "now", "5m", "2h", "3d", "2w", then the date. */
function shortTime(iso: string): string {
	const seconds = Math.max(0, (Date.now() - Date.parse(iso)) / 1000);
	if (seconds < 60) return "now";
	if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
	if (seconds < 86400) return `${Math.floor(seconds / 3600)}h`;
	if (seconds < 604800) return `${Math.floor(seconds / 86400)}d`;
	if (seconds < 2419200) return `${Math.floor(seconds / 604800)}w`;
	return new Date(iso).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function errorText(e: unknown): string {
	const message = e instanceof Error ? e.message : String(e);
	return message || "Something went wrong. Try again.";
}

type Phase = "loading" | "signedOut" | "needsProfile" | "ready";

export default function App() {
	return (
		<SafeAreaProvider>
			<StatusBar style="auto" />
			<AppContent />
		</SafeAreaProvider>
	);
}

function AppContent() {
	const [phase, setPhase] = useState<Phase>("loading");
	const [profileId, setProfileId] = useState<string | null>(null);

	const resolveProfile = useCallback(async () => {
		const me = (await callFn("myProfile", {})) as Profile | null;
		setProfileId(me?.id ?? null);
		setPhase(me ? "ready" : "needsProfile");
	}, []);

	useEffect(() => {
		(async () => {
			try {
				await ensureInit();
				// Demo content for a new install. The server writes it once.
				await callFn("seedDemo", {}).catch(() => undefined);
				const me = await getMe();
				if (!me?.user_id) {
					setPhase("signedOut");
					return;
				}
				await resolveProfile();
			} catch {
				// The server cannot be reached. Sign-in shows the error on retry.
				setPhase("signedOut");
			}
		})();
	}, [resolveProfile]);

	if (phase === "loading") {
		return (
			<View style={[styles.fill, styles.center]}>
				<ActivityIndicator />
			</View>
		);
	}
	if (phase === "signedOut") {
		return <AuthScreen onSignedIn={resolveProfile} />;
	}
	if (phase === "needsProfile" || !profileId) {
		return (
			<ProfileSetup
				onSaved={(p) => {
					setProfileId(p.id);
					setPhase("ready");
				}}
			/>
		);
	}
	return (
		<Feed
			myProfileId={profileId}
			onSignOut={async () => {
				await signOut();
				setProfileId(null);
				setPhase("signedOut");
			}}
		/>
	);
}

function AuthScreen({ onSignedIn }: { onSignedIn: () => Promise<void> }) {
	const [mode, setMode] = useState<"signIn" | "signUp">("signIn");
	const [email, setEmail] = useState("");
	const [password, setPassword] = useState("");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string | null>(null);

	const canSubmit = email.includes("@") && (mode === "signIn" ? password.length > 0 : password.length >= 8);

	async function submit() {
		if (!canSubmit || busy) return;
		setBusy(true);
		setError(null);
		try {
			const address = email.trim().toLowerCase();
			if (mode === "signIn") await passwordLogin(address, password);
			else await passwordRegister(address, password);
			await onSignedIn();
		} catch (e) {
			setError(errorText(e));
		} finally {
			setBusy(false);
		}
	}

	return (
		<SafeAreaView style={[styles.fill, styles.authScreen]}>
			<Text style={styles.wordmark}>__APP_NAME__</Text>
			<Text style={styles.muted}>
				{mode === "signIn" ? "Sign in to see photos from people you follow." : "Create an account to share your photos."}
			</Text>
			<TextInput
				style={styles.field}
				placeholder="Email"
				autoCapitalize="none"
				autoCorrect={false}
				keyboardType="email-address"
				textContentType="emailAddress"
				value={email}
				onChangeText={setEmail}
			/>
			<TextInput
				style={styles.field}
				placeholder="Password"
				secureTextEntry
				textContentType={mode === "signIn" ? "password" : "newPassword"}
				value={password}
				onChangeText={setPassword}
				onSubmitEditing={submit}
			/>
			{error ? <Text style={styles.error}>{error}</Text> : null}
			<Pressable
				onPress={submit}
				disabled={!canSubmit || busy}
				style={({ pressed }) => [styles.primary, (!canSubmit || busy) && styles.dim, pressed && styles.pressed]}
			>
				{busy ? (
					<ActivityIndicator color="#fff" />
				) : (
					<Text style={styles.primaryLabel}>{mode === "signIn" ? "Log in" : "Create account"}</Text>
				)}
			</Pressable>
			<Pressable onPress={() => setMode(mode === "signIn" ? "signUp" : "signIn")}>
				<Text style={styles.switchText}>
					{mode === "signIn" ? "No account yet? " : "Already have an account? "}
					<Text style={styles.link}>{mode === "signIn" ? "Sign up" : "Log in"}</Text>
				</Text>
			</Pressable>
		</SafeAreaView>
	);
}

function ProfileSetup({ onSaved }: { onSaved: (p: Profile) => void }) {
	const [displayName, setDisplayName] = useState("");
	const [handle, setHandle] = useState("");
	const [bio, setBio] = useState("");
	const [saving, setSaving] = useState(false);

	const canSave = displayName.trim().length > 0 && handle.length >= 2;

	async function save() {
		if (!canSave || saving) return;
		setSaving(true);
		try {
			const profile = (await callFn("upsertProfile", {
				handle,
				displayName: displayName.trim(),
				bio: bio.trim(),
			})) as Profile;
			onSaved(profile);
		} catch (e) {
			Alert.alert("Could not save your profile", errorText(e));
		} finally {
			setSaving(false);
		}
	}

	return (
		<SafeAreaView style={[styles.fill, styles.authScreen]}>
			<Text style={styles.screenTitle}>Create your profile</Text>
			<TextInput style={styles.field} placeholder="Your name" value={displayName} onChangeText={setDisplayName} />
			<TextInput
				style={styles.field}
				placeholder="username"
				autoCapitalize="none"
				autoCorrect={false}
				value={handle}
				onChangeText={(v) => setHandle(v.toLowerCase().replace(/[^a-z0-9._]/g, ""))}
			/>
			<TextInput style={styles.field} placeholder="A line about you" value={bio} onChangeText={setBio} maxLength={150} />
			<Pressable
				onPress={save}
				disabled={!canSave || saving}
				style={({ pressed }) => [styles.primary, (!canSave || saving) && styles.dim, pressed && styles.pressed]}
			>
				{saving ? <ActivityIndicator color="#fff" /> : <Text style={styles.primaryLabel}>Continue</Text>}
			</Pressable>
		</SafeAreaView>
	);
}

function Feed({ myProfileId, onSignOut }: { myProfileId: string; onSignOut: () => void }) {
	// Live queries: a post, like, or comment from any client re-renders this.
	const { data: profiles = [] } = db.useQuery<Profile>("Profile", {});
	const { data: posts = [] } = db.useQuery<Post>("Post", { orderBy: { createdAt: "desc" }, limit: 100 });
	const { data: likes = [] } = db.useQuery<Like>("Like", {});
	const { data: comments = [] } = db.useQuery<Comment>("Comment", {});
	const [posting, setPosting] = useState(false);

	const profilesById = useMemo(() => new Map(profiles.map((p) => [p.id, p])), [profiles]);
	const likesByPost = useMemo(() => {
		const map = new Map<string, Like[]>();
		for (const like of likes) map.set(like.postId, [...(map.get(like.postId) ?? []), like]);
		return map;
	}, [likes]);
	const commentCounts = useMemo(() => {
		const map = new Map<string, number>();
		for (const c of comments) map.set(c.postId, (map.get(c.postId) ?? 0) + 1);
		return map;
	}, [comments]);

	async function newPost() {
		const picked = await ImagePicker.launchImageLibraryAsync({
			mediaTypes: ["images"],
			quality: 0.8,
			allowsEditing: true,
			aspect: [4, 5],
		});
		if (picked.canceled || !picked.assets[0]) return;
		setPosting(true);
		try {
			const blob = await (await fetch(picked.assets[0].uri)).blob();
			const file = await uploadFile(blob, { filename: "photo.jpg", contentType: "image/jpeg", visibility: "public" });
			// Store the server path. With S3 or Stack0 storage `file.url` is a CDN
			// address; `/api/files/<id>` works for every backend.
			await callFn("createPost", { imageUrl: `/api/files/${file.id}`, caption: "" });
		} catch (e) {
			Alert.alert("Could not share the photo", errorText(e));
		} finally {
			setPosting(false);
		}
	}

	return (
		<SafeAreaView style={styles.fill} edges={["top"]}>
			<View style={styles.header}>
				<Text style={styles.wordmarkSmall}>__APP_NAME__</Text>
				<View style={styles.headerActions}>
					<Pressable onPress={newPost} disabled={posting} hitSlop={8}>
						{posting ? <ActivityIndicator /> : <Text style={styles.headerAction}>New post</Text>}
					</Pressable>
					<Pressable onPress={onSignOut} hitSlop={8}>
						<Text style={styles.headerActionMuted}>Sign out</Text>
					</Pressable>
				</View>
			</View>
			<FlatList
				data={posts}
				keyExtractor={(p) => p.id}
				ListEmptyComponent={<Text style={styles.empty}>No posts yet.</Text>}
				renderItem={({ item }) => {
					const postLikes = likesByPost.get(item.id) ?? [];
					return (
						<PostRow
							post={item}
							author={profilesById.get(item.authorId) ?? null}
							likeCount={postLikes.length}
							likedByMe={postLikes.some((l) => l.profileId === myProfileId)}
							commentCount={commentCounts.get(item.id) ?? 0}
						/>
					);
				}}
			/>
		</SafeAreaView>
	);
}

function PostRow({
	post,
	author,
	likeCount,
	likedByMe,
	commentCount,
}: {
	post: Post;
	author: Profile | null;
	likeCount: number;
	likedByMe: boolean;
	commentCount: number;
}) {
	const { width } = useWindowDimensions();
	const [lastTap, setLastTap] = useState(0);

	async function setLike(liked: boolean) {
		try {
			await callFn("setLike", { postId: post.id, liked });
		} catch (e) {
			Alert.alert("Could not update the like", errorText(e));
		}
	}

	function onPhotoPress() {
		const now = Date.now();
		if (now - lastTap < 300 && !likedByMe) void setLike(true);
		setLastTap(now);
	}

	return (
		<View style={styles.post}>
			<View style={styles.postHead}>
				{author?.avatarUrl ? (
					<Image source={{ uri: mediaUrl(author.avatarUrl) }} style={styles.avatar} />
				) : (
					<View style={[styles.avatar, styles.avatarFallback]}>
						<Text style={styles.avatarLetter}>{(author?.displayName ?? "?").slice(0, 1).toUpperCase()}</Text>
					</View>
				)}
				<Text style={styles.handle}>{author?.handle ?? ""}</Text>
				<Text style={styles.muted}>{shortTime(post.createdAt)}</Text>
			</View>
			<Pressable onPress={onPhotoPress}>
				<Image source={{ uri: mediaUrl(post.imageUrl) }} style={{ width, height: width * 1.25 }} resizeMode="cover" />
			</Pressable>
			<View style={styles.postBody}>
				<Pressable onPress={() => setLike(!likedByMe)} hitSlop={8}>
					<Text style={[styles.likeLabel, likedByMe && styles.liked]}>{likedByMe ? "Liked" : "Like"}</Text>
				</Pressable>
				{likeCount > 0 ? <Text style={styles.bold}>{likeCount === 1 ? "1 like" : `${likeCount} likes`}</Text> : null}
				{post.caption ? (
					<Text>
						<Text style={styles.bold}>{author?.handle ?? ""} </Text>
						{post.caption}
					</Text>
				) : null}
				{commentCount > 0 ? (
					<Text style={styles.muted}>{commentCount === 1 ? "1 comment" : `${commentCount} comments`}</Text>
				) : null}
			</View>
		</View>
	);
}

const styles = StyleSheet.create({
	fill: { flex: 1, backgroundColor: "#fff" },
	center: { alignItems: "center", justifyContent: "center" },
	authScreen: { paddingHorizontal: 24, justifyContent: "center", gap: 12 },
	wordmark: { fontSize: 44, fontWeight: "600", fontStyle: "italic", textAlign: "center", fontFamily: Platform.OS === "ios" ? "Georgia" : "serif" },
	wordmarkSmall: { fontSize: 26, fontWeight: "600", fontStyle: "italic", fontFamily: Platform.OS === "ios" ? "Georgia" : "serif" },
	screenTitle: { fontSize: 22, fontWeight: "600", marginBottom: 8 },
	muted: { color: "#8e8e93", textAlign: "left" },
	field: { backgroundColor: "#f2f2f7", borderRadius: 12, paddingHorizontal: 14, height: 50, fontSize: 16 },
	error: { color: "#ff3b30" },
	primary: { backgroundColor: "#0a84ff", borderRadius: 12, height: 50, alignItems: "center", justifyContent: "center" },
	primaryLabel: { color: "#fff", fontSize: 16, fontWeight: "600" },
	dim: { opacity: 0.6 },
	pressed: { opacity: 0.8 },
	switchText: { textAlign: "center", color: "#8e8e93", marginTop: 6 },
	link: { color: "#0a84ff", fontWeight: "600" },
	header: {
		height: 50,
		paddingHorizontal: 16,
		flexDirection: "row",
		alignItems: "center",
		justifyContent: "space-between",
		borderBottomWidth: StyleSheet.hairlineWidth,
		borderBottomColor: "#d1d1d6",
	},
	headerActions: { flexDirection: "row", gap: 18, alignItems: "center" },
	headerAction: { fontWeight: "600", color: "#0a84ff" },
	headerActionMuted: { color: "#8e8e93" },
	empty: { textAlign: "center", color: "#8e8e93", marginTop: 48 },
	post: { paddingBottom: 16 },
	postHead: { flexDirection: "row", alignItems: "center", gap: 8, paddingHorizontal: 12, paddingVertical: 10 },
	avatar: { width: 32, height: 32, borderRadius: 16 },
	avatarFallback: { backgroundColor: "#5856d6", alignItems: "center", justifyContent: "center" },
	avatarLetter: { color: "#fff", fontWeight: "600" },
	handle: { fontWeight: "600" },
	postBody: { paddingHorizontal: 14, paddingTop: 10, gap: 4 },
	likeLabel: { fontWeight: "600", color: "#1c1c1e" },
	liked: { color: "#ff3040" },
	bold: { fontWeight: "600" },
});
