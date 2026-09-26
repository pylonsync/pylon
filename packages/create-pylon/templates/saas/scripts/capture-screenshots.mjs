#!/usr/bin/env node
// Regenerates the product screenshots the landing page shows
// (public/screenshots/*.png) from this app's own dashboard.
//
//   1. Start the app:            pylon dev
//   2. Install Playwright once:  npm i -D playwright && npx playwright install chromium
//   3. Capture:                  node scripts/capture-screenshots.mjs
//
// The script signs up a workspace owner and three teammates through the auth
// API, invites the teammates, adds the sample projects (seedWorkspace), then
// opens the dashboard in Chromium and saves each screen. Run it against a
// fresh development database: it reuses the accounts on a second run, but the
// data then reflects whatever you changed in between.
//
// Set BASE_URL to capture from another port.

import { mkdir } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const BASE = (process.env.BASE_URL ?? "http://localhost:4321").replace(/\/$/, "");
const OUT = join(dirname(fileURLToPath(import.meta.url)), "..", "public", "screenshots");
const PASSWORD = "sample-password-2941";
const OWNER = { email: "dana@northwind.test", name: "Dana Reyes" };
const TEAM = [
  { email: "sam@northwind.test", name: "Sam Okafor" },
  { email: "priya@northwind.test", name: "Priya Raman" },
  { email: "leo@northwind.test", name: "Leo Park" },
];
const PENDING = ["maria@northwind.test", "jonas@northwind.test"];

let chromium;
try {
  ({ chromium } = await import("playwright"));
} catch {
  try {
    ({ chromium } = await import("playwright-core"));
  } catch {
    console.error("Playwright is not installed. Run: npm i -D playwright && npx playwright install chromium");
    process.exit(1);
  }
}

async function api(path, { token, body, method = "POST" } = {}) {
  const res = await fetch(BASE + path, {
    method,
    headers: {
      "content-type": "application/json",
      ...(token ? { authorization: `Bearer ${token}` } : {}),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    const err = new Error(`${method} ${path} → ${res.status} ${data?.error?.code ?? ""} ${data?.error?.message ?? ""}`);
    err.code = data?.error?.code;
    throw err;
  }
  return data;
}

/** Register the account, or sign in when it already exists. Returns a token. */
async function account({ email, name }) {
  try {
    const s = await api("/api/auth/password/register", { body: { email, password: PASSWORD, displayName: name } });
    return s.token;
  } catch (err) {
    if (err.code !== "EMAIL_TAKEN" && err.code !== "USER_EXISTS") throw err;
    const s = await api("/api/auth/password/login", { body: { email, password: PASSWORD } });
    return s.token;
  }
}

async function setUp() {
  const owner = await account(OWNER);
  const orgs = await api("/api/auth/orgs", { token: owner, method: "GET" });
  const list = Array.isArray(orgs) ? orgs : orgs.orgs ?? [];
  let org = list.find((o) => o.name === "Northwind");
  if (!org) org = await api("/api/auth/orgs", { token: owner, body: { name: "Northwind" } });
  await api("/api/auth/select-org", { token: owner, body: { orgId: org.id } });

  const members = await api(`/api/auth/orgs/${org.id}/members`, { token: owner, method: "GET" });
  const roster = Array.isArray(members) ? members : members.members ?? [];
  for (const person of TEAM) {
    const token = await account(person);
    if (roster.some((m) => m.email === person.email)) continue;
    const invite = await api(`/api/auth/orgs/${org.id}/invites`, {
      token: owner,
      body: { email: person.email, role: "member" },
    });
    await api(`/api/auth/invites/${encodeURIComponent(invite.token)}/accept`, { token, body: {} });
  }
  for (const email of PENDING) {
    await api(`/api/auth/orgs/${org.id}/invites`, { token: owner, body: { email, role: "member" } }).catch(() => {});
  }

  await api("/api/fn/seedWorkspace", { token: owner, body: { orgId: org.id } });
  await api("/api/fn/completeOnboarding", { token: owner, body: { orgId: org.id } });
  await api("/api/fn/dismissSetup", { token: owner, body: { orgId: org.id } });
  return org;
}

// Hide the development-only overlay badge that `pylon dev` adds.
async function hideDevBadge(page) {
  await page.evaluate(() => {
    for (const el of document.querySelectorAll("body *")) {
      if (el.children.length === 0 && el.textContent?.trim() === "pylon") {
        let node = el;
        while (node && getComputedStyle(node).position !== "fixed") node = node.parentElement;
        if (node) node.style.display = "none";
      }
    }
  });
}

async function shoot(page, path, file) {
  await page.goto(BASE + path, { waitUntil: "load" });
  await page.waitForTimeout(1500);
  await hideDevBadge(page);
  await page.screenshot({ path: join(OUT, file) });
  console.log(`saved public/screenshots/${file}`);
}

const org = await setUp();
await mkdir(OUT, { recursive: true });

const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2 });
const page = await context.newPage();
// Signing in from the page stores the session cookie in this browser
// context. The active workspace belongs to the session, so select it here too.
await page.goto(BASE + "/login");
await page.evaluate(
  async ({ email, password, orgId }) => {
    const post = (path, body) =>
      fetch(path, {
        method: "POST",
        headers: { "content-type": "application/json" },
        credentials: "include",
        body: JSON.stringify(body),
      });
    await post("/api/auth/password/login", { email, password });
    await post("/api/auth/select-org", { orgId });
  },
  { email: OWNER.email, password: PASSWORD, orgId: org.id },
);

await shoot(page, "/dashboard", "overview.png");
await shoot(page, "/dashboard/projects", "projects.png");
await shoot(page, "/dashboard/members", "members.png");
await page.goto(BASE + "/dashboard/projects", { waitUntil: "load" });
await page.getByRole("link", { name: /Website relaunch/ }).first().click();
await page.waitForURL(/\/dashboard\/projects\/.+/);
await shoot(page, new URL(page.url()).pathname, "board.png");

await browser.close();
