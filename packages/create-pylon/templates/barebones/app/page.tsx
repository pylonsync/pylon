import React from "react";
import { type Metadata } from "@pylonsync/react";
import { ItemList } from "./items-client";

export const metadata: Metadata = {
  title: "__APP_NAME__ — a minimal Pylon app",
  description:
    "One entity, a live list, and an optimistic create — server-rendered over one Pylon backend, one binary, one port.",
};

// `app/page.tsx` → `/`. The heading is server-rendered; the list is a client
// island (`<ItemList>`) that mints a guest session and runs the live query +
// optimistic create in the browser. This is the whole app — edit
// `app/items-client.tsx` for the UI or `app.ts` for the data model.
export default function IndexPage() {
  return (
    <div className="flex flex-1 flex-col">
      <header className="mb-8">
        <h1 className="text-3xl font-semibold tracking-tight">Items</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          One entity, a live list, optimistic writes. The smallest working
          Pylon app — grow it from here.
        </p>
      </header>
      <ItemList />
    </div>
  );
}
