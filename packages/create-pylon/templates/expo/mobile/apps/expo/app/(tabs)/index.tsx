import React, { useState } from "react";
import { Alert, FlatList, Pressable, Text, View } from "react-native";
import { useRouter } from "expo-router";
import { callFn, db } from "@pylonsync/react-native";
import { track } from "@/analytics";
import { usePro } from "@/entitlements";
import { useAppSession } from "@/session";
import { radius, space, useTheme } from "@/theme";
import { Body, Button, Caption, Field, Screen, Title } from "@/ui";

interface Note {
  id: string;
  ownerId: string;
  title: string;
  body?: string;
  createdAt: string;
}

const FREE_LIMIT = 10;

/**
 * The app's main screen: a live, offline-capable list. Reads come from the
 * local replica (instant on cold start), creates go through `createNote`
 * so the server enforces the free-tier cap, deletes are optimistic.
 */
export default function Notes() {
  const router = useRouter();
  const t = useTheme();
  const { isGuest } = useAppSession();
  const { pro } = usePro();
  const { data: notes = [], loading } = db.useQuery<Note>("Note", { orderBy: { createdAt: "desc" } });
  const [draft, setDraft] = useState("");
  const [saving, setSaving] = useState(false);

  async function add() {
    const title = draft.trim();
    if (!title) return;
    setSaving(true);
    try {
      await callFn("createNote", { title });
      setDraft("");
    } catch (e) {
      const code = (e as { code?: string })?.code ?? "";
      const msg = e instanceof Error ? e.message : String(e);
      if (code === "LIMIT_REACHED" || /LIMIT_REACHED/.test(msg)) {
        track("limit_reached");
        router.push({ pathname: "/paywall", params: { reason: "limit" } });
      } else {
        Alert.alert("Couldn't save", msg);
      }
    } finally {
      setSaving(false);
    }
  }

  async function remove(note: Note) {
    try {
      await db.delete("Note", note.id);
    } catch (e) {
      Alert.alert("Couldn't delete", e instanceof Error ? e.message : String(e));
    }
  }

  const remaining = Math.max(0, FREE_LIMIT - notes.length);

  return (
    <Screen>
      <View style={{ flexDirection: "row", justifyContent: "space-between", alignItems: "center", paddingVertical: space.md }}>
        <Title>Notes</Title>
        {pro ? (
          <View style={{ backgroundColor: t.text, borderRadius: radius.pill, paddingHorizontal: 10, paddingVertical: 4 }}>
            <Text style={{ color: t.bg, fontSize: 12, fontWeight: "700" }}>PRO</Text>
          </View>
        ) : (
          <Pressable onPress={() => router.push("/paywall")} accessibilityRole="button">
            <Caption>{remaining} free left · Upgrade</Caption>
          </Pressable>
        )}
      </View>

      <View style={{ flexDirection: "row", gap: space.sm }}>
        <Field
          placeholder="New note"
          value={draft}
          onChangeText={setDraft}
          onSubmitEditing={() => void add()}
          returnKeyType="done"
          style={{ flex: 1 }}
        />
        <Button title="Add" loading={saving} disabled={!draft.trim()} onPress={() => void add()} style={{ paddingHorizontal: space.lg }} />
      </View>

      {isGuest && notes.length >= 3 ? (
        <Pressable
          onPress={() => router.push("/(auth)/sign-in")}
          style={{ marginTop: space.md, padding: space.md, borderRadius: radius.md, backgroundColor: t.surface }}
        >
          <Caption>Sign in to keep these notes if you switch phones. ›</Caption>
        </Pressable>
      ) : null}

      <FlatList
        data={notes}
        keyExtractor={(n) => n.id}
        contentContainerStyle={{ paddingVertical: space.lg, gap: space.sm }}
        ListEmptyComponent={
          <View style={{ paddingTop: space.xxl, alignItems: "center", gap: space.sm }}>
            <Body muted>{loading ? "Loading…" : "Nothing here yet. Add your first note above."}</Body>
          </View>
        }
        renderItem={({ item }) => (
          <Pressable
            onLongPress={() =>
              Alert.alert("Delete note?", item.title, [
                { text: "Cancel", style: "cancel" },
                { text: "Delete", style: "destructive", onPress: () => void remove(item) },
              ])
            }
            style={{ padding: space.lg, borderRadius: radius.md, backgroundColor: t.surface }}
          >
            <Body>{item.title}</Body>
            <Caption>{new Date(item.createdAt).toLocaleDateString()}</Caption>
          </Pressable>
        )}
      />
    </Screen>
  );
}
