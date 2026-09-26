// Example conversations for a fresh install with no model provider. The
// `seedExamples` function writes them into a visitor's history (marked
// `example: true`) so the sidebar and thread show real layouts before a key is
// set. The thread labels each one as an example written for the template; none
// of this text came from a model. Delete this file and `functions/seedExamples.ts`
// once your app has a provider.

export interface ExampleMessage {
  role: "user" | "assistant";
  content: string;
}

export interface ExampleConversation {
  title: string;
  // How long ago the conversation happened, in hours. Spreads the examples
  // across the sidebar's day groups.
  hoursAgo: number;
  messages: ExampleMessage[];
}

export const exampleConversations: ExampleConversation[] = [
  {
    title: "Debounce a search input in React",
    hoursAgo: 2,
    messages: [
      {
        role: "user",
        content:
          "My search box fires a request on every keystroke. How do I debounce it in React without a library, and cancel requests that come back out of order?",
      },
      {
        role: "assistant",
        content: `There are two separate problems here, and they need two separate fixes:

1. **Too many requests.** Debounce the *value*, so a request only starts after the user pauses typing.
2. **Out-of-order responses.** A slow response for \`"rea"\` can land after a fast one for \`"react"\`. Cancel the previous request with an \`AbortController\` when a new one starts.

## A \`useDebouncedValue\` hook

Debouncing the value (not the handler) keeps the input responsive: the text field updates on every keystroke, and only the derived value waits.

\`\`\`tsx
import { useEffect, useState } from "react";

export function useDebouncedValue<T>(value: T, delayMs = 250): T {
  const [debounced, setDebounced] = useState(value);

  useEffect(() => {
    const id = setTimeout(() => setDebounced(value), delayMs);
    // A new keystroke clears the pending timer before it fires.
    return () => clearTimeout(id);
  }, [value, delayMs]);

  return debounced;
}
\`\`\`

## Fetch with cancellation

The effect's cleanup runs before the next effect, so aborting there cancels the stale request every time the query changes.

\`\`\`tsx
function Search() {
  const [query, setQuery] = useState("");
  const debounced = useDebouncedValue(query.trim(), 250);
  const [results, setResults] = useState<Result[]>([]);

  useEffect(() => {
    if (!debounced) return setResults([]);
    const controller = new AbortController();

    fetch(\`/api/search?q=\${encodeURIComponent(debounced)}\`, {
      signal: controller.signal,
    })
      .then((res) => res.json())
      .then(setResults)
      .catch((err) => {
        if (err.name !== "AbortError") throw err;
      });

    return () => controller.abort();
  }, [debounced]);

  return <input value={query} onChange={(e) => setQuery(e.target.value)} />;
}
\`\`\`

## Choosing the delay

| Delay | Feels like | Use it for |
| --- | --- | --- |
| 150 ms | Instant | Local filtering, small lists |
| 250 ms | Responsive | Most server search |
| 400 ms+ | Laggy | Expensive queries only |

A few details that matter in production:

- **Trim before debouncing**, so \`"react "\` and \`"react"\` do not trigger two requests.
- **Ignore \`AbortError\`**. It is the expected result of a cancelled request, not a failure.
- **Keep the last results visible** while the next request is in flight. Clearing them on every keystroke makes the list flicker.

If you later add a data library such as TanStack Query, keep the debounced value and pass it as part of the query key. The library then handles cancellation for you.`,
      },
      {
        role: "user",
        content: "Does the cleanup still run if the component unmounts mid-request?",
      },
      {
        role: "assistant",
        content: `Yes. React runs the effect's cleanup on unmount as well as before each re-run, so \`controller.abort()\` fires and the pending \`fetch\` rejects with an \`AbortError\`.

That also means \`setResults\` is never called on an unmounted component, so you do not need an \`isMounted\` flag.`,
      },
    ],
  },
  {
    title: "Index for a status and created_at query",
    hoursAgo: 30,
    messages: [
      {
        role: "user",
        content:
          "This Postgres query is slow on a 40M row table:\n\nSELECT * FROM orders WHERE status = 'pending' ORDER BY created_at DESC LIMIT 50;\n\nWhat index should I add?",
      },
      {
        role: "assistant",
        content: `Add a composite index with the equality column first and the sort column second:

\`\`\`sql
CREATE INDEX CONCURRENTLY orders_status_created_at_idx
  ON orders (status, created_at DESC);
\`\`\`

With this index, Postgres jumps to the \`pending\` entries, reads them already sorted by \`created_at\`, and stops after 50 rows. There is no sort step and no scan of the other statuses.

If \`pending\` is a small share of the table, a **partial index** is smaller and faster to maintain:

\`\`\`sql
CREATE INDEX CONCURRENTLY orders_pending_created_at_idx
  ON orders (created_at DESC)
  WHERE status = 'pending';
\`\`\`

Check the plan after you add it:

\`\`\`sql
EXPLAIN (ANALYZE, BUFFERS)
SELECT * FROM orders
WHERE status = 'pending'
ORDER BY created_at DESC
LIMIT 50;
\`\`\`

You want to see an \`Index Scan\` on the new index and a \`Limit\` node with no \`Sort\` above it. Use \`CONCURRENTLY\` on a live table so the build does not block writes.`,
      },
    ],
  },
  {
    title: "Shorter product update email",
    hoursAgo: 24 * 9,
    messages: [
      {
        role: "user",
        content:
          "Make this shorter: \"We wanted to take a moment to reach out and let you know that, after a lot of careful consideration, we have decided to make some changes to the way our pricing works, which will take effect starting next month.\"",
      },
      {
        role: "assistant",
        content: `Here are three versions, from shortest to most formal:

1. "Our pricing changes next month."
2. "Starting next month, our pricing is changing. Here is what that means for you."
3. "We are updating our pricing, effective next month. The details and your options are below."

The second works best for most customers: it states the change and promises the details in one line. Put the date and the new prices directly under it.`,
      },
    ],
  },
];
