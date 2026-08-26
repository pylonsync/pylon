import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
  discoverFunctions,
  font,
} from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// projects — client project delivery for a small team: projects, a task board,
// and time logged against the work.
//
// TIME IS A LEDGER. A task\'s logged hours are the sum of its TimeEntry rows,
// never a running total on the task. Two people logging against the same task at
// once is normal, and a mutable total loses one of them. It also means every
// hour on an invoice traces to who logged it and when — the question a client
// asks.
//
//   • Client    — who the work is for.
//   • Project   — budget, rate, due date, status.
//   • Task      — the board. Status, assignee, estimate.
//   • TimeEntry — minutes against a task. Append-only.
//   • User      — a team member (email/password).
//
// Minutes are integers. Hours as floats produce 7.999999 on a timesheet.
//
// The realtime hook: drag a task and it moves on every teammate\'s board
// instantly, so a standup doesn\'t start with reconciling two views of the work.
//
// TRUST MODEL: an internal tool for ONE team. Every entity is readable and
// writable by any SIGNED-IN user and by nobody else. There is no client portal —
// adding one means a separate, narrower policy for that client\'s own projects.
// ---------------------------------------------------------------------------

const TEAM = "auth.userId != null";

const Client = entity(
  "Client",
  {
    name: field.string(),
    email: field.string().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_name", fields: ["name"], unique: false }] },
);

const Project = entity(
  "Project",
  {
    name: field.string(),
    clientId: field.id("Client").optional(),
    // "active" | "paused" | "complete" — see PROJECT_STATUSES.
    status: field.string().default("active"),
    // Agreed budget in whole MINUTES; 0 means no budget to track against.
    budgetMinutes: field.number().default(0),
    hourlyRateCents: field.number().default(0),
    dueDate: field.datetime().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_status", fields: ["status"], unique: false },
      { name: "by_client", fields: ["clientId"], unique: false },
    ],
  },
);

const Task = entity(
  "Task",
  {
    projectId: field.id("Project"),
    title: field.string(),
    // "todo" | "doing" | "review" | "done" — see TASK_STATUSES.
    status: field.string().default("todo"),
    assigneeId: field.id("User").optional(),
    estimateMinutes: field.number().default(0),
    // Manual order within a column, so dragging is stable.
    position: field.number().default(0),
    createdAt: field.datetime().defaultNow(),
    updatedAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_project", fields: ["projectId"], unique: false },
      { name: "by_status", fields: ["status"], unique: false },
    ],
  },
);

// APPEND-ONLY, like a stock ledger. `projectId` is denormalised so project
// totals don\'t need a join through Task on every read — the task can\'t change
// project, so it can\'t go stale.
const TimeEntry = entity(
  "TimeEntry",
  {
    taskId: field.id("Task"),
    projectId: field.id("Project"),
    minutes: field.number(),
    note: field.string().optional(),
    userId: field.id("User").readonly().optional(),
    spentOn: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_task", fields: ["taskId"], unique: false },
      { name: "by_project", fields: ["projectId"], unique: false },
    ],
  },
);

const User = entity(
  "User",
  {
    email: field.string(),
    displayName: field.string().optional(),
    passwordHash: field.string().serverOnly().optional(),
    avatarColor: field.string().optional(),
    emailVerified: field.datetime().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_email", fields: ["email"], unique: true }] },
);

const teamPolicy = (name: string, entityName: string) =>
  policy({
    name,
    entity: entityName,
    allowRead: TEAM,
    allowInsert: TEAM,
    allowUpdate: TEAM,
    allowDelete: TEAM,
  });

// Logged time is insert-only: editing an entry would quietly change what a
// client was billed, and deleting one would make a total unexplainable. A
// correction is a negative entry, which stays visible in the history.
const timePolicy = policy({
  name: "time_append_only",
  entity: "TimeEntry",
  allowRead: TEAM,
  allowInsert: TEAM,
  allowUpdate: "false",
  allowDelete: "false",
});

const userPolicy = policy({
  name: "user_team_read",
  entity: "User",
  allowRead: TEAM,
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// Every non-internal function in functions/ becomes a manifest entry —
// this is what makes them show up in /api/manifest, the OpenAPI spec,
// and `pylon codegen`.
const fns = await discoverFunctions();

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Client, Project, Task, TimeEntry, User],
  queries: fns.queries,
  actions: fns.actions,
  policies: [
    teamPolicy("client_team", "Client"),
    teamPolicy("project_team", "Project"),
    teamPolicy("task_team", "Task"),
    timePolicy,
    userPolicy,
  ],
  auth: auth(),
  fonts: [
    font({
      family: "Inter",
      variable: "--font-sans",
      weights: ["400", "500", "600", "700"],
      subsets: ["latin"],
      display: "swap",
      preload: true,
    }),
  ],
  routes: await discoverAppRoutes(),
});

// Not a debug leftover: the CLI runs `bun run app.ts` and parses stdout as
// your manifest (see crates/cli/src/bun.rs). Deleting this line breaks every
// pylon command that reads your schema.
console.log(JSON.stringify(manifest, null, 2));

export default manifest;
