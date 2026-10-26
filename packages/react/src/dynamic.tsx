// Component-level code splitting — `next/dynamic` parity for Pylon.
//
// Route-level splitting gives every page its own entry chunk, which solves
// having many routes. It does nothing for ONE route with a heavy dependency: a
// page that statically imports a rich-text editor puts the whole editor in its
// entry, and since <Link> warms sibling routes on sight, that weight lands on
// the first load of every page in the section — for a component most visitors
// never open.
//
// `dynamic()` moves it behind a real `import()`, which the bundler emits as
// its own chunk and (deliberately) leaves out of the route's modulepreload
// set, so the bytes ship only when something actually renders it.
//
//   const Editor = dynamic(() => import("./Editor"), {
//     loading: () => <EditorSkeleton />,
//   });
//
// NOTE the name collision, which is easy to misread: the route-segment export
// `export const dynamic = "force-static" | "force-dynamic"` is a CACHE
// directive and unrelated to this. Same word, different thing, both from Next.

import React, {
  Suspense,
  lazy,
  useEffect,
  useState,
  type ComponentType,
} from "react";
import { appUpdate } from "./appUpdate";

export interface DynamicOptions {
  /** Rendered while the chunk loads (and, when `ssr` is false, on the server). */
  loading?: ComponentType<any> | null;
  /**
   * Render on the server as well as the client. Default **false**.
   *
   * This is the opposite of Next's default, and deliberately so. Under Pylon a
   * page hydrates ONCE, after the whole document has streamed, so the safe
   * contract is that the server HTML and the first client render are
   * identical — which `ssr: false` guarantees by rendering the fallback in
   * both. It is also the only mode that removes bytes from the first load: an
   * `ssr: true` component has to be in the client bundle before hydration, so
   * it saves nothing, and it's there for organizing code, not for weight.
   */
  ssr?: boolean;
}

type Loader<P> = () => Promise<{ default: ComponentType<P> } | ComponentType<P>>;

/** A module namespace or the component itself — accept both, like Next. */
/**
 * A chunk failed to load. After a deploy the server no longer has this
 * build's chunks: move the page to the new build (an app with `offline`
 * only; see `moveToLatestBuild`). Resolves true when the page is about to
 * load the new build. Offline, or with the server on this build, the page
 * stays as it is.
 */
async function movedAfterChunkError(): Promise<boolean> {
  try {
    return (await appUpdate.moveToLatestBuild({ recheck: true })) === "moving";
  } catch {
    return false;
  }
}

function resolveComponent<P>(
  mod: { default: ComponentType<P> } | ComponentType<P>,
): ComponentType<P> {
  return (mod as { default: ComponentType<P> })?.default ?? (mod as ComponentType<P>);
}

/**
 * Defer a component into its own chunk.
 *
 * The returned component renders `loading` until the chunk arrives, then the
 * real one. Call it at MODULE level, not inside a render — a `dynamic()` call
 * per render creates a new component type each time and remounts the subtree.
 *
 * `ref` reaches the loaded component: React 19 treats it as an ordinary prop,
 * so the spread carries it to a `forwardRef` target's imperative handle. That
 * matters for the exact components people defer — an editor whose save path
 * calls `ref.current.export()` would fail silently otherwise — so it's covered
 * by a test rather than left to React's prop semantics staying put.
 */
export function dynamic<P extends object>(
  loader: Loader<P>,
  options: DynamicOptions = {},
): ComponentType<P & { ref?: React.Ref<any> }> {
  const { loading: Loading = null, ssr = false } = options;

  if (ssr) {
    // Server-rendered: React awaits the lazy component during the SSR render,
    // and Suspense covers the client while the chunk loads. No first-load
    // saving — the component is in the tree the server rendered — so this is
    // for splitting code, not weight.
    const Lazy = lazy(async () => {
      let mod: Awaited<ReturnType<Loader<P>>>;
      try {
        mod = await loader();
      } catch (err) {
        // The page is loading the new build: keep the fallback up until it
        // unloads, rather than handing the error to an error boundary.
        if (await movedAfterChunkError()) return new Promise<never>(() => {});
        throw err;
      }
      return { default: resolveComponent<P>(mod) as ComponentType<any> };
    });
    const WithSuspense = (props: P) => (
      <Suspense fallback={Loading ? <Loading /> : null}>
        <Lazy {...(props as any)} />
      </Suspense>
    );
    WithSuspense.displayName = "DynamicSSR";
    return WithSuspense as ComponentType<P & { ref?: React.Ref<any> }>;
  }

  // Client-only. The fallback renders on the server AND on the first client
  // pass, so the two agree byte for byte and hydration cannot mismatch; the
  // real component swaps in from an effect, which never runs during SSR.
  //
  // The promise is cached on first use so remounts don't refetch, and so two
  // instances of the same dynamic component share one request. A failed load
  // is not kept: the next mount tries again (offline, then back online).
  let cached: Promise<ComponentType<P>> | null = null;
  const load = () =>
    (cached ??= Promise.resolve()
      .then(loader)
      .then(resolveComponent<P>)
      .catch((err) => {
        cached = null;
        throw err;
      }));

  const ClientOnly = (props: P) => {
    const [Loaded, setLoaded] = useState<ComponentType<P> | null>(null);
    useEffect(() => {
      let alive = true;
      load().then(
        (C) => {
          // The functional form — setState treats a bare function as an
          // updater, and a component IS a function.
          if (alive) setLoaded(() => C);
        },
        (err) => {
          // A chunk that fails to load leaves the fallback up rather than
          // blanking the subtree. After a deploy the page moves to the new
          // build; offline it keeps the fallback.
          void movedAfterChunkError().then((moving) => {
            if (!moving) console.error("[pylon] dynamic() failed to load a component:", err);
          });
        },
      );
      return () => {
        alive = false;
      };
    }, []);
    if (!Loaded) return Loading ? <Loading /> : null;
    return <Loaded {...(props as any)} />;
  };
  ClientOnly.displayName = "Dynamic";
  return ClientOnly as ComponentType<P & { ref?: React.Ref<any> }>;
}
