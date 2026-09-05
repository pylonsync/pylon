import React, { useEffect } from "react";
import { Stack, useRouter, useSegments } from "expo-router";
import { StatusBar } from "expo-status-bar";
import * as SplashScreen from "expo-splash-screen";
import { GestureHandlerRootView } from "react-native-gesture-handler";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { SessionProvider, useAppSession } from "@/session";
import { useTheme } from "@/theme";

// Keep the native splash up until the session state is known, so the
// first frame is the right screen instead of a flash of the wrong one.
void SplashScreen.preventAutoHideAsync();

export default function RootLayout() {
  return (
    <GestureHandlerRootView style={{ flex: 1 }}>
      <SafeAreaProvider>
        <SessionProvider>
          <Router />
        </SessionProvider>
      </SafeAreaProvider>
    </GestureHandlerRootView>
  );
}

/**
 * Route guard. The session state decides which group the user may be in:
 * onboarding → (onboarding), signedOut → (auth), ready → (tabs). The
 * paywall is a modal reachable from anywhere once ready.
 */
function Router() {
  const { state } = useAppSession();
  const segments = useSegments();
  const router = useRouter();
  const t = useTheme();

  useEffect(() => {
    if (state === "booting") return;
    void SplashScreen.hideAsync();
    const group = segments[0];
    if (state === "onboarding" && group !== "(onboarding)") {
      router.replace("/(onboarding)/welcome");
    } else if (state === "signedOut" && group !== "(auth)") {
      router.replace("/(auth)/sign-in");
    } else if (state === "ready" && (group === "(onboarding)" || group === undefined)) {
      router.replace("/(tabs)");
    }
  }, [state, segments, router]);

  return (
    <>
      <StatusBar style="auto" />
      <Stack screenOptions={{ headerShown: false, contentStyle: { backgroundColor: t.bg } }}>
        <Stack.Screen name="(onboarding)" />
        <Stack.Screen name="(auth)" />
        <Stack.Screen name="(tabs)" />
        <Stack.Screen name="paywall" options={{ presentation: "modal", headerShown: false }} />
      </Stack>
    </>
  );
}
