import { entity, field, policy, buildManifest } from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// Crew — a multi-agent orchestration console.
//
// Users create Agents (persona + system prompt + model), optionally chain
// them into Pipelines (ordered steps), and kick off Runs. Each run produces
// live-streaming Messages that sync to every connected client in the tenant
// via the change log — no app-specific socket protocol required.
// ---------------------------------------------------------------------------

const User = entity("User", {
  email: field.string().unique(),
  displayName: field.string(),
  avatarColor: field.string(),
  createdAt: field.datetime(),
});

const Organization = entity("Organization", {
  name: field.string(),
  slug: field.string().unique(),
  createdBy: field.id("User"),
  createdAt: field.datetime(),
});

const OrgMember = entity(
  "OrgMember",
  {
    userId: field.id("User"),
    orgId: field.id("Organization"),
    role: field.string(),
    joinedAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_org_user", fields: ["orgId", "userId"], unique: true },
      { name: "by_user", fields: ["userId"], unique: false },
    ],
  },
);

// An Agent is a reusable LLM persona — system prompt + model + a tool list.
// Tools are referenced by name (a client-side registry); the server treats
// them as opaque strings so new tools can land without schema migrations.
const Agent = entity(
  "Agent",
  {
    orgId: field.id("Organization"),
    name: field.string(),
    role: field.string(), // "Researcher", "Copywriter", "Reviewer", …
    systemPrompt: field.string(),
    model: field.string(), // "claude-haiku-4-5", "claude-sonnet-4-6", …
    avatarEmoji: field.string(), // one-codepoint emoji shown on cards
    tools: field.string().optional(), // JSON array of tool names
    createdBy: field.id("User"),
    createdAt: field.datetime(),
  },
  {
    indexes: [{ name: "by_org", fields: ["orgId"], unique: false }],
  },
);

// Pipelines chain agents. Each step is "run this agent with this instruction
// template" — the previous step's output is available as {{previous}}.
const Pipeline = entity(
  "Pipeline",
  {
    orgId: field.id("Organization"),
    name: field.string(),
    description: field.string().optional(),
    createdBy: field.id("User"),
    createdAt: field.datetime(),
  },
  {
    indexes: [{ name: "by_org", fields: ["orgId"], unique: false }],
  },
);

const PipelineStep = entity(
  "PipelineStep",
  {
    orgId: field.id("Organization"),
    pipelineId: field.id("Pipeline"),
    position: field.float(), // 0-based order within the pipeline
    agentId: field.id("Agent"),
    instruction: field.string(), // prompt template with {{input}} / {{previous}}
  },
  {
    indexes: [
      { name: "by_pipeline_position", fields: ["pipelineId", "position"], unique: true },
    ],
  },
);

// A Run is one execution. It can target a single Agent (ad-hoc) or a
// Pipeline (ordered multi-step). Status drives the UI: queued → running →
// completed/failed/cancelled.
const Run = entity(
  "Run",
  {
    orgId: field.id("Organization"),
    pipelineId: field.id("Pipeline").optional(),
    agentId: field.id("Agent").optional(),
    title: field.string(), // short human label for the runs list
    input: field.string(),
    status: field.string(), // queued | running | completed | failed | cancelled
    startedBy: field.id("User"),
    createdAt: field.datetime(),
    startedAt: field.datetime().optional(),
    completedAt: field.datetime().optional(),
    error: field.string().optional(),
    tokensIn: field.float().optional(),
    tokensOut: field.float().optional(),
  },
  {
    indexes: [{ name: "by_org_created", fields: ["orgId", "createdAt"], unique: false }],
  },
);

// One RunStep per agent invocation. For ad-hoc runs there's exactly one;
// pipeline runs produce N in order. output accumulates as tokens stream.
const RunStep = entity(
  "RunStep",
  {
    orgId: field.id("Organization"),
    runId: field.id("Run"),
    stepNumber: field.float(), // 1-based position within the run
    agentId: field.id("Agent"),
    input: field.string(),
    output: field.string(), // grows incrementally during streaming
    status: field.string(), // pending | running | completed | failed | cancelled
    tokensIn: field.float().optional(),
    tokensOut: field.float().optional(),
    startedAt: field.datetime().optional(),
    completedAt: field.datetime().optional(),
    error: field.string().optional(),
  },
  {
    indexes: [
      { name: "by_run_step", fields: ["runId", "stepNumber"], unique: true },
    ],
  },
);

// Chat-style transcript inside a run step. role = system | user | assistant.
// Tokens arrive as incremental inserts during streaming — the sync engine
// turns every insert into a WebSocket change event, so any client watching
// the run sees the transcript build in real time.
const Message = entity(
  "Message",
  {
    orgId: field.id("Organization"),
    runId: field.id("Run"),
    runStepId: field.id("RunStep"),
    role: field.string(), // system | user | assistant | tool
    content: field.string(),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_run_created", fields: ["runId", "createdAt"], unique: false },
      { name: "by_step", fields: ["runStepId", "createdAt"], unique: false },
    ],
  },
);

// ---------------------------------------------------------------------------
// Policies — tenant isolation
// ---------------------------------------------------------------------------

const orgScoped = (name: string) =>
  policy({
    name: `${name}_org_scoped`,
    entity: name,
    allowRead: "auth.tenantId == data.orgId",
    allowInsert: "auth.tenantId == data.orgId",
    allowUpdate: "auth.tenantId == data.orgId",
    allowDelete: "auth.tenantId == data.orgId",
  });

const organizationPolicy = policy({
  name: "organization_access",
  entity: "Organization",
  allowRead: "auth.userId != null",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId == data.createdBy",
  allowDelete: "auth.userId == data.createdBy",
});

const orgMemberPolicy = policy({
  name: "org_member_access",
  entity: "OrgMember",
  allowRead: "auth.userId == data.userId || auth.tenantId == data.orgId",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.tenantId == data.orgId",
  allowDelete: "auth.tenantId == data.orgId",
});

const manifest = buildManifest({
  name: "crew",
  version: "0.1.0",
  entities: [
    User,
    Organization,
    OrgMember,
    Agent,
    Pipeline,
    PipelineStep,
    Run,
    RunStep,
    Message,
  ],
  queries: [],
  actions: [],
  policies: [
    organizationPolicy,
    orgMemberPolicy,
    orgScoped("Agent"),
    orgScoped("Pipeline"),
    orgScoped("PipelineStep"),
    orgScoped("Run"),
    orgScoped("RunStep"),
    orgScoped("Message"),
  ],
  routes: [],
});

console.log(JSON.stringify(manifest, null, 2));
