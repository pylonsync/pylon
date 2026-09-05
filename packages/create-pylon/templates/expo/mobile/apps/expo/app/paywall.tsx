import React, { useEffect, useState } from "react";
import { Alert, Linking, Pressable, Text, View } from "react-native";
import { useLocalSearchParams, useRouter } from "expo-router";
import { track } from "@/analytics";
import { usePro } from "@/entitlements";
import { available, offers, purchase, restore, type Offer } from "@/purchases";
import { radius, space, useTheme } from "@/theme";
import { Body, Button, Caption, Screen, Spacer, Title } from "@/ui";

const BENEFITS = [
  "Unlimited notes",
  "Sync across all your devices",
  "Everything we ship next",
];

/**
 * The paywall. Shown once after onboarding, again when the free tier's
 * cap is hit (`reason=limit`), and from Settings. Annual is preselected
 * with its saving called out; a free trial is shown when the store
 * product has one.
 *
 * Products come from RevenueCat's current offering, so prices, trials, and
 * the packages on offer change from the dashboard without an app update.
 */
export default function Paywall() {
  const router = useRouter();
  const t = useTheme();
  const { reason } = useLocalSearchParams<{ reason?: string }>();
  const { pro } = usePro();
  const [list, setList] = useState<Offer[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [busy, setBusy] = useState<"buy" | "restore" | null>(null);

  useEffect(() => {
    track("paywall_shown", { reason: reason ?? "onboarding" });
    void offers().then((o) => {
      setList(o);
      const annual = o.find((x) => x.period === "annual");
      setSelected((annual ?? o[0])?.id ?? null);
    });
  }, [reason]);

  // Purchased (or already Pro): close.
  useEffect(() => {
    if (pro) router.back();
  }, [pro, router]);

  const privacy = process.env.EXPO_PUBLIC_PRIVACY_URL;
  const terms = process.env.EXPO_PUBLIC_TERMS_URL;

  async function buy() {
    const offer = list?.find((o) => o.id === selected);
    if (!offer) return;
    setBusy("buy");
    try {
      const ok = await purchase(offer);
      if (!ok) return;
      // The synced entitlement row closes the modal via the effect above.
    } catch (e) {
      Alert.alert("Purchase failed", e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function restorePurchases() {
    setBusy("restore");
    try {
      const ok = await restore();
      if (!ok) Alert.alert("Nothing to restore", "No previous purchase was found for this store account.");
    } finally {
      setBusy(null);
    }
  }

  function dismiss() {
    track("paywall_dismissed", { reason: reason ?? "onboarding" });
    router.back();
  }

  const chosen = list?.find((o) => o.id === selected);
  const cta = chosen?.trial ? `Start ${chosen.trial} free` : "Continue";

  return (
    <Screen>
      <View style={{ alignItems: "flex-end", minHeight: 44 }}>
        <Pressable onPress={dismiss} hitSlop={12} accessibilityRole="button" accessibilityLabel="Close">
          <Text style={{ color: t.muted, fontSize: 22 }}>×</Text>
        </Pressable>
      </View>
      <Title>{reason === "limit" ? "You've hit the free limit" : "Go Pro"}</Title>
      <Body muted style={{ marginTop: space.sm }}>
        {reason === "limit"
          ? "Upgrade to keep adding notes without limits."
          : "Everything in the free version, without the limits."}
      </Body>
      <Spacer />
      <View style={{ gap: space.sm }}>
        {BENEFITS.map((b) => (
          <View key={b} style={{ flexDirection: "row", gap: space.md, alignItems: "center" }}>
            <Text style={{ color: t.success, fontSize: 16 }}>✓</Text>
            <Body>{b}</Body>
          </View>
        ))}
      </View>
      <Spacer h={space.xxl} />

      {!available() ? (
        <View style={{ padding: space.lg, borderRadius: radius.lg, backgroundColor: t.surface, gap: space.sm }}>
          <Body>Purchases need a development build.</Body>
          <Caption>
            Set EXPO_PUBLIC_REVENUECAT_IOS_KEY / _ANDROID_KEY and run `eas build --profile development`.
            Expo Go cannot load the native purchase module.
          </Caption>
        </View>
      ) : list === null ? (
        <Caption>Loading plans…</Caption>
      ) : list.length === 0 ? (
        <Caption>No products are configured for this offering yet. Add packages to the current offering in RevenueCat.</Caption>
      ) : (
        <View style={{ gap: space.md }}>
          {list.map((o) => {
            const active = o.id === selected;
            return (
              <Pressable
                key={o.id}
                accessibilityRole="radio"
                accessibilityState={{ selected: active }}
                onPress={() => setSelected(o.id)}
                style={{
                  borderWidth: 2,
                  borderColor: active ? t.text : t.border,
                  borderRadius: radius.lg,
                  padding: space.lg,
                  backgroundColor: active ? t.surface : "transparent",
                }}
              >
                <View style={{ flexDirection: "row", justifyContent: "space-between", alignItems: "center" }}>
                  <View>
                    <Text style={{ color: t.text, fontSize: 16, fontWeight: "600", textTransform: "capitalize" }}>
                      {o.period}
                    </Text>
                    {o.trial ? <Caption>{o.trial} free, then {o.price}</Caption> : <Caption>{o.price}</Caption>}
                  </View>
                  {o.period === "annual" ? (
                    <View style={{ backgroundColor: t.text, borderRadius: radius.pill, paddingHorizontal: 10, paddingVertical: 4 }}>
                      <Text style={{ color: t.bg, fontSize: 12, fontWeight: "700" }}>BEST VALUE</Text>
                    </View>
                  ) : null}
                </View>
              </Pressable>
            );
          })}
        </View>
      )}

      <View style={{ flex: 1 }} />
      <Button title={cta} loading={busy === "buy"} disabled={!chosen} onPress={() => void buy()} />
      <Spacer h={space.sm} />
      <Button title="Restore purchases" variant="ghost" loading={busy === "restore"} onPress={() => void restorePurchases()} />
      <Text style={{ color: t.muted, fontSize: 11, textAlign: "center", lineHeight: 16, marginTop: space.sm }}>
        Renews automatically until cancelled. Manage in your store account settings.{" "}
        {terms ? (
          <Text style={{ textDecorationLine: "underline" }} onPress={() => void Linking.openURL(terms)}>
            Terms
          </Text>
        ) : null}
        {terms && privacy ? " · " : ""}
        {privacy ? (
          <Text style={{ textDecorationLine: "underline" }} onPress={() => void Linking.openURL(privacy)}>
            Privacy
          </Text>
        ) : null}
      </Text>
    </Screen>
  );
}
