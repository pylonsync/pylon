"use client";

import React, { useEffect, useState } from "react";
import { bootstrap } from "../../../client/bootstrap";
import { Editor } from "../../../client/Editor";

export default function EditorIsland({ docId }: { docId: string }) {
  const [userId, setUserId] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    void bootstrap().then((uid) => {
      if (alive) setUserId(uid);
    });
    return () => {
      alive = false;
    };
  }, []);

  if (userId === null) {
    return (
      <div className="mx-auto max-w-6xl px-6 py-16 text-sm text-zinc-400">
        Connecting…
      </div>
    );
  }
  return <Editor docId={docId} userId={userId} />;
}
