import React from "react";

// `(chat)` is a route group: the folder name is not part of the URL, so
// `(chat)/page.tsx` serves `/` and `(chat)/c/[id]/page.tsx` serves `/c/:id`.
// The chat fills the viewport and draws its own sidebar and header, so this
// layout adds nothing around it. /login sits outside the group.
export default function ChatLayout({ children }: { children: React.ReactNode }) {
  return <>{children}</>;
}
