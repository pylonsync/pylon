import React from "react";
import type { Metadata } from "@pylonsync/react";
import ArenaIsland from "./ArenaIsland";

export const metadata: Metadata = {
  title: "Shard Arena",
  description: "A realtime shard whose game logic is Rust compiled to WebAssembly.",
};

export default function Page() {
  return <ArenaIsland />;
}
