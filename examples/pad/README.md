# Pad — collaborative markdown on Pylon

A collaborative markdown editor with one entity, two pages, and one
binary. Open a document in two windows and type; keystrokes merge
character-by-character through a [Loro](https://loro.dev) text CRDT,
with live presence and a rendered preview.

```sh
cd examples/pad
bun install       # from the repo root also works
pylon dev
```

Visit http://localhost:4321. A welcome document seeds itself on first
load. Copy its URL into a second window and type in either.

## How it works

- `app.ts`: the whole data model: a `Doc` entity whose body is
  `field.string().crdt("text")`, plus a policy that lets any signed-in
  session (guests included) co-edit.
- `client/Editor.tsx`: the editor is one hook:
  `useCollabTextarea("Doc", id, "content")` from `@pylonsync/loro`
  (diff-aware splices out, caret-preserving merges in). Presence
  avatars come from `useRoom`.
- `client/markdown.tsx`: a small safe renderer straight to React
  elements: pasted `<script>` is just text.
- `functions/seedPad.ts`: idempotent first-boot seed so the demo is
  never an empty shell.
