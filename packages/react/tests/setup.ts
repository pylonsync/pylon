import { GlobalRegistrator } from "@happy-dom/global-registrator";

// Make document/window available so @testing-library/react can render. Loaded
// via bunfig.toml's [test] preload, before any test file runs.
//
// Until this existed, packages/react shipped no way to MOUNT a hook — the few
// tests here drove module-level registries directly. That is why a
// re-render bug in useQuery (a ref flipped inside getSnapshot, which
// useSyncExternalStore then bailed out of rendering) reached production: every
// piece of it was individually correct and only the mounted behaviour was
// wrong.
GlobalRegistrator.register();
