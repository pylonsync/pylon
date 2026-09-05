import React from "react";
import { Text } from "react-native";
import { Tabs } from "expo-router";
import { useTheme } from "@/theme";

export default function TabsLayout() {
  const t = useTheme();
  return (
    <Tabs
      screenOptions={{
        headerShown: false,
        tabBarActiveTintColor: t.text,
        tabBarInactiveTintColor: t.muted,
        tabBarStyle: { backgroundColor: t.bg, borderTopColor: t.border },
      }}
    >
      <Tabs.Screen
        name="index"
        options={{ title: "Notes", tabBarIcon: ({ color }) => <Text style={{ color, fontSize: 18 }}>▤</Text> }}
      />
      <Tabs.Screen
        name="settings"
        options={{ title: "Settings", tabBarIcon: ({ color }) => <Text style={{ color, fontSize: 18 }}>⚙</Text> }}
      />
    </Tabs>
  );
}
