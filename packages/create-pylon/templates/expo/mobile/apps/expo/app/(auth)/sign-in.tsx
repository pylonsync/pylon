import React, { useEffect, useState } from "react";
import { Alert, Platform, Text, View } from "react-native";
import { useLocalSearchParams, useRouter } from "expo-router";
import { nativeSignIn, sendEmailCode } from "@pylonsync/react-native";
import { track } from "@/analytics";
import { useAppSession } from "@/session";
import { space, useTheme } from "@/theme";
import { Body, Button, Caption, Field, Screen, Spacer, Title } from "@/ui";

type AppleAuth = typeof import("expo-apple-authentication");
type GoogleSignin = typeof import("@react-native-google-signin/google-signin");

function loadApple(): AppleAuth | null {
  if (Platform.OS !== "ios") return null;
  try {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    return require("expo-apple-authentication") as AppleAuth;
  } catch {
    return null;
  }
}

function loadGoogle(): GoogleSignin | null {
  if (!process.env.EXPO_PUBLIC_GOOGLE_WEB_CLIENT_ID) return null;
  try {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    return require("@react-native-google-signin/google-signin") as GoogleSignin;
  } catch {
    return null;
  }
}

/**
 * Sign in or create an account. Every method lands on the same server
 * session; a guest's data merges into the account on first sign-in.
 *
 *   Apple   iOS only. Required by App Review when any third-party sign-in
 *           is offered. Reveals the name only on the first sign-in, which
 *           is forwarded to the server.
 *   Google  when EXPO_PUBLIC_GOOGLE_WEB_CLIENT_ID is set.
 *   Email   a 6-digit code; no password to remember.
 */
export default function SignIn() {
  const router = useRouter();
  const t = useTheme();
  const params = useLocalSearchParams<{ next?: string }>();
  const { continueAsGuest, state } = useAppSession();
  const [email, setEmail] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [appleAvailable, setAppleAvailable] = useState(false);
  const apple = loadApple();
  const google = loadGoogle();

  useEffect(() => {
    void apple?.isAvailableAsync().then(setAppleAvailable);
    google?.GoogleSignin.configure({
      webClientId: process.env.EXPO_PUBLIC_GOOGLE_WEB_CLIENT_ID,
      iosClientId: process.env.EXPO_PUBLIC_GOOGLE_IOS_CLIENT_ID,
    });
  }, [apple, google]);

  function done() {
    track("sign_in_completed");
    if (params.next) router.replace(params.next as never);
    else router.replace("/(tabs)");
  }

  async function withApple() {
    if (!apple) return;
    setBusy("apple");
    track("sign_in_started", { method: "apple" });
    try {
      const credential = await apple.signInAsync({
        requestedScopes: [
          apple.AppleAuthenticationScope.FULL_NAME,
          apple.AppleAuthenticationScope.EMAIL,
        ],
      });
      if (!credential.identityToken) throw new Error("Apple returned no identity token");
      const name = [credential.fullName?.givenName, credential.fullName?.familyName]
        .filter(Boolean)
        .join(" ");
      await nativeSignIn("apple", credential.identityToken, name || undefined);
      done();
    } catch (e) {
      if ((e as { code?: string })?.code !== "ERR_REQUEST_CANCELED") {
        Alert.alert("Sign in failed", messageOf(e));
      }
    } finally {
      setBusy(null);
    }
  }

  async function withGoogle() {
    if (!google) return;
    setBusy("google");
    track("sign_in_started", { method: "google" });
    try {
      await google.GoogleSignin.hasPlayServices();
      const result = await google.GoogleSignin.signIn();
      const idToken = result.type === "success" ? result.data.idToken : null;
      if (!idToken) return;
      await nativeSignIn("google", idToken, result.type === "success" ? result.data.user.name : undefined);
      done();
    } catch (e) {
      Alert.alert("Sign in failed", messageOf(e));
    } finally {
      setBusy(null);
    }
  }

  async function withEmail() {
    const address = email.trim().toLowerCase();
    if (!address.includes("@")) {
      Alert.alert("Enter your email address");
      return;
    }
    setBusy("email");
    track("sign_in_started", { method: "email" });
    try {
      await sendEmailCode(address);
      router.push({ pathname: "/(auth)/verify", params: { email: address, next: params.next ?? "" } });
    } catch (e) {
      Alert.alert("Couldn't send the code", messageOf(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <Screen>
      <Spacer h={space.xxl} />
      <Title>Sign in</Title>
      <Body muted style={{ marginTop: space.sm }}>
        Keep your notes on every device. Anything you made already comes with you.
      </Body>
      <Spacer h={space.xxl} />
      <View style={{ gap: space.md }}>
        {appleAvailable && apple ? (
          <apple.AppleAuthenticationButton
            buttonType={apple.AppleAuthenticationButtonType.CONTINUE}
            buttonStyle={
              t.bg === "#ffffff"
                ? apple.AppleAuthenticationButtonStyle.BLACK
                : apple.AppleAuthenticationButtonStyle.WHITE
            }
            cornerRadius={12}
            style={{ height: 52 }}
            onPress={() => void withApple()}
          />
        ) : null}
        {google ? (
          <Button title="Continue with Google" variant="secondary" loading={busy === "google"} onPress={() => void withGoogle()} />
        ) : null}
        <View style={{ flexDirection: "row", alignItems: "center", gap: space.md, marginVertical: space.sm }}>
          <View style={{ flex: 1, height: 1, backgroundColor: t.border }} />
          <Caption>or</Caption>
          <View style={{ flex: 1, height: 1, backgroundColor: t.border }} />
        </View>
        <Field
          placeholder="you@example.com"
          autoCapitalize="none"
          autoCorrect={false}
          keyboardType="email-address"
          textContentType="emailAddress"
          autoComplete="email"
          value={email}
          onChangeText={setEmail}
          onSubmitEditing={() => void withEmail()}
          returnKeyType="send"
        />
        <Button title="Email me a code" loading={busy === "email"} onPress={() => void withEmail()} />
      </View>
      <Spacer />
      {state === "signedOut" ? (
        <Button
          title="Continue without an account"
          variant="ghost"
          onPress={() => void continueAsGuest().then(() => router.replace("/(tabs)"))}
        />
      ) : (
        <Button title="Not now" variant="ghost" onPress={() => router.back()} />
      )}
      <Spacer />
      <Text style={{ color: t.muted, fontSize: 12, textAlign: "center", lineHeight: 18 }}>
        By continuing you agree to the Terms and Privacy Policy.
      </Text>
    </Screen>
  );
}

function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
