// The module registry of a production server bundle (`pylon build`).
//
// In source mode the runner imports app code by path at run time: function
// files from `functions/`, workflows from `workflows/`, and page / layout /
// boundary modules from `app/`. It also resolves `react` from the app's
// `node_modules`. A production bundle has none of those files. Its generated
// entry puts a loader for every module in this registry and then starts the
// runtime. Every loader in this package asks the registry first.
//
// The registry holds loaders, not modules, so each module is evaluated when
// it is first needed, as in source mode: the runtime loads each function
// with its own try/catch at boot, and a page loads on its first render. A
// module that throws at the top level fails alone.
//
// The entry sets the registry before it imports the runtime, whose top-level
// `main()` reads it.

/** Loads one bundled module and returns its namespace. */
export type ModuleLoader = () => Promise<any>;

export interface PylonServerBundle {
  /** Function name (file name without extension) → module loader. */
  functions: Record<string, ModuleLoader>;
  /** Workflow file name (without extension) → module loader. */
  workflows: Record<string, ModuleLoader>;
  /** Project-relative, extension-less, "/"-separated module path
   *  (`app/blog/page`) → module loader. Holds every module under the app
   *  dir that the SSR runtime can import: pages, layouts, boundaries,
   *  loading states, route handlers, OG image modules, metadata routes. */
  modules: Record<string, ModuleLoader>;
  /** The app's React and React DOM server, as bundled with the pages, so SSR
   *  renders with the same React instance the page modules import. */
  react: any;
  reactDomServer: any;
  /** Client bundle directory, relative to the artifact root. */
  clientDir: string;
  /** Absolute paths of the files the OG image renderer reads at run time.
   *  In source mode it finds them next to its own source file and in
   *  `node_modules`; the bundle copies them next to the server output. */
  ogAssets: {
    resvgWasm: string;
    interRegular: string;
    interSemiBold: string;
  };
}

const KEY = "__PYLON_SERVER_BUNDLE__";

export function serverBundle(): PylonServerBundle | null {
  return ((globalThis as any)[KEY] as PylonServerBundle | undefined) ?? null;
}

export function setServerBundle(bundle: PylonServerBundle): void {
  (globalThis as any)[KEY] = bundle;
}

/** Normalize a module path to the registry key form. */
export function moduleKey(relPath: string): string {
  return relPath
    .replace(/\\/g, "/")
    .replace(/^\.\//, "")
    .replace(/\.(tsx?|jsx?)$/, "");
}
