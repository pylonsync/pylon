"use client";

import React, { useEffect, useMemo, useRef, useState } from "react";
import { db, useRoom } from "@pylonsync/react";
import { useCollabTextarea } from "@pylonsync/loro";
import { ArrowLeft, Check, Link2 } from "lucide-react";
import { identityFor, tabIdentity } from "./bootstrap";
import { Markdown } from "./markdown";

interface PeerCursor {
  name: string;
  color: string;
  caret: number | null;
}

export function Editor({ docId, userId }: { docId: string; userId: string }) {
  const { data: doc, loading } = db.useQueryOne("Doc", docId);
  // The collaborative body: minimal-splice diffs out, caret-preserving
  // merges in. `value` doubles as the live source for the preview pane.
  const { ref, value, onInput } = useCollabTextarea("Doc", docId, "content");

  // Presence identity is per-TAB (not per-user): two windows of the
  // same guest are the canonical demo, and useRoom filters peers by
  // user_id — a shared identity would hide the other window entirely.
  const presenceId = useMemo(() => tabIdentity(userId), [userId]);
  const me = useMemo(() => identityFor(presenceId), [presenceId]);
  const { peers, setPresence } = useRoom(`pad:${docId}`, presenceId, {
    initialPresence: { name: me.name, color: me.color, caret: null },
  });

  // Publish the local caret, trailing-throttled so a burst of
  // keystrokes/arrow keys costs ~5 presence updates a second, not one
  // per event.
  const caretTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastSentCaret = useRef<number | null>(null);
  const publishCaret = () => {
    if (caretTimer.current) return;
    caretTimer.current = setTimeout(() => {
      caretTimer.current = null;
      const el = ref.current;
      if (!el) return;
      const caret = el.selectionStart ?? null;
      if (caret === lastSentCaret.current) return;
      lastSentCaret.current = caret;
      setPresence({ name: me.name, color: me.color, caret });
    }, 200);
  };
  useEffect(() => {
    return () => {
      if (caretTimer.current) clearTimeout(caretTimer.current);
    };
  }, []);

  // Scroll tick: remote carets are positioned in content coordinates,
  // so the overlay must re-render when the textarea scrolls.
  const [, setScrollTick] = useState(0);

  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(t);
  }, [copied]);

  async function copyLink() {
    try {
      await navigator.clipboard.writeText(window.location.href);
      setCopied(true);
    } catch {
      // Clipboard unavailable (permissions) — the URL bar still works.
    }
  }

  async function renameDoc(title: string) {
    try {
      await db.update("Doc", docId, {
        title,
        updatedAt: new Date().toISOString(),
      });
    } catch {
      // Best-effort; the CRDT body is the document of record.
    }
  }

  if (!loading && !doc) {
    return (
      <div className="mx-auto max-w-2xl px-6 py-16">
        <p className="text-sm text-zinc-500">Document not found.</p>
        <a href="/" className="mt-2 inline-block text-sm underline">
          Back to documents
        </a>
      </div>
    );
  }

  const title = (doc as { title?: string } | null)?.title ?? "";

  const cursors: PeerCursor[] = peers.map(
    (p: { user_id: string; data: Record<string, unknown> }) => {
      const data = p.data as {
        name?: string;
        color?: string;
        caret?: number | null;
      };
      const fallback = identityFor(p.user_id);
      return {
        name: data.name ?? fallback.name,
        color: data.color ?? fallback.color,
        caret: typeof data.caret === "number" ? data.caret : null,
      };
    },
  );

  return (
    <div className="flex h-screen flex-col">
      <header className="flex h-14 shrink-0 items-center gap-3 border-b border-zinc-200 bg-white px-4">
        <a
          href="/"
          className="rounded p-1.5 text-zinc-400 transition-colors hover:bg-zinc-100 hover:text-zinc-700"
          aria-label="Back to documents"
        >
          <ArrowLeft size={16} />
        </a>
        <input
          defaultValue={title}
          key={title || docId}
          placeholder="Untitled"
          onBlur={(e) => {
            const next = e.target.value.trim();
            if (next && next !== title) void renameDoc(next);
          }}
          className="w-64 rounded border border-transparent bg-transparent px-2 py-1 text-sm font-semibold outline-none transition-colors focus:border-zinc-300 focus:bg-white"
        />

        <div className="ml-auto flex items-center gap-3">
          <div className="flex items-center -space-x-1.5">
            <Avatar name={me.name} color={me.color} me />
            {cursors.map((c, i) => (
              <Avatar key={`${c.name}-${i}`} name={c.name} color={c.color} />
            ))}
          </div>
          <span className="text-xs text-zinc-400">
            {cursors.length === 0
              ? "just you — open this URL in a second window"
              : `${cursors.length + 1} editing`}
          </span>
          <button
            onClick={copyLink}
            className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-zinc-300 bg-white px-3 text-xs font-medium transition-colors hover:bg-zinc-50"
          >
            {copied ? <Check size={13} /> : <Link2 size={13} />}
            {copied ? "Copied" : "Copy link"}
          </button>
        </div>
      </header>

      <main className="grid min-h-0 flex-1 grid-cols-1 md:grid-cols-2">
        <div className="relative h-full min-h-0 border-r border-zinc-200">
          <textarea
            ref={ref}
            defaultValue={value}
            onInput={(e) => {
              onInput(e);
              publishCaret();
            }}
            onKeyUp={publishCaret}
            onClick={publishCaret}
            onSelect={publishCaret}
            onScroll={() => setScrollTick((t) => t + 1)}
            spellCheck={false}
            placeholder="Write markdown…"
            className="h-full w-full resize-none bg-white p-6 font-mono text-[13.5px] leading-relaxed outline-none"
          />
          <CursorOverlay
            textareaRef={ref as React.RefObject<HTMLTextAreaElement | null>}
            value={value}
            cursors={cursors}
          />
        </div>
        <div className="hidden h-full overflow-y-auto bg-zinc-50 p-6 md:block">
          <Markdown source={value} />
        </div>
      </main>
    </div>
  );
}

/**
 * Remote carets over a plain <textarea>. You can't draw inside a
 * textarea, so each peer's caret index is measured against a hidden
 * MIRROR div with identical text metrics (font, padding, width,
 * wrapping): text up to the caret plus a marker span, whose offset is
 * the caret's content-space position. Content coords minus the
 * textarea's scroll offset gives the viewport position for a colored
 * bar + name pill. Positions refresh on every peers/value/scroll
 * render.
 */
function CursorOverlay({
  textareaRef,
  value,
  cursors,
}: {
  textareaRef: React.RefObject<HTMLTextAreaElement | null>;
  value: string;
  cursors: PeerCursor[];
}) {
  const mirrorRef = useRef<HTMLDivElement | null>(null);
  const [positions, setPositions] = useState<
    Array<{ x: number; y: number; height: number; cursor: PeerCursor }>
  >([]);

  useEffect(() => {
    const el = textareaRef.current;
    const mirror = mirrorRef.current;
    if (!el || !mirror) {
      setPositions([]);
      return;
    }
    const style = window.getComputedStyle(el);
    for (const prop of [
      "fontFamily",
      "fontSize",
      "fontWeight",
      "lineHeight",
      "letterSpacing",
      "paddingTop",
      "paddingRight",
      "paddingBottom",
      "paddingLeft",
    ] as const) {
      mirror.style[prop] = style[prop];
    }
    mirror.style.width = `${el.clientWidth}px`;
    const lineHeight = parseFloat(style.lineHeight) || 22;

    const next: Array<{
      x: number;
      y: number;
      height: number;
      cursor: PeerCursor;
    }> = [];
    for (const cursor of cursors) {
      if (cursor.caret === null) continue;
      const caret = Math.min(cursor.caret, value.length);
      mirror.textContent = value.slice(0, caret);
      const marker = document.createElement("span");
      marker.textContent = "​";
      mirror.appendChild(marker);
      const x = marker.offsetLeft - el.scrollLeft;
      const y = marker.offsetTop - el.scrollTop;
      // Cull carets scrolled out of view.
      if (y < -lineHeight || y > el.clientHeight) continue;
      next.push({ x, y, height: lineHeight, cursor });
    }
    setPositions(next);
    // cursors is a fresh array every render; stringify for a stable dep.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, JSON.stringify(cursors), textareaRef]);

  return (
    <>
      <div
        ref={mirrorRef}
        aria-hidden
        className="invisible absolute inset-0 overflow-hidden whitespace-pre-wrap break-words"
      />
      <div className="pointer-events-none absolute inset-0 overflow-hidden">
        {positions.map(({ x, y, height, cursor }, i) => (
          <div
            key={`${cursor.name}-${i}`}
            className="absolute transition-all duration-75"
            style={{ transform: `translate(${x}px, ${y}px)` }}
          >
            <div
              className="w-0.5"
              style={{ height, backgroundColor: cursor.color }}
            />
            <div
              className="absolute -top-4 left-0 rounded px-1 py-px text-[10px] leading-tight font-medium whitespace-nowrap text-white"
              style={{ backgroundColor: cursor.color }}
            >
              {cursor.name}
            </div>
          </div>
        ))}
      </div>
    </>
  );
}

function Avatar({
  name,
  color,
  me = false,
}: {
  name: string;
  color: string;
  me?: boolean;
}) {
  return (
    <span
      title={me ? `${name} (you)` : name}
      style={{ backgroundColor: color }}
      className="inline-flex h-7 w-7 items-center justify-center rounded-full border-2 border-white text-[11px] font-semibold text-white"
    >
      {name.slice(0, 1)}
    </span>
  );
}
