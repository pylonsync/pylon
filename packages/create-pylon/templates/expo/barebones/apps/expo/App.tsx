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
} from "react-native";
import { StatusBar } from "expo-status-bar";
import { init, db, callFn } from "@pylonsync/react-native";

// In dev, point at the Pylon control plane on your machine. The iOS
// simulator can reach `localhost`; an Android emulator uses `10.0.2.2`;
// a physical device needs your LAN IP. Override via `PYLON_BASE_URL`
// in app.json `extra` or via an `.env`.
const PYLON_BASE_URL =
	process.env.EXPO_PUBLIC_PYLON_BASE_URL ??
	(Platform.OS === "android" ? "http://10.0.2.2:4321" : "http://localhost:4321");

type Widget = {
	id: string;
	name: string;
	count: number;
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
	return <Home />;
}

function Home() {
	const { data: widgets, loading } = db.useQuery<Widget>("Widget", {});
	const [name, setName] = useState("");
	const [creating, setCreating] = useState(false);

	async function create() {
		const trimmed = name.trim();
		if (!trimmed) return;
		setCreating(true);
		try {
			await callFn("createWidget", { name: trimmed });
			setName("");
		} finally {
			setCreating(false);
		}
	}

	return (
		<View style={styles.screen}>
			<StatusBar style="auto" />
			<Text style={styles.title}>__APP_NAME__</Text>
			<Text style={styles.subtitle}>
				Pylon backend → Expo + React Native, live-updating list.
			</Text>

			<View style={styles.row}>
				<TextInput
					style={styles.input}
					placeholder="Name a widget…"
					value={name}
					onChangeText={setName}
					editable={!creating}
					onSubmitEditing={create}
					autoCorrect={false}
				/>
				<Pressable
					onPress={create}
					disabled={creating || !name.trim()}
					style={({ pressed }) => [
						styles.button,
						(creating || !name.trim()) && styles.buttonDisabled,
						pressed && styles.buttonPressed,
					]}
				>
					<Text style={styles.buttonLabel}>Create</Text>
				</Pressable>
			</View>

			{loading ? (
				<ActivityIndicator style={{ marginTop: 24 }} />
			) : (widgets ?? []).length === 0 ? (
				<Text style={styles.empty}>No widgets yet. Create one above.</Text>
			) : (
				<FlatList
					data={widgets ?? []}
					keyExtractor={(w) => w.id}
					contentContainerStyle={{ paddingTop: 16 }}
					ItemSeparatorComponent={() => <View style={styles.separator} />}
					renderItem={({ item }) => (
						<View style={styles.item}>
							<Text style={styles.itemName}>{item.name}</Text>
							<Text style={styles.itemCount}>count: {item.count}</Text>
						</View>
					)}
				/>
			)}
		</View>
	);
}

const styles = StyleSheet.create({
	screen: {
		flex: 1,
		paddingTop: 64,
		paddingHorizontal: 20,
		backgroundColor: "#fff",
	},
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
		justifyContent: "space-between",
		paddingVertical: 12,
	},
	itemName: { fontSize: 15, fontWeight: "500" },
	itemCount: { fontSize: 12, fontFamily: "Menlo", color: "#999" },
	separator: { height: 1, backgroundColor: "#e5e5e5" },
});
