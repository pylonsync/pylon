import React from "react";
import { ActivityIndicator } from "react-native";
import { Centered, Screen } from "@/ui";

// "/" renders only while the session boots; the root layout replaces it
// with the right group as soon as the state is known.
export default function Index() {
  return (
    <Screen>
      <Centered>
        <ActivityIndicator />
      </Centered>
    </Screen>
  );
}
