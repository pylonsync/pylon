// Public surface of the sync test harness. Re-export the pieces
// tests typically reach for so a `import { createTestEnv } from
// "../test-harness"` covers 90% of usage.

export { createTestEnv } from "./env";
export type { CreateTestEnvOptions, TestEnv } from "./env";

export { TestServer, defaultVisibilityFilter } from "./server";
export type {
  AuthContext,
  ServerSession,
  TestServerOptions,
  VisibilityFilter,
} from "./server";

export type { TransportHandle } from "./transport";
