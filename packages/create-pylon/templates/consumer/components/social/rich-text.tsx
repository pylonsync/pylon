"use client";

import React from "react";
import { Link } from "@pylonsync/react";
import { splitMentions } from "@/lib/social";

/** Text with each @username linked to its profile. */
export function RichText({ text }: { text: string }) {
  return (
    <>
      {splitMentions(text).map((part, i) =>
        part.kind === "text" ? (
          <React.Fragment key={i}>{part.text}</React.Fragment>
        ) : (
          <Link key={i} href={`/u/${part.username}`} className="text-brand hover:underline">
            @{part.username}
          </Link>
        ),
      )}
    </>
  );
}
