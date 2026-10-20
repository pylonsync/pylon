/**
 * Ranges of a ProseMirror document bound with ProseMirrorSyncPlugin, held as
 * two Loro cursors each: comment threads, review findings, highlights. The
 * start binds to the range's first character and the end to its last, so a
 * range stays on its words while anyone edits around it, grows when someone
 * types inside it, and does not take in text typed just before or after it.
 * Ranges may overlap (they are decorations, not marks).
 *
 *   // When a person comments on the selection:
 *   const anchor = encodeRange(view.state, doc, containerId, from, to);
 *   // store anchor.from and anchor.to with the thread
 *
 *   // In the editor:
 *   plugins: [ProseMirrorRangesPlugin({ doc, containerId, onReanchor: (id, anchor) => save(id, anchor) })]
 *   setAnchoredRanges(view, threads.map((t) => ({ id: t.id, from: t.from, to: t.to, className: "thread" })));
 *
 *   // For a margin: where each range is now, or `gone`.
 *   resolveRanges(view.state, doc, containerId, ranges)
 *
 *   // In a server function, with no editor: positions in the document
 *   // readProseMirror (or editProseMirror's state) gives for the doc now.
 *   resolveRangesInDoc(doc, "body", schema, ranges)
 *
 * The binding writes some edits as a delete and an insert: splitting or
 * joining paragraphs moves text into new characters, and a cursor on a moved
 * character no longer names it. The editor that makes such an edit still
 * knows where each range went, from its own transaction: it keeps drawing
 * the range there and calls `onReanchor` with new cursors (an undo too).
 * Store them in place of the old ones, so every other editor finds the range
 * again. Until then, other editors draw a range only where its cursors find
 * it, so never on characters that replaced its own.
 */
import type { ContainerID, LoroDoc, LoroMap } from "loro-crdt";
import { createNodeFromLoroObj, loroSyncPluginKey } from "loro-prosemirror";
import type { Node, Schema } from "prosemirror-model";
import { type EditorState, Plugin, PluginKey, type Transaction } from "prosemirror-state";
import { Decoration, DecorationSet, type EditorView } from "prosemirror-view";
import { decodeCursor, encodeCursor } from "./cursors";
import { findProseMirrorContainer } from "./index";
import { cursorToPosition, positionToCursor } from "./sync";

/** Loro containers to the ProseMirror nodes made from them (loro-prosemirror's mapping). */
type Mapping = Map<ContainerID, Node | Node[]>;

/** A range as encoded cursors (see {@link encodeRange}). */
export interface EncodedRange {
  from: string | null;
  to: string | null;
}

/** A range to draw. */
export interface AnchoredRange extends EncodedRange {
  id: string;
  /** CSS class of the decoration, such as "thread-anchor". */
  className?: string;
  /** Extra attributes on the decoration, such as `data-thread-id`. */
  attributes?: Record<string, string>;
}

/** Where a range is now. */
export interface ResolvedRange {
  id: string;
  from: number;
  to: number;
  /** All of the range's text was deleted. `from` and `to` are where it was. */
  gone: boolean;
}

type Span = readonly [number, number];

/** What is left of a range's own text after the edits since it was last found. */
interface Followed {
  spans: Span[];
  /** The last edit it was followed through was made here (typing, undo): this editor stores new cursors. */
  local: boolean;
}

interface RangesState {
  ranges: AnchoredRange[];
  /** Each range followed through this editor's own edits since `followedAt`. */
  followed: Map<string, Followed>;
  /** The import count when they were followed (see `imports`). */
  followedAt: number;
  /** Loro holds every edit this state shows (the binding has written them). */
  synced: boolean;
}

export const proseMirrorRangesKey = new PluginKey<RangesState>("pylonAnchoredRanges");

/** The positions of the first and last text in [from, to), or null when there is no text. */
function textBounds(root: Node, from: number, to: number): Span | null {
  let start = -1;
  let end = -1;
  root.nodesBetween(from, to, (node, pos) => {
    if (!node.isText) return true;
    const a = Math.max(from, pos);
    const b = Math.min(to, pos + node.nodeSize);
    if (a < b) {
      if (start < 0) start = a;
      end = b;
    }
    return false;
  });
  return start < 0 ? null : [start, end];
}

/**
 * The document range [from, to) as encoded cursors. The range is first
 * narrowed to its text, so a selection that starts at the end of one
 * paragraph starts at the next one's text. A range with no text has null
 * cursors.
 */
export function encodeRange(
  state: EditorState,
  doc: LoroDoc,
  containerId: ContainerID,
  from: number,
  to: number,
): EncodedRange {
  const bounds = textBounds(state.doc, Math.min(from, to), Math.max(from, to));
  if (!bounds) return { from: null, to: null };
  const start = positionToCursor(state.doc, bounds[0], doc, containerId, { bind: "right" });
  const end = positionToCursor(state.doc, bounds[1], doc, containerId);
  return { from: start ? encodeCursor(start) : null, to: end ? encodeCursor(end) : null };
}

/** The mapping of an editor bound with ProseMirrorSyncPlugin, or null without one. */
function syncMapping(state: EditorState): Mapping | null {
  const sync = loroSyncPluginKey.getState(state) as { mapping?: Mapping } | undefined;
  return sync?.mapping ?? null;
}

/** Where a stored cursor is now, and whether its character was deleted. */
function locate(
  encoded: string | null | undefined,
  state: EditorState,
  doc: LoroDoc,
  containerId: ContainerID,
): { pos: number; moved: boolean } | null {
  const mapping = syncMapping(state);
  return mapping ? locateIn(encoded, state.doc, mapping, doc, containerId) : null;
}

function locateIn(
  encoded: string | null | undefined,
  root: Node,
  mapping: Mapping,
  doc: LoroDoc,
  containerId: ContainerID,
): { pos: number; moved: boolean } | null {
  if (!encoded) return null;
  const cursor = decodeCursor(encoded);
  if (!cursor) return null;
  const inDoc = (pos: number | null) => pos != null && pos >= 0 && pos <= root.content.size;
  try {
    const exact = cursorToPosition(cursor, doc, containerId, mapping);
    if (inDoc(exact)) return { pos: exact!, moved: false };
    const near = cursorToPosition(cursor, doc, containerId, mapping, { acceptMoved: true });
    return inDoc(near) ? { pos: near!, moved: true } : null;
  } catch {
    return null;
  }
}

/** The range between its cursors when both characters are still there. */
function exactSpan(range: EncodedRange, state: EditorState, doc: LoroDoc, containerId: ContainerID): Span | null {
  const from = locate(range.from, state, doc, containerId);
  const to = locate(range.to, state, doc, containerId);
  return from && to && !from.moved && !to.moved && to.pos > from.pos ? [from.pos, to.pos] : null;
}

/**
 * Where each range is in `state` now, from its cursors alone. When neither
 * end's character was deleted, the range is between them. When one was, it
 * runs from the other end to where the deleted character was. When both
 * were, or a cursor cannot be read, it is `gone`.
 *
 * The plugin does better for the editor that made an edit: it follows the
 * range through the transaction (see the module comment).
 */
export function resolveRanges(
  state: EditorState,
  doc: LoroDoc,
  containerId: ContainerID,
  ranges: Array<Pick<AnchoredRange, "id" | "from" | "to">>,
): ResolvedRange[] {
  const mapping = syncMapping(state);
  return resolveWith(state.doc, mapping, doc, containerId, ranges);
}

/**
 * {@link resolveRanges} for code with no editor, such as a server function:
 * the positions are in `field`'s document as the doc holds it now, the
 * document `readProseMirror` returns and `editProseMirror` hands its
 * `change`. Every range is `gone` when the doc has no document in `field`.
 */
export function resolveRangesInDoc(
  doc: LoroDoc,
  field: string,
  schema: Schema,
  ranges: Array<Pick<AnchoredRange, "id" | "from" | "to">>,
): ResolvedRange[] {
  const containerId = findProseMirrorContainer(doc, field);
  const map = containerId ? (doc.getContainerById(containerId) as LoroMap | undefined) : undefined;
  if (!containerId || !map || map.get("nodeName") == null) {
    return ranges.map((r) => ({ id: r.id, from: 0, to: 0, gone: true }));
  }
  const mapping: Mapping = new Map();
  const root = createNodeFromLoroObj(schema, map as never, mapping as never) as Node;
  return resolveWith(root, mapping, doc, containerId, ranges);
}

function resolveWith(
  root: Node,
  mapping: Mapping | null,
  doc: LoroDoc,
  containerId: ContainerID,
  ranges: Array<Pick<AnchoredRange, "id" | "from" | "to">>,
): ResolvedRange[] {
  return ranges.map((r) => {
    const from = mapping ? locateIn(r.from, root, mapping, doc, containerId) : null;
    const to = mapping ? locateIn(r.to, root, mapping, doc, containerId) : null;
    const at = from?.pos ?? to?.pos ?? 0;
    if (!from || !to || (from.moved && to.moved) || to.pos <= from.pos) return { id: r.id, from: at, to: at, gone: true };
    return { id: r.id, from: from.pos, to: to.pos, gone: false };
  });
}

/** A paragraph break in flattened text. It has no position of its own. */
const BREAK = "\u2029";

/** The text in [from, to) of `root`, one entry per UTF-16 unit, with a break before each textblock that starts inside. */
function flattenRegion(root: Node, from: number, to: number): { text: string[]; pos: number[] } {
  const text: string[] = [];
  const pos: number[] = [];
  root.nodesBetween(from, to, (node, at) => {
    if (node.isTextblock && at >= from) {
      text.push(BREAK);
      pos.push(-1);
    }
    if (!node.isText || !node.text) return true;
    for (let i = Math.max(0, from - at); i < node.text.length && at + i < to; i++) {
      text.push(node.text[i]!);
      pos.push(at + i);
    }
    return false;
  });
  return { text, pos };
}

/** Edit distance past which a replace is not diffed: its text counts as replaced. */
const DIFF_LIMIT = 400;

/**
 * The pairs of equal entries in a shortest edit script from `a` to `b`
 * (Myers' diff), or null when finding it needs more than DIFF_LIMIT edits
 * between the common prefix and suffix.
 */
export function diffMatches(a: readonly string[], b: readonly string[]): Array<[number, number]> | null {
  // The common prefix and suffix are matched first. When one side is all of
  // its middle (a pure insert or delete), there is no script to search: the
  // bound is for the search.
  let prefix = 0;
  while (prefix < a.length && prefix < b.length && a[prefix] === b[prefix]) prefix++;
  let suffix = 0;
  while (suffix < a.length - prefix && suffix < b.length - prefix && a[a.length - 1 - suffix] === b[b.length - 1 - suffix]) suffix++;
  const A = a.slice(prefix, a.length - suffix);
  const B = b.slice(prefix, b.length - suffix);
  const matches: Array<[number, number]> = [];
  for (let i = 0; i < prefix; i++) matches.push([i, i]);
  const n = A.length;
  const m = B.length;
  if (n > 0 && m > 0) {
    const max = Math.min(n + m, DIFF_LIMIT);
    const off = max + 1;
    const v = new Int32Array(2 * max + 3);
    const trace: Int32Array[] = [];
    let found = false;
    for (let d = 0; d <= max && !found; d++) {
      trace.push(v.slice());
      for (let k = -d; k <= d; k += 2) {
        let x = k === -d || (k !== d && v[off + k - 1]! < v[off + k + 1]!) ? v[off + k + 1]! : v[off + k - 1]! + 1;
        let y = x - k;
        while (x < n && y < m && A[x] === B[y]) {
          x++;
          y++;
        }
        v[off + k] = x;
        if (x >= n && y >= m) {
          found = true;
          break;
        }
      }
    }
    if (!found) return null;
    const middle: Array<[number, number]> = [];
    let x = n;
    let y = m;
    for (let d = trace.length - 1; d >= 0; d--) {
      const t = trace[d]!;
      const k = x - y;
      const prevK = k === -d || (k !== d && t[off + k - 1]! < t[off + k + 1]!) ? k + 1 : k - 1;
      const prevX = d === 0 ? 0 : t[off + prevK]!;
      const prevY = d === 0 ? 0 : prevX - prevK;
      while (x > prevX && y > prevY) {
        middle.push([prefix + x - 1, prefix + y - 1]);
        x--;
        y--;
      }
      x = prevX;
      y = prevY;
    }
    for (let i = middle.length - 1; i >= 0; i--) matches.push(middle[i]!);
  }
  for (let i = suffix; i > 0; i--) matches.push([a.length - i, b.length - i]);
  return matches;
}

/** Runs of consecutive positions, as spans. */
function runs(positions: number[]): Span[] {
  const sorted = [...positions].sort((x, y) => x - y);
  const out: Span[] = [];
  for (const p of sorted) {
    const last = out[out.length - 1];
    if (last && last[1] === p) out[out.length - 1] = [last[0], p + 1];
    else out.push([p, p + 1]);
  }
  return out;
}

/**
 * The spans left of `spans` after the edits in `tr`: deleted text is cut
 * out, and each end maps toward the inside, so text typed at an edge stays
 * out.
 *
 * With `diff`, text a step replaces is matched against the text it was
 * replaced with, character by character, and kept where it is unchanged.
 * Use it for an undo: it writes earlier text back as new characters and
 * reaches the editor as one replace, which can span text that is still in
 * place. Not for someone else's edit: there Loro knows which characters were
 * replaced, and a diff would keep a range on new text that looks the same.
 */
export function mapSpans(spans: readonly Span[], tr: Transaction, options: { diff?: boolean } = {}): Span[] {
  let out = spans.map((s) => [s[0], s[1]] as Span);
  tr.steps.forEach((step, i) => {
    const map = step.getMap();
    const before = tr.docs[i]!;
    const after = tr.docs[i + 1] ?? tr.doc;
    const replaced: Array<{ a: number; b: number; na: number; nb: number }> = [];
    map.forEach((a, b, na, nb) => {
      if (b > a) replaced.push({ a, b, na, nb });
    });
    const kept: Span[] = [];
    for (const span of out) {
      let pieces: Span[] = [span];
      for (const r of replaced) {
        pieces = pieces.flatMap(([x, y]) => {
          if (r.b <= x || r.a >= y) return [[x, y] as Span];
          const left: Span[] = r.a > x ? [[x, r.a]] : [];
          const right: Span[] = r.b < y ? [[r.b, y]] : [];
          if (options.diff) {
            const from = Math.max(x, r.a);
            const to = Math.min(y, r.b);
            const old = flattenRegion(before, r.a, r.b);
            const now = flattenRegion(after, r.na, r.nb);
            const matches = diffMatches(old.text, now.text);
            if (matches) {
              const moved = matches
                .filter(([oi, ni]) => old.pos[oi]! >= from && old.pos[oi]! < to && now.pos[ni]! >= 0)
                .map(([, ni]) => now.pos[ni]!);
              // Already in the new document's positions: kept apart from the mapping below.
              for (const run of runs(moved)) kept.push(run);
            }
          }
          return [...left, ...right];
        });
      }
      for (const [x, y] of pieces) {
        const nx = map.map(x, 1);
        const ny = map.map(y, -1);
        if (ny > nx) kept.push([nx, ny]);
      }
    }
    out = kept.sort((p, q) => p[0] - q[0]);
  });
  return out;
}

/** Draw these ranges. Replaces the previous list. */
export function setAnchoredRanges(view: EditorView, ranges: AnchoredRange[]): void {
  view.dispatch(view.state.tr.setMeta(proseMirrorRangesKey, ranges).setMeta("addToHistory", false));
}

export interface ProseMirrorRangesOptions {
  doc: LoroDoc;
  containerId: ContainerID;
  /**
   * This editor moved a range's characters (a split, a join, an undo) and
   * has new cursors for it. Store them in place of the range's old ones.
   * Null cursors mean this editor deleted the range's text while its old
   * cursors still found text that remains.
   */
  onReanchor?: (id: string, range: EncodedRange) => void;
}

type Source = "cursors" | "followed" | "near";

interface Resolution {
  spans: Map<string, { span: Span; source: Source }>;
  /** Ranges this editor followed through its own edit: their cursors need replacing. */
  reanchor: Map<string, Span>;
  /** Ranges whose text this editor deleted while their cursors still find text: clear their cursors. */
  cleared: Set<string>;
}

const REFRESH = "pylonAnchoredRangesRefresh";
/** An undo that left the editor's document as it was: the ranges stay where they were drawn. */
const UNDO_IN_PLACE = "pylonAnchoredRangesUndoInPlace";
const key = (r: EncodedRange) => `${r.from ?? ""} ${r.to ?? ""}`;

export function ProseMirrorRangesPlugin({ doc, containerId, onReanchor }: ProseMirrorRangesOptions): Plugin {
  // A Loro change can leave the editor's document as it was (the same text
  // deleted and typed again, as new characters) and still move the ranges,
  // and new cursors change where a range resolves, so the drawn set is kept
  // for one state and version.
  let version = 0;
  // Changes to the field from outside the editor (imports, app code),
  // counted. Spans followed before the latest one may name characters it
  // replaced, so they no longer count.
  let imports = 0;
  // The last change to the field that the binding may bring into the editor:
  // "undo" for this editor's undo and redo (Loro commits them with origin
  // "undo"), "other" for anything else (an import, or app code writing the
  // field directly). The editor's own edits are written by the binding,
  // which does not bring them back.
  let lastChange: "undo" | "other" = "other";
  let cache: { state: EditorState; version: number; imports: number; resolution: Resolution; set: DecorationSet } | null = null;
  const current = (value: RangesState) => (value.followedAt === imports ? value.followed : null);

  // Cursors this editor made, by range. They stand in for the range's
  // cursors while the app still sets the ones they replaced (or any earlier
  // ones this editor made: saves can come back out of order). Cursors the app
  // got from elsewhere end the stand-in.
  const replaced = new Map<string, { known: Set<string>; now: EncodedRange }>();
  const cursorsOf = (range: AnchoredRange): EncodedRange => {
    const r = replaced.get(range.id);
    if (!r) return range;
    if (r.known.has(key(range))) return r.now;
    replaced.delete(range.id);
    return range;
  };

  function resolve(state: EditorState): Resolution {
    const pluginState = proseMirrorRangesKey.getState(state)!;
    const { ranges } = pluginState;
    const followed = current(pluginState);
    const out: Resolution = { spans: new Map(), reanchor: new Map(), cleared: new Set() };
    const near = resolveRanges(state, doc, containerId, ranges.map((r) => ({ id: r.id, ...cursorsOf(r) })));
    ranges.forEach((range, i) => {
      const exact = exactSpan(cursorsOf(range), state, doc, containerId);
      const own = followed?.get(range.id);
      // After this editor's own edit, its transaction says where the range
      // went. The binding writes the smallest text change, which can keep
      // other characters than the ones the person edited (deleting the first
      // of two equal words keeps the first word's characters).
      if (own) {
        // Narrowed to its text, as encodeRange stores it: a span can keep the
        // edge of a paragraph whose text was deleted.
        const span =
          own.spans.length === 0 ? null : textBounds(state.doc, own.spans[0]![0], own.spans[own.spans.length - 1]![1]);
        // All of its text was deleted here: gone, wherever its cursors land.
        // When they still find text (both ends, or one), other editors would
        // draw it there: its cursors are cleared.
        if (!span) {
          if (!near[i]!.gone) out.cleared.add(range.id);
          return;
        }
        if (exact && exact[0] === span[0] && exact[1] === span[1]) {
          return void out.spans.set(range.id, { span: exact, source: "cursors" });
        }
        out.spans.set(range.id, { span, source: "followed" });
        out.reanchor.set(range.id, span);
        return;
      }
      if (exact) return void out.spans.set(range.id, { span: exact, source: "cursors" });
      const f = near[i]!;
      if (!f.gone) out.spans.set(range.id, { span: [f.from, f.to], source: "near" });
    });
    return out;
  }

  /**
   * Where each range is in `state`, to follow through the next transaction:
   * from the drawn set when it is for this state (it was resolved while Loro
   * matched it), else from the cursors when Loro holds this state's edits,
   * else from what was followed so far. A range drawn only near its cursors
   * is not followed: it would store a guess.
   */
  function startingSpans(state: EditorState, value: RangesState): Map<string, Span[]> {
    const base = new Map<string, Span[]>();
    // An import since the last draw may have replaced the characters the
    // drawn set and the followed spans point at: start from the cursors.
    const drawn = cache && cache.state === state && cache.imports === imports ? cache.resolution.spans : null;
    const followedNow = current(value);
    for (const range of value.ranges) {
      const d = drawn?.get(range.id);
      if (drawn) {
        if (d && d.source !== "near") base.set(range.id, d.source === "followed" ? followedNow!.get(range.id)!.spans : [d.span]);
        else if (!d && followedNow?.has(range.id)) base.set(range.id, []);
        continue;
      }
      // What this editor followed comes first: between transactions of one
      // dispatch, the cursors can disagree with it (see resolve).
      const followed = followedNow?.get(range.id)?.spans;
      if (followed) {
        base.set(range.id, followed);
        continue;
      }
      const exact = value.synced ? exactSpan(cursorsOf(range), state, doc, containerId) : null;
      if (exact) base.set(range.id, [exact]);
    }
    return base;
  }

  const draw = (state: EditorState) => {
    if (cache && cache.state === state && cache.version === version) return cache;
    const { ranges } = proseMirrorRangesKey.getState(state)!;
    const byId = new Map(ranges.map((r) => [r.id, r]));
    const resolution = resolve(state);
    const decos = [...resolution.spans].map(([id, { span }]) => {
      const range = byId.get(id)!;
      return Decoration.inline(
        span[0],
        span[1],
        { class: range.className ?? "pylon-anchored-range", ...(range.attributes ?? {}) },
        { rangeId: id },
      );
    });
    cache = { state, version, imports, resolution, set: DecorationSet.create(state.doc, decos) };
    return cache;
  };

  return new Plugin<RangesState>({
    key: proseMirrorRangesKey,
    state: {
      init: () => ({ ranges: [], followed: new Map(), followedAt: 0, synced: true }),
      apply: (tr: Transaction, value, oldState) => {
        const ranges = tr.getMeta(proseMirrorRangesKey) as AnchoredRange[] | undefined;
        if (Array.isArray(ranges)) {
          // Keep following a range while its cursors are the ones it had, or
          // ones this editor made. Other cursors are a new anchor.
          const before = new Map(value.ranges.map((r) => [r.id, key(r)]));
          const followed = new Map<string, Followed>();
          const was = current(value);
          for (const range of ranges) {
            const own = was?.get(range.id);
            const k = key(range);
            if (own && (before.get(range.id) === k || replaced.get(range.id)?.known.has(k))) followed.set(range.id, own);
          }
          return { ranges, followed, followedAt: imports, synced: value.synced };
        }
        // A Loro import that left the editor's document as it was: the
        // cursors decide.
        if (tr.getMeta(REFRESH)) return { ...value, followed: new Map() };
        const inPlace = tr.getMeta(UNDO_IN_PLACE) as Map<string, Span> | undefined;
        if (inPlace) {
          const followed = new Map<string, Followed>();
          for (const [id, span] of inPlace) followed.set(id, { spans: [span], local: true });
          return { ...value, followed, followedAt: imports, synced: true };
        }
        const sync = (tr.getMeta(loroSyncPluginKey) as { type?: string } | undefined)?.type;
        if (tr.docChanged) {
          const fromLoro = sync === "non-local-updates";
          const undoing = fromLoro && lastChange === "undo";
          // Someone else's edit: their editor follows the ranges and stores
          // new cursors. Here the cursors decide; a range whose characters
          // they moved is hidden until the new cursors arrive.
          if (fromLoro && undoing !== true) return { ...value, followed: new Map(), synced: true };
          // This editor's own edit, or its undo (which reaches the editor from
          // Loro, already applied).
          const followed = new Map<string, Followed>();
          for (const [id, spans] of startingSpans(oldState, value)) {
            // An undo writes earlier text back as new characters, and reaches the
            // editor as one replace: match the restored text to what it replaces.
            followed.set(id, { spans: mapSpans(spans, tr, { diff: fromLoro }), local: true });
          }
          return { ...value, followed, followedAt: imports, synced: fromLoro };
        }
        // The binding wrote this editor's edits to Loro.
        if (sync === "doc-changed") return { ...value, synced: true };
        return value;
      },
    },
    view(view) {
      // Registered before the binding's own listener on the field (which it
      // adds after a timeout), so it runs first for each change: the
      // transaction the binding then builds knows who made the change.
      const field = doc.getContainerById(containerId) as
        | { subscribe(f: (e: { by: string; origin?: string }) => void): () => void }
        | undefined;
      // The editor's state when an import into the field arrived, before the
      // binding brought it in. The doc listener, which Loro calls after the
      // field's listeners (a test holds this), compares: an unchanged state
      // means the import changed text without changing the editor's document
      // (the same text deleted and typed again), so the ranges are redrawn.
      let beforeImport: EditorState | null = null;
      // The same for an undo, with where the ranges were drawn before it.
      let beforeUndo: { state: EditorState; spans: Map<string, Span> } | null = null;
      let needsRefresh = false;
      let scheduled = false;
      const unsubscribeField = field?.subscribe((event) => {
        const local = event.by === "local";
        // The binding writes this editor's edits with origin "loroSyncPlugin"
        // and its setup with "sys:init" (loro-prosemirror, sync.ts).
        if (local && (event.origin === "loroSyncPlugin" || event.origin === "sys:init")) return;
        lastChange = local && event.origin === "undo" ? "undo" : "other";
        // The drawn set counts only when no outside change came since it was drawn.
        if (lastChange === "undo" && cache && cache.state === view.state && cache.imports === imports) {
          const spans = new Map<string, Span>();
          for (const [id, d] of cache.resolution.spans) if (d.source !== "near") spans.set(id, d.span);
          beforeUndo = { state: view.state, spans };
        }
        if (lastChange === "other") {
          // From outside the editor: an import, or app code writing the field.
          imports += 1;
          beforeImport = view.state;
        }
      });
      const unsubscribe = doc.subscribe(() => {
        version += 1;
        const undo = beforeUndo;
        beforeUndo = null;
        // An undo that rewrote text as it was: the binding made no transaction.
        // Dispatched now, before anything else can change the field or the
        // selection: the drawn spans are for exactly this state.
        if (undo && view.state === undo.state && undo.spans.size > 0 && !view.isDestroyed) {
          view.dispatch(view.state.tr.setMeta(UNDO_IN_PLACE, undo.spans).setMeta("addToHistory", false));
        }
        if (!beforeImport) return;
        if (view.state === beforeImport) needsRefresh = true;
        beforeImport = null;
        if (!needsRefresh || scheduled) return;
        scheduled = true;
        queueMicrotask(() => {
          scheduled = false;
          const refresh = needsRefresh;
          needsRefresh = false;
          if (!refresh || view.isDestroyed || proseMirrorRangesKey.getState(view.state)!.ranges.length === 0) return;
          view.dispatch(view.state.tr.setMeta(REFRESH, true).setMeta("addToHistory", false));
        });
      });
      const reanchor = () => {
        const { state } = view;
        const pluginState = proseMirrorRangesKey.getState(state)!;
        if (!pluginState.synced) return;
        const { resolution } = draw(state);
        let changed = false;
        const updates: Array<[string, EncodedRange]> = [
          ...[...resolution.reanchor].map(([id, [from, to]]) => [id, encodeRange(state, doc, containerId, from, to)] as [string, EncodedRange]),
          // Null cursors: the range's text was deleted.
          ...[...resolution.cleared].map((id) => [id, { from: null, to: null }] as [string, EncodedRange]),
        ];
        for (const [id, now] of updates) {
          const range = pluginState.ranges.find((r) => r.id === id);
          if (!range) continue;
          if (resolution.reanchor.has(id) && (!now.from || !now.to)) continue;
          // Nothing new to store.
          if (key(now) === key(cursorsOf(range))) continue;
          const entry = replaced.get(id) ?? { known: new Set([key(range)]), now };
          entry.known.add(key(now));
          entry.now = now;
          replaced.set(id, entry);
          changed = true;
          onReanchor?.(id, now);
        }
        // The new cursors change how the ranges resolve.
        if (changed) version += 1;
      };
      return {
        update: reanchor,
        destroy: () => {
          unsubscribe();
          unsubscribeField?.();
        },
      };
    },
    props: {
      decorations(state) {
        if (proseMirrorRangesKey.getState(state)!.ranges.length === 0) return DecorationSet.empty;
        return draw(state).set;
      },
    },
  });
}
