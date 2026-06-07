import React, { Suspense, use } from "react";
import { type Metadata, type PageProps, type ServerData } from "@pylonsync/react";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

// SEO metadata for `/notes`. (`generateMetadata(props)` is the dynamic
// form when the title depends on params — e.g. a `[slug]` route.)
export const metadata: Metadata = {
  title: "Notes — __APP_NAME__",
  description:
    "A list of notes read from the database during the server render — no client fetch, no loading flash.",
};

// The `Note` entity from app.ts. Type your rows however you like; the
// shape is whatever your entity declares.
interface Note {
  id: string;
  body: string;
  done: boolean;
}

// This component reads the database DURING the render. `serverData.list`
// returns a promise; React 19 `use()` suspends the subtree until it
// resolves on the server, then the HTML streams with the rows already in
// it — no `useEffect`, no client round-trip, no loading flash on first
// paint. The resolved value is replayed into the hydration payload, so the
// browser renders the exact same markup. Reads run through the same policy
// gate as a query function's `ctx.db`; writes are rejected.
function NotesList({ serverData }: { serverData: ServerData }) {
  const notes = use(serverData.list<Note>("Note"));

  if (notes.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        No notes yet. Create one through the auto-generated API:{" "}
        <code className="rounded bg-muted px-1">
          curl -X POST localhost:8787/api/entities/Note -d
          '{"{"}"body":"hello"{"}"}'
        </code>{" "}
        then refresh.
      </p>
    );
  }

  return (
    <ul className="space-y-2">
      {notes.map((note) => (
        <li
          key={note.id}
          className="flex items-center gap-3 rounded-md border px-3 py-2 text-sm"
        >
          <span
            aria-hidden
            className={
              note.done
                ? "text-emerald-600"
                : "text-muted-foreground/50"
            }
          >
            {note.done ? "✓" : "○"}
          </span>
          <span className={note.done ? "line-through text-muted-foreground" : ""}>
            {note.body}
          </span>
        </li>
      ))}
    </ul>
  );
}

// `app/notes/page.tsx` → `/notes`. The page destructures `serverData` out
// of its `PageProps` and hands it to the suspending child. (The same props
// carry `response` for status/redirect/cookies and `params`/`searchParams`
// for the URL — all typed, all from @pylonsync/react.)
export default function NotesPage({ serverData }: PageProps) {
  return (
    <div className="space-y-6">
      <section>
        <h1 className="text-2xl font-semibold tracking-tight">Notes</h1>
        <p className="mt-2 text-muted-foreground">
          These rows are read from the database <em>during the server
          render</em> with <code>serverData</code> + React&apos;s{" "}
          <code>use()</code>. The HTML arrives with the data in it — view
          source and you&apos;ll see the notes in the markup, not an empty
          shell that fetches later.
        </p>
      </section>

      <Card>
        <CardHeader>
          <CardTitle>From the database</CardTitle>
          <CardDescription>
            Server-rendered, then hydrated. Edit{" "}
            <code className="rounded bg-muted px-1">app.ts</code> to change
            the <code className="rounded bg-muted px-1">Note</code> shape.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Suspense
            fallback={
              <p className="text-sm text-muted-foreground">Loading notes…</p>
            }
          >
            <NotesList serverData={serverData} />
          </Suspense>
        </CardContent>
      </Card>
    </div>
  );
}
