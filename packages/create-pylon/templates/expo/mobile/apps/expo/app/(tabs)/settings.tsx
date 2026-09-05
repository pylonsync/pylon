import React, { useState } from "react";
import { Alert, Linking, ScrollView, View } from "react-native";
import { useRouter } from "expo-router";
import Constants from "expo-constants";
import { deleteAccount } from "@pylonsync/react-native";
import { track } from "@/analytics";
import { usePro } from "@/entitlements";
import { resetFlags } from "@/flags";
import { manageSubscriptionUrl, restore } from "@/purchases";
import { useAppSession } from "@/session";
import { space } from "@/theme";
import { Caption, Row, Screen, Spacer, Title } from "@/ui";

/**
 * Account, subscription, legal, and support. App Review requires an
 * in-app account deletion path when the app offers account creation, and
 * both stores require the privacy policy and terms links.
 */
export default function Settings() {
  const router = useRouter();
  const { userId, isGuest, signOut } = useAppSession();
  const { pro } = usePro();
  const [busy, setBusy] = useState<string | null>(null);

  const privacy = process.env.EXPO_PUBLIC_PRIVACY_URL;
  const terms = process.env.EXPO_PUBLIC_TERMS_URL;
  const support = process.env.EXPO_PUBLIC_SUPPORT_EMAIL;
  const version = Constants.expoConfig?.version ?? "dev";

  async function restorePurchases() {
    setBusy("restore");
    try {
      const ok = await restore();
      Alert.alert(ok ? "Purchases restored" : "Nothing to restore");
    } finally {
      setBusy(null);
    }
  }

  function confirmDelete() {
    Alert.alert(
      "Delete account?",
      "This removes your account and every note on our servers. Subscriptions are managed by the store and must be cancelled there.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Delete",
          style: "destructive",
          onPress: () =>
            void (async () => {
              setBusy("delete");
              try {
                await deleteAccount();
                track("account_deleted");
                await resetFlags();
                router.replace("/(onboarding)/welcome");
              } catch (e) {
                Alert.alert("Couldn't delete", e instanceof Error ? e.message : String(e));
              } finally {
                setBusy(null);
              }
            })(),
        },
      ],
    );
  }

  return (
    <Screen>
      <ScrollView contentContainerStyle={{ paddingBottom: space.xxl }}>
        <View style={{ paddingVertical: space.md }}>
          <Title>Settings</Title>
        </View>

        <Caption>Account</Caption>
        {isGuest ? (
          <Row label="Sign in or create account" onPress={() => router.push("/(auth)/sign-in")} />
        ) : (
          <>
            <Row label="Signed in" value={userId ?? ""} />
            <Row label="Sign out" onPress={() => void signOut()} />
          </>
        )}
        <Spacer />

        <Caption>Subscription</Caption>
        {pro ? (
          <Row label="Plan" value="Pro" />
        ) : (
          <Row label="Upgrade to Pro" onPress={() => router.push("/paywall")} />
        )}
        {pro ? <Row label="Manage subscription" onPress={() => void Linking.openURL(manageSubscriptionUrl())} /> : null}
        <Row label={busy === "restore" ? "Restoring…" : "Restore purchases"} onPress={() => void restorePurchases()} />
        <Spacer />

        <Caption>About</Caption>
        {privacy ? <Row label="Privacy policy" onPress={() => void Linking.openURL(privacy)} /> : null}
        {terms ? <Row label="Terms of service" onPress={() => void Linking.openURL(terms)} /> : null}
        {support ? <Row label="Contact support" onPress={() => void Linking.openURL(`mailto:${support}`)} /> : null}
        <Row label="Version" value={version} />
        <Spacer />

        {!isGuest ? (
          <>
            <Caption>Danger zone</Caption>
            <Row label={busy === "delete" ? "Deleting…" : "Delete account"} danger onPress={confirmDelete} />
          </>
        ) : null}
        {__DEV__ ? (
          <>
            <Spacer />
            <Caption>Development</Caption>
            <Row
              label="Reset onboarding"
              onPress={() => void resetFlags().then(() => router.replace("/(onboarding)/welcome"))}
            />
          </>
        ) : null}
      </ScrollView>
    </Screen>
  );
}
