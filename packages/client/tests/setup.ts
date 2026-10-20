import { GlobalRegistrator } from "@happy-dom/global-registrator";

// Make document/window available so @testing-library/react can render. Loaded
// via bunfig.toml's [test] preload, before any test file runs. Same setup as
// packages/react.
GlobalRegistrator.register();
