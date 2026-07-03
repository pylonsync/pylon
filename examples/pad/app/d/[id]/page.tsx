import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import EditorIsland from "./EditorIsland";

export const metadata: Metadata = {
  title: "Pad — collaborative markdown on Pylon",
  description:
    "Live co-editing over a Loro text CRDT. Share this URL and type together.",
};

export default function Page({ params }: PageProps<{ id: string }>) {
  return <EditorIsland docId={params.id} />;
}
