import React, { useEffect, useRef, useState } from "react";
import { Alert, TextInput } from "react-native";
import { useLocalSearchParams, useRouter } from "expo-router";
import { sendEmailCode, verifyEmailCode } from "@pylonsync/react-native";
import { track } from "@/analytics";
import { space } from "@/theme";
import { Body, Button, Field, Screen, Spacer, Title } from "@/ui";

/** Enter the 6-digit code that was emailed. Auto-submits on the sixth digit. */
export default function Verify() {
  const router = useRouter();
  const { email, next } = useLocalSearchParams<{ email: string; next?: string }>();
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [resent, setResent] = useState(false);
  const input = useRef<TextInput>(null);

  useEffect(() => {
    const id = setTimeout(() => input.current?.focus(), 300);
    return () => clearTimeout(id);
  }, []);

  async function submit(value: string) {
    if (value.length !== 6 || busy) return;
    setBusy(true);
    try {
      await verifyEmailCode(email, value);
      track("sign_in_completed", { method: "email" });
      if (next) router.replace(next as never);
      else router.replace("/(tabs)");
    } catch (e) {
      setCode("");
      Alert.alert("That code didn't work", e instanceof Error ? e.message : "Try again.");
    } finally {
      setBusy(false);
    }
  }

  async function resend() {
    try {
      await sendEmailCode(email);
      setResent(true);
    } catch (e) {
      Alert.alert("Couldn't resend", e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <Screen>
      <Spacer h={space.xxl} />
      <Title>Check your email</Title>
      <Body muted style={{ marginTop: space.sm }}>
        We sent a 6-digit code to {email}.
      </Body>
      <Spacer h={space.xxl} />
      <Field
        ref={input}
        placeholder="123456"
        keyboardType="number-pad"
        textContentType="oneTimeCode"
        autoComplete="one-time-code"
        maxLength={6}
        value={code}
        onChangeText={(v) => {
          const digits = v.replace(/\D/g, "");
          setCode(digits);
          if (digits.length === 6) void submit(digits);
        }}
        style={{ fontSize: 24, letterSpacing: 8, textAlign: "center" }}
      />
      <Spacer />
      <Button title="Continue" loading={busy} disabled={code.length !== 6} onPress={() => void submit(code)} />
      <Spacer h={space.sm} />
      <Button title={resent ? "Code sent again" : "Resend code"} variant="ghost" disabled={resent} onPress={() => void resend()} />
      <Button title="Use a different email" variant="ghost" onPress={() => router.back()} />
    </Screen>
  );
}
