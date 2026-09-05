import React, { useEffect, useRef, useState } from "react";
import { Dimensions, FlatList, Pressable, Text, View, type ViewToken } from "react-native";
import { useRouter } from "expo-router";
import { track } from "@/analytics";
import { setPaywallSeenAt } from "@/flags";
import { useAppSession } from "@/session";
import { space, useTheme } from "@/theme";
import { Body, Button, Screen, Title } from "@/ui";

/**
 * Three value slides, then "Get started". Replace the copy with your own;
 * keep it to one outcome per slide. The last tap creates a guest session
 * (no sign-up wall) and opens the paywall once, then lands in the app.
 */
const SLIDES = [
  {
    title: "Capture anything, instantly",
    body: "Notes sync to every device the moment you type them. Works offline.",
  },
  {
    title: "Built for speed",
    body: "No accounts to create before you start. Sign in later to keep everything.",
  },
  {
    title: "Go Pro when you're ready",
    body: "Unlimited notes and everything we ship next. Cancel any time.",
  },
];

const { width } = Dimensions.get("window");

export default function Welcome() {
  const router = useRouter();
  const t = useTheme();
  const { completeOnboarding } = useAppSession();
  const [index, setIndex] = useState(0);
  const [busy, setBusy] = useState(false);
  const list = useRef<FlatList>(null);

  useEffect(() => {
    track("onboarding_started");
  }, []);

  const onViewable = useRef(({ viewableItems }: { viewableItems: ViewToken[] }) => {
    const first = viewableItems[0]?.index;
    if (typeof first === "number") setIndex(first);
  }).current;

  async function finish() {
    setBusy(true);
    try {
      await completeOnboarding();
      track("onboarding_completed");
      await setPaywallSeenAt();
      // Show the paywall once, right after onboarding: the highest-intent
      // moment. It is dismissible; the free tier keeps working.
      router.replace("/(tabs)");
      router.push("/paywall");
    } finally {
      setBusy(false);
    }
  }

  const last = index === SLIDES.length - 1;

  return (
    <Screen padded={false}>
      <View style={{ alignItems: "flex-end", paddingHorizontal: space.xl, minHeight: 44 }}>
        {!last && (
          <Pressable onPress={finish} hitSlop={12} accessibilityRole="button">
            <Text style={{ color: t.muted, fontSize: 15 }}>Skip</Text>
          </Pressable>
        )}
      </View>
      <FlatList
        ref={list}
        data={SLIDES}
        keyExtractor={(s) => s.title}
        horizontal
        pagingEnabled
        showsHorizontalScrollIndicator={false}
        onViewableItemsChanged={onViewable}
        viewabilityConfig={{ itemVisiblePercentThreshold: 60 }}
        renderItem={({ item }) => (
          <View style={{ width, paddingHorizontal: space.xl, justifyContent: "center" }}>
            <View
              style={{
                height: 220,
                borderRadius: 24,
                backgroundColor: t.surface,
                marginBottom: space.xxl,
              }}
            />
            <Title>{item.title}</Title>
            <Body muted style={{ marginTop: space.md }}>
              {item.body}
            </Body>
          </View>
        )}
      />
      <View style={{ paddingHorizontal: space.xl, paddingBottom: space.lg, gap: space.lg }}>
        <View style={{ flexDirection: "row", justifyContent: "center", gap: 6 }}>
          {SLIDES.map((s, i) => (
            <View
              key={s.title}
              style={{
                width: i === index ? 18 : 6,
                height: 6,
                borderRadius: 3,
                backgroundColor: i === index ? t.text : t.border,
              }}
            />
          ))}
        </View>
        <Button
          title={last ? "Get started" : "Next"}
          loading={busy}
          onPress={() =>
            last ? void finish() : list.current?.scrollToIndex({ index: index + 1, animated: true })
          }
        />
      </View>
    </Screen>
  );
}
