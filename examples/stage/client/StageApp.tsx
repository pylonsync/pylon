/**
 * Stage — a block-based multi-page site builder.
 *
 * Demonstrates realtime collaborative editing: every block edit is a
 * mutation that broadcasts through the change log, so two tabs open on
 * the same page watch each other's edits land live.
 *
 * Also shows publish gating: the public preview at /p/:slug renders
 * only when Site.publishedAt is set.
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  init,
  db,
  callFn,
  configureClient,
  storageKey,
  useSession,
  useRoom,
} from "@pylonsync/react";
import { installDesignTokens, getDesignTokens } from "./designTokens";
import { renderSection, SECTION_TYPES } from "./sections";
import { AlertDialog, Dialog, PromptDialog, ToastProvider, useToast } from "./ui";
import { Icon, IconLibraryProvider, ICON_LIBRARIES, type IconLibrary } from "./icons";
import { SHADCN_PRESETS, parseShadcnPreset, type PresetTokens } from "./presets";

const BASE_URL = "http://localhost:4321";
init({ baseUrl: BASE_URL, appName: "stage" });
configureClient({ baseUrl: BASE_URL, appName: "stage" });

// Inject the DESIGN.md tokens as CSS custom properties — the whole
// editor chrome reads from them via `var(--color-*)`, `var(--rounded-*)`,
// etc. Runs once at module load; idempotent. See client/designTokens.ts.
installDesignTokens();

// ---------------------------------------------------------------------------
// Breakpoints — responsive editor state
// ---------------------------------------------------------------------------

type Breakpoint = "desktop" | "tablet" | "phone";
type ViewMode = Breakpoint | "all";
const BREAKPOINTS: { id: Breakpoint; icon: string; label: string; width: number }[] = [
  { id: "desktop", icon: "Monitor", label: "Desktop", width: 1024 },
  { id: "tablet", icon: "Tablet", label: "Tablet", width: 720 },
  { id: "phone", icon: "Smartphone", label: "Phone", width: 390 },
];

// Merge breakpoint-scoped props. Block props can be flat {key: val} or
// keyed {desktop:{...},tablet:{...},phone:{...}}. Desktop wins first,
// then the current breakpoint overrides. Phone inherits tablet overrides
// when targeting phone — matching how users expect mobile-first design
// to cascade.
function resolveProps(propsJson: string, bp: Breakpoint): Record<string, unknown> {
  const raw = safeJson(propsJson) ?? {};
  const keyed =
    "desktop" in raw || "tablet" in raw || "phone" in raw;
  if (!keyed) return raw as Record<string, unknown>;
  const desktop = (raw as any).desktop ?? {};
  const tablet = (raw as any).tablet ?? {};
  const phone = (raw as any).phone ?? {};
  if (bp === "desktop") return { ...desktop };
  if (bp === "tablet") return { ...desktop, ...tablet };
  return { ...desktop, ...tablet, ...phone };
}

// Write changes into the current breakpoint without losing other bp
// overrides. Migrates flat props → keyed the first time a non-desktop
// breakpoint is edited.
function patchBreakpointProps(
  propsJson: string,
  bp: Breakpoint,
  changes: Record<string, unknown>,
): string {
  const raw = (safeJson(propsJson) as Record<string, unknown>) ?? {};
  const keyed = "desktop" in raw || "tablet" in raw || "phone" in raw;
  if (!keyed) {
    if (bp === "desktop") return JSON.stringify({ ...raw, ...changes });
    return JSON.stringify({ desktop: raw, [bp]: changes });
  }
  const bucket = (raw[bp] as Record<string, unknown>) ?? {};
  return JSON.stringify({ ...raw, [bp]: { ...bucket, ...changes } });
}

function safeJson(s: string): unknown {
  try { return JSON.parse(s); } catch { return null; }
}

// Normalize a page slug for URL use. "/" becomes "home" so the URL
// reads /sites/XYZ/p/home instead of /sites/XYZ/p/ (which trims).
function slugSegment(slug: string): string {
  if (slug === "/" || slug === "") return "home";
  return slug.replace(/^\/+/, "");
}

// ---------------------------------------------------------------------------
// Cursor color palette — deterministic per userId, drawn from DESIGN.md
// ---------------------------------------------------------------------------

function colorForUser(userId: string): string {
  const tokens = getDesignTokens();
  const colors = (tokens.colors as Record<string, string>) ?? {};
  const palette = [
    colors["cursor-1"] ?? "#FF3D7F",
    colors["cursor-2"] ?? "#7C5CFF",
    colors["cursor-3"] ?? "#F2994A",
    colors["cursor-4"] ?? "#00A86B",
    colors["cursor-5"] ?? "#2D9CDB",
    colors["cursor-6"] ?? "#E5484D",
  ];
  let hash = 0;
  for (let i = 0; i < userId.length; i++) hash = (hash * 31 + userId.charCodeAt(i)) | 0;
  return palette[Math.abs(hash) % palette.length];
}

// ---------------------------------------------------------------------------
// Site tokens → CSS variable string. Applied scoped to the canvas so
// user tokens don't pollute the editor chrome.
// ---------------------------------------------------------------------------

type SiteTokens = {
  colors: {
    primary: string; accent: string; body: string;
    muted: string; surface: string; outline: string;
  };
  typography: {
    heading: { fontFamily: string; fontWeight: number };
    body: { fontFamily: string; fontWeight: number };
  };
  rounded: { md: number; lg: number };
  // shadcn/ui-style appearance knobs. Drive the canvas look without
  // a schema migration — all live under Site.tokensJson.
  iconLibrary?: IconLibrary;
  radius?: "none" | "sm" | "md" | "lg" | "xl";
  menuStyle?: "solid" | "outline" | "ghost";
  style?: "nova" | "classic" | "brutalist" | "playful";
};

const DEFAULT_SITE_TOKENS: SiteTokens = {
  colors: {
    primary: "#0B0B0F",
    accent: "#FF3D7F",
    body: "#334155",
    muted: "#64748B",
    surface: "#FFFFFF",
    outline: "#E6E4EB",
  },
  typography: {
    heading: { fontFamily: "Inter", fontWeight: 700 },
    body: { fontFamily: "Inter", fontWeight: 400 },
  },
  rounded: { md: 8, lg: 12 },
  iconLibrary: "lucide",
  radius: "md",
  menuStyle: "solid",
  style: "nova",
};

function resolveSiteTokens(site: Site): SiteTokens {
  if (!site.tokensJson) {
    return {
      ...DEFAULT_SITE_TOKENS,
      colors: { ...DEFAULT_SITE_TOKENS.colors, accent: site.accentColor },
    };
  }
  const parsed = safeJson(site.tokensJson);
  if (!parsed || typeof parsed !== "object") return DEFAULT_SITE_TOKENS;
  const merged = { ...DEFAULT_SITE_TOKENS, ...(parsed as object) } as SiteTokens;
  merged.colors = { ...DEFAULT_SITE_TOKENS.colors, ...(merged as any).colors };
  return merged;
}

function siteTokenStyle(site: Site): React.CSSProperties {
  const t = resolveSiteTokens(site);
  return {
    // Canvas-scoped CSS variables. Block renderers reference these
    // instead of hardcoded colors, so editing the Site panel updates
    // every block live.
    ["--site-accent" as any]: t.colors.accent,
    ["--site-body" as any]: t.colors.body,
    ["--site-muted" as any]: t.colors.muted,
    ["--site-surface" as any]: t.colors.surface,
    ["--site-outline" as any]: t.colors.outline,
    ["--site-primary" as any]: t.colors.primary,
  } as React.CSSProperties;
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type User = { id: string; email: string; displayName: string; avatarColor: string };
type Organization = { id: string; name: string; slug: string; createdBy: string; createdAt: string };
type OrgMember = { id: string; userId: string; orgId: string; role: string; joinedAt: string };

type Site = {
  id: string; orgId: string; name: string; slug: string;
  faviconEmoji: string; accentColor: string; typeface: string;
  tokensJson?: string | null;
  createdBy: string; createdAt: string;
  publishedAt?: string | null;
};

type Page = {
  id: string; orgId: string; siteId: string;
  slug: string; title: string; sort: number;
  metaTitle?: string | null; metaDescription?: string | null;
  createdAt: string;
};

type Block = {
  id: string; orgId: string; siteId: string;
  pageId?: string | null; parentId?: string | null;
  componentId?: string | null;
  sort: number;
  type: string; propsJson: string;
  createdAt: string;
};

type Component = {
  id: string; orgId: string; siteId: string;
  name: string; createdBy: string; createdAt: string;
};

type BlockType = string;

type BlockEntry = { type: BlockType; label: string; icon: string; category: string };
const BLOCK_TYPES: BlockEntry[] = [
  // Basic primitives — icon names are Lucide PascalCase, rendered via <Icon name=".." />
  { category: "Basic", type: "heading", label: "Heading", icon: "Heading2" },
  { category: "Basic", type: "text", label: "Text", icon: "Type" },
  { category: "Basic", type: "button", label: "Button", icon: "MousePointer" },
  { category: "Basic", type: "image", label: "Image", icon: "Image" },
  { category: "Basic", type: "divider", label: "Divider", icon: "Minus" },
  { category: "Basic", type: "container", label: "Container", icon: "Square" },
  // Hero sections
  { category: "Hero", type: "hero-centered", label: "Centered", icon: "Target" },
  { category: "Hero", type: "hero-split", label: "Split + Image", icon: "Columns2" },
  // Social proof + conversion
  { category: "Sections", type: "feature-grid", label: "Features", icon: "LayoutGrid" },
  { category: "Sections", type: "stats", label: "Stats", icon: "Gauge" },
  { category: "Sections", type: "logo-cloud", label: "Logos", icon: "Sparkles" },
  { category: "Sections", type: "testimonial", label: "Testimonial", icon: "Quote" },
  { category: "Sections", type: "pricing", label: "Pricing", icon: "DollarSign" },
  { category: "Sections", type: "faq", label: "FAQ", icon: "HelpCircle" },
  { category: "Sections", type: "cta-banner", label: "CTA Banner", icon: "Flag" },
  { category: "Sections", type: "footer", label: "Footer", icon: "LayoutPanelLeft" },
];
const BLOCK_CATEGORIES = ["Basic", "Hero", "Sections"];

// ---------------------------------------------------------------------------
// Root — decides between public preview, login, and editor
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Minimal router. URL shape:
//   /                              → dashboard (all sites)
//   /sites/:siteId                 → site editor (home page)
//   /sites/:siteId/p/:pageSlug     → site editor on a specific page
//   /p/:siteSlug                   → public preview (unauthenticated)
//   /p/:siteSlug/:nestedSlug       → public preview, nested page
// Uses history.pushState + a popstate listener so back/forward work.
// ---------------------------------------------------------------------------

type Route =
  | { kind: "dashboard" }
  | { kind: "site"; siteId: string; pageSlug?: string }
  | { kind: "preview"; siteSlug: string; pageSlug: string };

function parseRoute(pathname: string): Route {
  const preview = pathname.match(/^\/p\/([a-z0-9-]+)(?:\/(.*))?$/);
  if (preview) return { kind: "preview", siteSlug: preview[1], pageSlug: preview[2] ?? "" };
  const site = pathname.match(/^\/sites\/([^/]+)(?:\/p\/(.+))?$/);
  if (site) return { kind: "site", siteId: site[1], pageSlug: site[2] };
  return { kind: "dashboard" };
}

function useRoute(): [Route, (next: Route) => void] {
  const [route, setRoute] = useState<Route>(() => parseRoute(window.location.pathname));
  useEffect(() => {
    function onPop() {
      setRoute(parseRoute(window.location.pathname));
    }
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);
  const navigate = useCallback((next: Route) => {
    let path = "/";
    if (next.kind === "site") path = `/sites/${next.siteId}${next.pageSlug ? `/p/${next.pageSlug}` : ""}`;
    else if (next.kind === "preview") path = `/p/${next.siteSlug}${next.pageSlug ? `/${next.pageSlug}` : ""}`;
    if (window.location.pathname !== path) {
      window.history.pushState({}, "", path);
    }
    setRoute(next);
  }, []);
  return [route, navigate];
}

export function StageApp() {
  return (
    <ToastProvider>
      <Router />
    </ToastProvider>
  );
}

function Router() {
  const [route, navigate] = useRoute();
  if (route.kind === "preview") {
    return <PublicPreview siteSlug={route.siteSlug} pageSlug={route.pageSlug} />;
  }
  return <Editor route={route} navigate={navigate} />;
}

// ---------------------------------------------------------------------------
// Editor — the authenticated app
// ---------------------------------------------------------------------------

function Editor({ route, navigate }: {
  route: Route;
  navigate: (next: Route) => void;
}) {
  const [currentUser, setCurrentUser] = useState<User | null>(() => {
    try {
      const token = localStorage.getItem(storageKey("token"));
      const cached = localStorage.getItem(storageKey("user"));
      return token && cached ? (JSON.parse(cached) as User) : null;
    } catch {
      return null;
    }
  });
  const { tenantId: activeOrgId } = useSession(db.sync);

  // Derive site state from the URL rather than duplicating it — "open
  // a site" is just pushState('/sites/:id'). Back/forward navigate
  // between dashboard and editor for free.
  const activeSiteId = route.kind === "site" ? route.siteId : null;
  const urlPageSlug = route.kind === "site" ? route.pageSlug : undefined;
  function openSite(siteId: string) { navigate({ kind: "site", siteId }); }
  function exitToDashboard() { navigate({ kind: "dashboard" }); }

  async function signOut() {
    const token = localStorage.getItem(storageKey("token"));
    localStorage.removeItem(storageKey("token"));
    localStorage.removeItem(storageKey("user"));
    if (token) {
      fetch(`${BASE_URL}/api/auth/session`, {
        method: "DELETE",
        headers: { Authorization: `Bearer ${token}` },
      }).catch(() => {});
    }
    try { indexedDB.deleteDatabase("pylon_sync_stage"); } catch {}
    setCurrentUser(null);
    exitToDashboard();
    await db.sync.notifySessionChanged();
  }

  async function selectOrg(orgId: string | null) {
    const token = localStorage.getItem(storageKey("token"));
    if (!token) return;
    const res = await fetch(`${BASE_URL}/api/auth/select-org`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
      body: JSON.stringify({ orgId }),
    });
    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      throw new Error(err.error?.message || `switch failed (${res.status})`);
    }
    await db.sync.notifySessionChanged();
  }

  if (!currentUser) return <Login onReady={setCurrentUser} />;

  return (
    <OrgGate
      currentUser={currentUser}
      activeOrgId={activeOrgId}
      onSelectOrg={selectOrg}
      onSignOut={signOut}
    >
      {activeSiteId ? (
        <SiteEditor
          siteId={activeSiteId}
          currentUser={currentUser}
          urlPageSlug={urlPageSlug}
          onNavigatePage={(pageSlug) => navigate({ kind: "site", siteId: activeSiteId, pageSlug })}
          onExit={exitToDashboard}
          onSignOut={signOut}
        />
      ) : (
        <Dashboard
          currentUser={currentUser}
          onOpen={openSite}
          onSignOut={signOut}
        />
      )}
    </OrgGate>
  );
}

// ---------------------------------------------------------------------------
// Login
// ---------------------------------------------------------------------------

function Login({ onReady }: { onReady: (u: User) => void }) {
  const [email, setEmail] = useState("director@stage.dev");
  const [name, setName] = useState("Director");
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function go(e?: React.FormEvent) {
    e?.preventDefault();
    setLoading(true);
    setErr(null);
    try {
      const session = await fetch(`${BASE_URL}/api/auth/guest`, { method: "POST" }).then((r) => r.json());
      const token: string = session.token;
      localStorage.setItem(storageKey("token"), token);
      configureClient({ baseUrl: BASE_URL, appName: "stage" });
      const user = await callFn<User>("upsertUser", { email, displayName: name });
      await fetch(`${BASE_URL}/api/auth/upgrade`, {
        method: "POST",
        headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
        body: JSON.stringify({ user_id: user.id }),
      });
      localStorage.setItem(storageKey("user"), JSON.stringify(user));
      void db.sync.notifySessionChanged();
      onReady(user);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="login">
      <div className="login-panel">
        <div className="brand" style={{ marginBottom: 24 }}>
          <div className="brand-mark">S</div>
          Stage
        </div>
        <h1 style={{ fontSize: 28, fontWeight: 700, letterSpacing: "-0.01em", marginBottom: 6 }}>
          Sign in
        </h1>
        <p style={{ color: "var(--text-muted)", marginBottom: 24, lineHeight: 1.5 }}>
          Build and ship landing pages fast. Drag blocks onto a canvas, publish a
          public URL, watch edits sync live.
        </p>
        <form onSubmit={go}>
          <div className="field">
            <label className="field-label">Email</label>
            <input className="insp-input" autoFocus value={email} onChange={(e) => setEmail(e.target.value)} />
          </div>
          <div className="field">
            <label className="field-label">Display name</label>
            <input className="insp-input" value={name} onChange={(e) => setName(e.target.value)} />
          </div>
          {err && <div style={{ color: "var(--danger)", fontSize: 12.5, marginBottom: 10 }}>{err}</div>}
          <button className="btn primary" disabled={loading} style={{ width: "100%" }} type="submit">
            {loading ? "Signing in…" : "Continue"}
          </button>
        </form>
      </div>
      <div className="login-art">
        <div className="login-art-content">
          <span className="eyebrow">New — realtime multiplayer</span>
          <h2>Ship sites<br/>that feel <em>alive.</em></h2>
          <p>
            A canvas-first site builder with responsive breakpoints, reusable
            components, and cursors that sync across every teammate's browser.
            Publish with one click.
          </p>
        </div>
        {/* Floating mock card — gives the sign-in screen a real product to point at. */}
        <div className="login-mock">
          <div className="login-mock-topbar">
            <span/><span/><span/>
          </div>
          <div className="login-mock-body">
            <h4>Ship something good this week.</h4>
            <p>Stage turns ideas into shippable pages in minutes.</p>
            <span className="login-mock-btn">
              Get started <Icon name="ArrowRight" size={12} />
            </span>
          </div>
          <div className="login-mock-cursor">
            <svg width="18" height="18" viewBox="0 0 18 18">
              <path d="M1 1 L1 14 L5 10.5 L7.5 16 L10 15 L7.5 9.5 L13 9.5 Z" fill="#7C5CFF" stroke="white" strokeWidth="1" />
            </svg>
            <span style={{
              display: "inline-block", padding: "2px 8px", marginTop: 4, marginLeft: 10,
              borderRadius: 999, fontSize: 10, fontWeight: 600, color: "white",
              background: "#7C5CFF", whiteSpace: "nowrap",
            }}>Alex</span>
          </div>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// OrgGate + onboarding
// ---------------------------------------------------------------------------

function OrgGate({
  currentUser, activeOrgId, onSelectOrg, onSignOut, children,
}: {
  currentUser: User;
  activeOrgId: string | null;
  onSelectOrg: (orgId: string | null) => Promise<void>;
  onSignOut: () => void;
  children: React.ReactNode;
}) {
  const { data: memberships } = db.useQuery<OrgMember>("OrgMember", {
    where: { userId: currentUser.id },
  });
  const { data: organizations } = db.useQuery<Organization>("Organization");

  const myOrgs = useMemo(() => {
    const byId = new Map<string, Organization>();
    for (const o of organizations ?? []) byId.set(o.id, o);
    const rows: Organization[] = [];
    for (const m of memberships ?? []) {
      const org = byId.get(m.orgId);
      if (org) rows.push(org);
    }
    return rows.sort((a, b) => a.name.localeCompare(b.name));
  }, [memberships, organizations]);

  useEffect(() => {
    if (!activeOrgId && myOrgs.length === 1) {
      void onSelectOrg(myOrgs[0].id);
    }
  }, [activeOrgId, myOrgs]);

  if (activeOrgId && myOrgs.find((o) => o.id === activeOrgId)) return <>{children}</>;

  return (
    <Onboarding
      currentUser={currentUser}
      myOrgs={myOrgs}
      onSelectOrg={onSelectOrg}
      onSignOut={onSignOut}
    />
  );
}

function Onboarding({
  currentUser, myOrgs, onSelectOrg, onSignOut,
}: {
  currentUser: User;
  myOrgs: Organization[];
  onSelectOrg: (orgId: string) => Promise<void>;
  onSignOut: () => void;
}) {
  const [createOpen, setCreateOpen] = useState(myOrgs.length === 0);
  return (
    <div style={{
      height: "100vh", display: "grid", placeItems: "center",
      background: "radial-gradient(circle at 25% 25%, #f9a8d4 0%, #a78bfa 45%, #312e81 100%)",
    }}>
      <div style={{
        width: "min(440px, 92vw)", background: "var(--surface)",
        borderRadius: 12, padding: 24, boxShadow: "var(--shadow-lg)",
      }}>
        <div className="brand" style={{ marginBottom: 20 }}>
          <div className="brand-mark">S</div>
          Stage
        </div>
        <h2 style={{ fontSize: 18, fontWeight: 600, marginBottom: 12 }}>
          Hi {currentUser.displayName.split(" ")[0]} — pick a workspace
        </h2>
        {myOrgs.length === 0 ? (
          <p style={{ color: "var(--text-muted)", fontSize: 13, marginBottom: 16 }}>
            Create your first workspace to get started.
          </p>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 6, marginBottom: 16 }}>
            {myOrgs.map((o) => (
              <button key={o.id} onClick={() => onSelectOrg(o.id)}
                style={{
                  textAlign: "left", padding: "10px 14px", borderRadius: 8,
                  border: "1px solid var(--border)", background: "var(--surface-raised)",
                }}>
                <div style={{ fontWeight: 500 }}>{o.name}</div>
                <div style={{ fontSize: 12, color: "var(--text-dim)" }}>{o.slug}</div>
              </button>
            ))}
          </div>
        )}
        <div style={{ display: "flex", gap: 8 }}>
          <button className="btn primary" onClick={() => setCreateOpen(true)}>New workspace</button>
          <button className="btn ghost" onClick={onSignOut}>Sign out</button>
        </div>
      </div>
      {createOpen && (
        <NewOrgModal
          onClose={() => setCreateOpen(false)}
          onCreated={async (id) => onSelectOrg(id)}
        />
      )}
    </div>
  );
}

function NewOrgModal({ onClose, onCreated }: {
  onClose: () => void;
  onCreated: (orgId: string) => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true); setErr(null);
    try {
      const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
      const r = await callFn<{ orgId: string }>("createOrganization", { name, slug });
      await onCreated(r.orgId);
      onClose();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally { setBusy(false); }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <form className="modal" onClick={(e) => e.stopPropagation()} onSubmit={submit}>
        <h2>Create workspace</h2>
        <p>Workspaces own sites. Team members share workspace access.</p>
        <div className="field">
          <label className="field-label">Workspace name</label>
          <input className="insp-input" autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="Acme Studio" />
        </div>
        {err && <div style={{ color: "var(--danger)", fontSize: 12.5, marginBottom: 10 }}>{err}</div>}
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
          <button type="button" className="btn ghost" onClick={onClose}>Cancel</button>
          <button className="btn primary" disabled={busy || !name.trim()}>{busy ? "Creating…" : "Create"}</button>
        </div>
      </form>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Dashboard — list of sites
// ---------------------------------------------------------------------------

function Dashboard({
  currentUser, onOpen, onSignOut,
}: {
  currentUser: User;
  onOpen: (siteId: string) => void;
  onSignOut: () => void;
}) {
  const { data: sites } = db.useQuery<Site>("Site");
  const [createOpen, setCreateOpen] = useState(false);
  const sorted = useMemo(
    () => [...(sites ?? [])].sort((a, b) => (a.createdAt < b.createdAt ? 1 : -1)),
    [sites],
  );

  return (
    <>
      <div className="topbar">
        <div className="brand"><div className="brand-mark">S</div>Stage</div>
        <div className="tb-spacer" />
        <button className="btn ghost" onClick={onSignOut}>
          <span style={{
            display: "inline-block", width: 20, height: 20, borderRadius: "50%",
            background: currentUser.avatarColor, color: "white",
            textAlign: "center", lineHeight: "20px",
            fontSize: 11, fontWeight: 600, marginRight: 6,
          }}>
            {currentUser.displayName[0]}
          </span>
          Sign out
        </button>
      </div>
      <div className="dashboard">
        <div className="dashboard-header">
          <div>
            <h1>Your sites</h1>
            <p className="lede">
              {sorted.length > 0
                ? `${sorted.length} ${sorted.length === 1 ? "site" : "sites"} in ${/* workspace hint */ "this workspace"} · drag blocks, invite teammates, ship fast.`
                : "Create a site, drop in blocks, invite teammates, publish in one click."}
            </p>
          </div>
          <button className="btn primary" onClick={() => setCreateOpen(true)}>
            <Icon name="Plus" size={14} /> New site
          </button>
        </div>
        {sorted.length === 0 ? (
          <div className="empty-state">
            <div className="empty-illus" style={{ display: "inline-flex" }}>
              <Icon name="Sparkles" size={40} strokeWidth={1.5} />
            </div>
            <h2>Your canvas is blank</h2>
            <p>Create your first site — it comes seeded with a home page and starter blocks.</p>
            <button className="btn primary" onClick={() => setCreateOpen(true)}>
              <Icon name="Plus" size={14} /> Create your first site
            </button>
          </div>
        ) : (
          <div className="site-grid">
            {sorted.map((s, i) => (
              <SiteCard key={s.id} site={s} index={i} onClick={() => onOpen(s.id)} />
            ))}
          </div>
        )}
      </div>
      {createOpen && (
        <NewSiteModal
          onClose={() => setCreateOpen(false)}
          onCreated={(siteId) => { setCreateOpen(false); onOpen(siteId); }}
        />
      )}
    </>
  );
}

// Site card with mini preview — shows a scaled-down stack of line
// placeholders colored by the site's accent, plus an emoji chip. Real
// preview rendering of blocks would be beautiful but for now the
// stylized lines feel like a designer's mock and match Framer's grid.
function SiteCard({ site, index, onClick }: {
  site: Site; index: number; onClick: () => void;
}) {
  const { data: homePages } = db.useQuery<Page>("Page", { where: { siteId: site.id, slug: "/" } });
  const homePageId = homePages?.[0]?.id;
  const { data: blocks } = db.useQuery<Block>("Block", {
    where: homePageId ? { pageId: homePageId } : undefined,
  });
  const sortedBlocks = useMemo(
    () => [...(blocks ?? [])]
      .filter((b) => !b.parentId)
      .sort((a, b) => a.sort - b.sort)
      .slice(0, 5),
    [blocks],
  );

  // Stagger entry animation by index so the grid cascades in.
  return (
    <div
      className="site-card"
      onClick={onClick}
      style={{
        animationDelay: `${Math.min(index * 40, 240)}ms`,
        ["--thumb-bg" as any]: `linear-gradient(135deg, ${site.accentColor}, #7C5CFF)`,
      }}
    >
      <div className="site-thumb">
        <div className="site-thumb-emoji"><SiteFavicon site={site} /></div>
        <div className="site-thumb-stack">
          {sortedBlocks.length > 0 ? sortedBlocks.map((b, i) => {
            const cls = b.type === "heading" ? "short" : b.type === "button" ? "short" : i % 2 === 0 ? "long" : "med";
            return <div key={b.id} className={`site-thumb-line ${cls}`} />;
          }) : (
            // Placeholder lines for brand-new sites.
            <>
              <div className="site-thumb-line short" />
              <div className="site-thumb-line long" />
              <div className="site-thumb-line med" />
            </>
          )}
        </div>
      </div>
      <div className="site-info">
        <div className="site-title">{site.name}</div>
        <div className="site-meta">
          <code>/p/{site.slug}</code>
          <span className={`pub ${site.publishedAt ? "on" : "off"}`}>
            {site.publishedAt ? "Live" : "Draft"}
          </span>
        </div>
      </div>
    </div>
  );
}

function NewSiteModal({ onClose, onCreated }: {
  onClose: () => void;
  onCreated: (siteId: string) => void;
}) {
  const [name, setName] = useState("");
  const [slug, setSlug] = useState("");
  const [emoji, setEmoji] = useState("Sparkles");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  function autoSlug(v: string) {
    return v.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true); setErr(null);
    try {
      const r = await callFn<{ siteId: string }>("createSite", {
        name,
        slug: slug || autoSlug(name),
        faviconEmoji: emoji,
      });
      onCreated(r.siteId);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally { setBusy(false); }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <form className="modal" onClick={(e) => e.stopPropagation()} onSubmit={submit}>
        <h2>New site</h2>
        <p>Sites start with a home page and a few starter blocks.</p>
        <div style={{ display: "grid", gridTemplateColumns: "96px 1fr", gap: 12 }}>
          <div className="field">
            <label className="field-label">Icon</label>
            <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
              <span style={{
                display: "inline-flex", alignItems: "center", justifyContent: "center",
                width: 34, height: 34, borderRadius: "var(--rounded-sm)",
                background: "var(--color-surface-raised)", border: "1px solid var(--color-outline)",
              }}>
                <Icon name={/^[A-Z][a-zA-Z0-9]+$/.test(emoji) ? emoji : "Sparkles"} size={18} />
              </span>
              <input className="insp-input" value={emoji} onChange={(e) => setEmoji(e.target.value)} maxLength={40}
                placeholder="Lucide name" style={{ flex: 1 }} />
            </div>
          </div>
          <div className="field">
            <label className="field-label">Name</label>
            <input className="insp-input" autoFocus value={name}
              onChange={(e) => { setName(e.target.value); setSlug(autoSlug(e.target.value)); }}
              placeholder="Launch Page" />
          </div>
        </div>
        <div className="field">
          <label className="field-label">Public slug</label>
          <input className="insp-input" value={slug} onChange={(e) => setSlug(e.target.value)} placeholder="launch-page" />
          <span style={{ fontSize: 11.5, color: "var(--text-dim)" }}>
            Your site will live at <code>/p/{slug || "…"}</code>
          </span>
        </div>
        {err && <div style={{ color: "var(--danger)", fontSize: 12.5, marginBottom: 10 }}>{err}</div>}
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
          <button type="button" className="btn ghost" onClick={onClose}>Cancel</button>
          <button className="btn primary" disabled={busy || !name.trim()}>{busy ? "Creating…" : "Create site"}</button>
        </div>
      </form>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Site editor
// ---------------------------------------------------------------------------

// Undo/redo operates over per-site stacks of inverse mutations. Kept
// client-side because the change log is authoritative; we just
// reproduce user actions in reverse. Scope: block prop edits + delete
// (which inserts back the block) + moves. Not: cross-tab undo — each
// session has its own history.
type UndoOp =
  | { kind: "updateBlock"; blockId: string; prevPropsJson: string }
  | { kind: "deleteBlock"; block: Block };

function SiteEditor({
  siteId, currentUser, urlPageSlug, onNavigatePage, onExit, onSignOut,
}: {
  siteId: string;
  currentUser: User;
  urlPageSlug?: string;
  onNavigatePage: (pageSlug: string) => void;
  onExit: () => void;
  onSignOut: () => void;
}) {
  const { data: site } = db.useQueryOne<Site>("Site", siteId);
  const { data: pages } = db.useQuery<Page>("Page", { where: { siteId } });
  const { data: components } = db.useQuery<Component>("Component", { where: { siteId } });
  const sortedPages = useMemo(
    () => [...(pages ?? [])].sort((a, b) => a.sort - b.sort),
    [pages],
  );
  const [activePageId, setActivePageId] = useState<string | null>(null);
  const [editingComponentId, setEditingComponentId] = useState<string | null>(null);
  const [selectedBlockId, setSelectedBlockId] = useState<string | null>(null);
  const [inspectorTab, setInspectorTab] = useState<"block" | "site">("block");
  // `viewMode` controls canvas rendering (single vs. all three); the
  // separate `editBreakpoint` is which breakpoint inspector edits write
  // to. In compare mode the edit breakpoint shifts to whichever
  // artboard was last clicked — intuitive "I clicked the phone canvas
  // so I'm editing phone now". Defaults to "all" so new users see
  // every breakpoint at once, Framer-style.
  const [viewMode, setViewMode] = useState<ViewMode>("all");
  const [editBreakpoint, setEditBreakpoint] = useState<Breakpoint>("desktop");
  function handleViewModeChange(next: ViewMode) {
    setViewMode(next);
    if (next !== "all") setEditBreakpoint(next);
  }

  // Undo/redo stacks — most recent on the right.
  const undoRef = useRef<UndoOp[]>([]);
  const redoRef = useRef<UndoOp[]>([]);
  const [undoBump, setUndoBump] = useState(0); // force re-render on stack changes
  const pushUndo = useCallback((op: UndoOp) => {
    undoRef.current.push(op);
    redoRef.current = []; // new action invalidates forward history
    if (undoRef.current.length > 50) undoRef.current.shift();
    setUndoBump((x) => x + 1);
  }, []);

  // Realtime collab: per-site room with compact presence (cursor pos,
  // selected block id, user name/color). Heartbeat tightened to 300ms
  // so peer cursors feel near-instant.
  const roomName = `stage:site:${siteId}`;
  const { peers, setPresence } = useRoom(roomName, currentUser.id, {
    baseUrl: BASE_URL,
    heartbeatInterval: 300,
    initialPresence: { name: currentUser.displayName, color: colorForUser(currentUser.id) },
  });

  // Sync active page with URL. URL slug wins: if the URL carries a
  // page slug (including "home" for "/"), pick that page. Otherwise
  // fall back to the first page in sort order so the editor never
  // shows an empty canvas.
  useEffect(() => {
    if (sortedPages.length === 0) return;
    const matchByUrl =
      urlPageSlug !== undefined
        ? sortedPages.find((p) => slugSegment(p.slug) === urlPageSlug)
        : undefined;
    const target = matchByUrl ?? sortedPages[0];
    if (activePageId !== target.id) setActivePageId(target.id);
  }, [sortedPages, urlPageSlug, activePageId]);

  // When the user picks a page locally, update the URL so it's
  // shareable. "/" normalizes to "home" in the URL for readability.
  function setActivePageAndUrl(id: string) {
    setActivePageId(id);
    const page = sortedPages.find((p) => p.id === id);
    if (page) onNavigatePage(slugSegment(page.slug));
  }

  // Broadcast our selected block to peers. Cursor position updates
  // happen inline in the canvas mousemove handler — keeping those out
  // of React state avoids a render on every pixel of movement.
  useEffect(() => {
    setPresence({
      name: currentUser.displayName,
      color: colorForUser(currentUser.id),
      selectedBlockId,
    });
  }, [selectedBlockId, currentUser.displayName, currentUser.id, setPresence]);

  // ⌘Z / ⌘⇧Z for undo/redo. Also Esc to deselect.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const target = e.target as HTMLElement;
      const typing = target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable);
      if (typing) return;
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "z") {
        e.preventDefault();
        if (e.shiftKey) void redo();
        else void undo();
      }
      if (e.key === "Escape") setSelectedBlockId(null);
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // Canvas pan + zoom. Framer-style: trackpad pinch or ctrl/cmd + wheel
  // zooms; regular wheel + shift-wheel pans; space+drag also pans.
  // Applied as a CSS transform on .compare-stack / .canvas inside the
  // .canvas-shell so the artboards themselves are untouched.
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [spaceHeld, setSpaceHeld] = useState(false);
  const panState = useRef<{
    dragging: boolean; startX: number; startY: number;
    origX: number; origY: number; justDragged: boolean;
  }>({
    dragging: false, startX: 0, startY: 0, origX: 0, origY: 0, justDragged: false,
  });
  function resetView() { setZoom(1); setPan({ x: 0, y: 0 }); }
  function zoomBy(factor: number) {
    setZoom((z) => Math.max(0.25, Math.min(2, z * factor)));
  }
  useEffect(() => {
    function down(e: KeyboardEvent) {
      const tgt = e.target as HTMLElement;
      const typing = tgt && (tgt.tagName === "INPUT" || tgt.tagName === "TEXTAREA" || tgt.isContentEditable);
      if (typing) return;
      if (e.code === "Space") { e.preventDefault(); setSpaceHeld(true); }
    }
    function up(e: KeyboardEvent) {
      if (e.code === "Space") setSpaceHeld(false);
    }
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
    };
  }, []);

  if (!site) {
    return <div className="shell"><div style={{ padding: 40 }}>Loading site…</div></div>;
  }

  async function togglePublish() {
    if (!site) return;
    await callFn("publishSite", { siteId, publish: !site.publishedAt });
  }

  async function undo() {
    const op = undoRef.current.pop();
    if (!op) return;
    if (op.kind === "updateBlock") {
      // Capture current to push onto redo stack.
      const now = db.sync.store.get("Block", op.blockId) as Block | null;
      if (now) redoRef.current.push({ kind: "updateBlock", blockId: op.blockId, prevPropsJson: now.propsJson });
      await callFn("updateBlock", { blockId: op.blockId, propsJson: op.prevPropsJson });
    }
    setUndoBump((x) => x + 1);
  }
  async function redo() {
    const op = redoRef.current.pop();
    if (!op) return;
    if (op.kind === "updateBlock") {
      const now = db.sync.store.get("Block", op.blockId) as Block | null;
      if (now) undoRef.current.push({ kind: "updateBlock", blockId: op.blockId, prevPropsJson: now.propsJson });
      await callFn("updateBlock", { blockId: op.blockId, propsJson: op.prevPropsJson });
    }
    setUndoBump((x) => x + 1);
  }

  return (
    <IconLibraryProvider library={resolveSiteTokens(site).iconLibrary ?? "lucide"}>
    <div className="shell">
      <div className="topbar">
        <button className="btn ghost" onClick={onExit}>
          <Icon name="ChevronLeft" size={14} /> Sites
        </button>
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <SiteFavicon site={site} />
          <div style={{ fontWeight: 600, letterSpacing: "-0.005em" }}>{site.name}</div>
          <span style={{ fontSize: 11, color: "var(--color-text-dim)", fontFamily: "JetBrains Mono, ui-monospace, monospace" }}>
            /p/{site.slug}
          </span>
        </div>

        <div className="tb-spacer" />

        <BreakpointSwitcher value={viewMode} onChange={handleViewModeChange} />

        <ZoomControls
          zoom={zoom}
          onZoomIn={() => zoomBy(1.2)}
          onZoomOut={() => zoomBy(1 / 1.2)}
          onReset={resetView}
        />

        <div className="tb-group">
          <button className="btn icon" onClick={undo} disabled={undoRef.current.length === 0} title="Undo (⌘Z)">
            <Icon name="Undo2" size={14} />
          </button>
          <button className="btn icon" onClick={redo} disabled={redoRef.current.length === 0} title="Redo (⌘⇧Z)">
            <Icon name="Redo2" size={14} />
          </button>
        </div>

        <PeerAvatars peers={peers} />

        <span className={`tb-chip ${site.publishedAt ? "live" : ""}`}>
          <span className="dot" />
          {site.publishedAt ? "Live" : "Draft"}
        </span>
        <a className="btn ghost" href={`/p/${site.slug}`} target="_blank" rel="noreferrer">
          Preview <Icon name="ArrowUpRight" size={12} />
        </a>
        <button className="btn primary" onClick={togglePublish}>
          {site.publishedAt ? "Unpublish" : "Publish"}
        </button>
        <button className="btn ghost" onClick={onSignOut} title={currentUser.displayName}>
          <span style={{
            display: "inline-block", width: 22, height: 22, borderRadius: "50%",
            background: currentUser.avatarColor, color: "white",
            textAlign: "center", lineHeight: "22px",
            fontSize: 11, fontWeight: 600,
          }}>
            {currentUser.displayName[0]}
          </span>
        </button>
      </div>
      <div className="body">
        <PageNav
          siteId={siteId}
          pages={sortedPages}
          components={components ?? []}
          activePageId={activePageId}
          editingComponentId={editingComponentId}
          setActivePageId={(id) => { setActivePageAndUrl(id); setEditingComponentId(null); setSelectedBlockId(null); }}
          setEditingComponentId={(id) => { setEditingComponentId(id); setActivePageId(null); setSelectedBlockId(null); }}
        />
        <div
          className={`canvas-shell ${viewMode === "all" ? "compare" : ""} ${spaceHeld || panState.current.dragging ? "panning" : ""}`}
          onClick={(e) => {
            // Suppress click-to-deselect when the user just finished dragging
            // to pan — otherwise pans always collapse the inspector.
            if (panState.current.justDragged) {
              panState.current.justDragged = false;
              return;
            }
            setSelectedBlockId(null);
          }}
          onWheel={(e) => {
            // Ctrl/⌘ + wheel zooms around the cursor; plain wheel pans
            // (Figma/Framer behavior). Two-finger trackpad scroll sends
            // both deltaX and deltaY so it feels like moving the canvas.
            if (e.ctrlKey || e.metaKey) {
              e.preventDefault();
              const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
              const cx = e.clientX - rect.left;
              const cy = e.clientY - rect.top;
              const factor = e.deltaY < 0 ? 1.08 : 1 / 1.08;
              setZoom((z) => {
                const next = Math.max(0.25, Math.min(2, z * factor));
                const ratio = next / z;
                // Zoom around the cursor: adjust pan so the point under
                // the cursor stays fixed on-screen.
                setPan((p) => ({
                  x: cx - (cx - p.x) * ratio,
                  y: cy - (cy - p.y) * ratio,
                }));
                return next;
              });
              return;
            }
            e.preventDefault();
            setPan((p) => ({ x: p.x - e.deltaX, y: p.y - e.deltaY }));
          }}
          onMouseDown={(e) => {
            // Space+drag or middle-click drag both pan.
            const isPanDrag = (spaceHeld && e.button === 0) || e.button === 1;
            if (isPanDrag) {
              e.preventDefault();
              panState.current = {
                dragging: true,
                startX: e.clientX, startY: e.clientY,
                origX: pan.x, origY: pan.y,
                justDragged: false,
              };
            }
          }}
          onMouseMove={(e) => {
            if (panState.current.dragging) {
              setPan({
                x: panState.current.origX + (e.clientX - panState.current.startX),
                y: panState.current.origY + (e.clientY - panState.current.startY),
              });
            }
          }}
          onMouseUp={() => {
            if (panState.current.dragging) {
              panState.current.justDragged = true;
            }
            panState.current.dragging = false;
          }}
          onMouseLeave={() => { panState.current.dragging = false; }}
        >
          <div
            className="canvas-viewport"
            style={{
              transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`,
              transformOrigin: "0 0",
              transition: panState.current.dragging ? "none" : "transform 140ms cubic-bezier(0.2, 0.8, 0.2, 1)",
            }}
          >
          {(activePageId || editingComponentId) ? (
            viewMode === "all" ? (
              <div className="compare-stack">
                {(["desktop", "tablet", "phone"] as const).map((bp) => (
                  <PageCanvas
                    key={bp}
                    pageId={activePageId}
                    componentId={editingComponentId}
                    site={site}
                    breakpoint={bp}
                    activeEdit={editBreakpoint === bp}
                    selectedBlockId={selectedBlockId}
                    onSelect={(id) => {
                      setSelectedBlockId(id);
                      setInspectorTab("block");
                      setEditBreakpoint(bp);
                    }}
                    onPresence={(x, y) => {
                      setPresence({
                        name: currentUser.displayName,
                        color: colorForUser(currentUser.id),
                        selectedBlockId,
                        cursor: { x, y },
                      });
                    }}
                    peers={peers}
                    pushUndo={pushUndo}
                  />
                ))}
              </div>
            ) : (
              <PageCanvas
                pageId={activePageId}
                componentId={editingComponentId}
                site={site}
                breakpoint={viewMode}
                activeEdit={true}
                selectedBlockId={selectedBlockId}
                onSelect={(id) => {
                  setSelectedBlockId(id);
                  setInspectorTab("block");
                  setEditBreakpoint(viewMode);
                }}
                onPresence={(x, y) => {
                  setPresence({
                    name: currentUser.displayName,
                    color: colorForUser(currentUser.id),
                    selectedBlockId,
                    cursor: { x, y },
                  });
                }}
                peers={peers}
                pushUndo={pushUndo}
              />
            )
          ) : (
            <div className="canvas-empty">Pick a page on the left to start editing.</div>
          )}
          </div>{/* /canvas-viewport */}
        </div>
        <Inspector
          site={site}
          blockId={selectedBlockId}
          breakpoint={editBreakpoint}
          tab={inspectorTab}
          setTab={setInspectorTab}
          pushUndo={pushUndo}
        />
      </div>
    </div>
    </IconLibraryProvider>
  );
}

// ---------------------------------------------------------------------------
// Topbar widgets
// ---------------------------------------------------------------------------

function BreakpointSwitcher({ value, onChange }: { value: ViewMode; onChange: (v: ViewMode) => void }) {
  return (
    <div className="seg">
      {BREAKPOINTS.map((bp) => (
        <button
          key={bp.id}
          className={bp.id === value ? "active" : ""}
          onClick={() => onChange(bp.id)}
          title={`${bp.label} · ${bp.width}px`}
        >
          <Icon name={bp.icon} size={14} />
          <span>{bp.label}</span>
        </button>
      ))}
      <button
        className={value === "all" ? "active" : ""}
        onClick={() => onChange("all")}
        title="Compare all breakpoints side-by-side"
      >
        <Icon name="LayoutGrid" size={14} />
        <span>All</span>
      </button>
    </div>
  );
}

// Renders a site's "favicon" — accepts an emoji (1-4 chars) or a
// Lucide icon name (PascalCase). Heuristic: if the string matches a
// typed identifier, treat as icon; else render as text. Keeps the
// user-picked brand free-form while letting them type real icons.
function SiteFavicon({ site }: { site: Site }) {
  const v = site.faviconEmoji;
  if (!v) return <Icon name="Sparkles" size={17} />;
  const looksLikeIconName = /^[A-Z][a-zA-Z0-9]+$/.test(v);
  if (looksLikeIconName) return <Icon name={v} size={17} />;
  return <div style={{ fontSize: 17 }}>{v}</div>;
}

function ZoomControls({
  zoom, onZoomIn, onZoomOut, onReset,
}: {
  zoom: number;
  onZoomIn: () => void;
  onZoomOut: () => void;
  onReset: () => void;
}) {
  return (
    <div className="tb-group" title="Zoom — ctrl/cmd + wheel to zoom, space+drag to pan">
      <button className="btn icon" onClick={onZoomOut} title="Zoom out">
        <Icon name="Minus" size={14} />
      </button>
      <button className="btn ghost" onClick={onReset} style={{ minWidth: 56, fontSize: 12, fontWeight: 500 }} title="Reset view">
        {Math.round(zoom * 100)}%
      </button>
      <button className="btn icon" onClick={onZoomIn} title="Zoom in">
        <Icon name="Plus" size={14} />
      </button>
    </div>
  );
}

function PeerAvatars({ peers }: { peers: { user_id: string; data: any }[] }) {
  if (!peers || peers.length === 0) return null;
  return (
    <div style={{ display: "flex", alignItems: "center", gap: -6 }}>
      {peers.slice(0, 4).map((p) => {
        const color = p.data?.color ?? colorForUser(p.user_id);
        const name = p.data?.name ?? "…";
        return (
          <div key={p.user_id}
            title={name}
            style={{
              width: 26, height: 26, borderRadius: "50%",
              background: color, color: "white",
              display: "grid", placeItems: "center",
              fontSize: 11, fontWeight: 600,
              border: "2px solid var(--color-surface)",
              marginLeft: -6,
            }}>
            {name[0]?.toUpperCase() ?? "?"}
          </div>
        );
      })}
      {peers.length > 4 && (
        <div style={{
          marginLeft: -6, fontSize: 11, fontWeight: 500, color: "var(--color-text-muted)",
          padding: "0 6px",
        }}>
          +{peers.length - 4}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Page nav
// ---------------------------------------------------------------------------

function PageNav({
  siteId, pages, components, activePageId, editingComponentId,
  setActivePageId, setEditingComponentId,
}: {
  siteId: string;
  pages: Page[];
  components: Component[];
  activePageId: string | null;
  editingComponentId: string | null;
  setActivePageId: (id: string) => void;
  setEditingComponentId: (id: string) => void;
}) {
  const [addingPage, setAddingPage] = useState(false);
  const [newTitle, setNewTitle] = useState("");
  const [addingComponent, setAddingComponent] = useState(false);
  const [newCompName, setNewCompName] = useState("");
  const [confirmingDeletePageId, setConfirmingDeletePageId] = useState<string | null>(null);
  const t = useToast();

  async function createComponent() {
    if (!newCompName.trim()) return;
    try {
      const r = await callFn<{ componentId: string }>("createComponent", {
        siteId, name: newCompName.trim(),
      });
      setEditingComponentId(r.componentId);
      setAddingComponent(false); setNewCompName("");
    } catch (e) {
      t.error("Couldn't create component", e instanceof Error ? e.message : String(e));
    }
  }

  async function createPage() {
    if (!newTitle.trim()) return;
    const slug = newTitle.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
    try {
      const r = await callFn<{ id: string }>("createPage", {
        siteId, title: newTitle, slug,
      });
      setActivePageId(r.id);
      setAddingPage(false); setNewTitle("");
    } catch (e) {
      t.error("Couldn't create page", e instanceof Error ? e.message : String(e));
    }
  }

  async function confirmDeletePage() {
    if (!confirmingDeletePageId) return;
    const id = confirmingDeletePageId;
    try {
      await callFn("deletePage", { pageId: id });
      t.success("Page deleted");
    } catch (e) {
      t.error("Couldn't delete page", e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <aside className="nav">
      <div className="nav-section">
        <span>Pages</span>
        <button onClick={() => setAddingPage((x) => !x)}>+</button>
      </div>
      {addingPage && (
        <div style={{ padding: "4px 6px 8px" }}>
          <input
            className="insp-input"
            placeholder="Page title"
            autoFocus
            value={newTitle}
            onChange={(e) => setNewTitle(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") createPage();
              if (e.key === "Escape") { setAddingPage(false); setNewTitle(""); }
            }}
          />
        </div>
      )}
      {pages.map((p) => (
        <div
          key={p.id}
          className={`page-item ${activePageId === p.id ? "active" : ""}`}
          onClick={() => setActivePageId(p.id)}
        >
          <span style={{ display: "flex", alignItems: "center", gap: 6, overflow: "hidden" }}>
            <span style={{ display: "inline-flex", color: "var(--color-text-muted)" }}>
              <Icon name={p.slug === "/" ? "Home" : "FileText"} size={13} />
            </span>
            <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{p.title}</span>
          </span>
          <span className="page-slug">{p.slug}</span>
          {pages.length > 1 && (
            <button
              onClick={(e) => { e.stopPropagation(); setConfirmingDeletePageId(p.id); }}
              style={{ color: "var(--color-text-dim)", display: "inline-flex" }}
              title="Delete page"
            ><Icon name="X" size={13} /></button>
          )}
        </div>
      ))}

      <div className="nav-section">
        <span>Components</span>
        <button onClick={() => setAddingComponent((x) => !x)}>+</button>
      </div>
      {addingComponent && (
        <div style={{ padding: "4px 6px 8px" }}>
          <input
            className="insp-input"
            placeholder="Component name"
            autoFocus
            value={newCompName}
            onChange={(e) => setNewCompName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") createComponent();
              if (e.key === "Escape") { setAddingComponent(false); setNewCompName(""); }
            }}
          />
        </div>
      )}
      {components.length === 0 && !addingComponent && (
        <div style={{
          padding: "2px 12px 6px", fontSize: 11, color: "var(--color-text-dim)", lineHeight: 1.4,
        }}>
          Reusable block trees.
        </div>
      )}
      {components.map((c) => (
        <div
          key={c.id}
          className={`side-item ${editingComponentId === c.id ? "active" : ""}`}
          onClick={() => setEditingComponentId(c.id)}
        >
          <span style={{ display: "flex", alignItems: "center", gap: 6, overflow: "hidden" }}>
            <span style={{ display: "inline-flex", color: "var(--color-text-muted)" }}>
              <Icon name="Component" size={13} />
            </span>
            <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{c.name}</span>
          </span>
        </div>
      ))}

      <AlertDialog
        open={confirmingDeletePageId !== null}
        onOpenChange={(next) => !next && setConfirmingDeletePageId(null)}
        title="Delete this page?"
        description="The page and every block on it will be permanently removed. This can't be undone."
        confirmLabel="Delete page"
        destructive
        onConfirm={async () => {
          await confirmDeletePage();
          setConfirmingDeletePageId(null);
        }}
      />
    </aside>
  );
}

// ---------------------------------------------------------------------------
// Canvas
// ---------------------------------------------------------------------------

type Peer = { user_id: string; data: any };

function PageCanvas({
  pageId, componentId, site, breakpoint, activeEdit, selectedBlockId, onSelect, onPresence, peers, pushUndo,
}: {
  pageId: string | null;
  componentId: string | null;
  site: Site;
  breakpoint: Breakpoint;
  activeEdit: boolean;
  selectedBlockId: string | null;
  onSelect: (id: string | null) => void;
  onPresence: (x: number, y: number) => void;
  peers: Peer[];
  pushUndo: (op: UndoOp) => void;
}) {
  // Query both block scopes — pageId for page editing, componentId for
  // component master editing. Whichever is active renders the canvas.
  const { data: pageBlocks } = db.useQuery<Block>("Block", {
    where: pageId ? { pageId } : undefined,
  });
  const { data: componentBlocks } = db.useQuery<Block>("Block", {
    where: componentId ? { componentId, pageId: null } : undefined,
  });

  const blocks = pageId ? pageBlocks : componentBlocks;
  const topLevel = useMemo(
    () => [...(blocks ?? [])]
      .filter((b) => !b.parentId)
      .sort((a, b) => a.sort - b.sort),
    [blocks],
  );

  const canvasRef = useRef<HTMLDivElement>(null);
  const t = useToast();

  async function addAt(type: BlockType, afterSort?: number) {
    if (pageId) {
      await callFn("addBlock", { pageId, type, afterSort });
      return;
    }
    // Editing a component: create a block with componentId set, pageId null.
    // No backend helper for this case yet; fall back to a direct insert
    // via db.insert would bypass mutation gates, so we skip.
    t.toast("Coming soon", "Adding blocks to components isn't wired yet — edit the existing master block for now.");
  }

  function handleMouseMove(e: React.MouseEvent) {
    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect) return;
    // Cursor coords are relative to the canvas content box so peers see
    // the pointer in the same place even when their window is wider.
    onPresence(e.clientX - rect.left, e.clientY - rect.top);
  }

  return (
    <div
      ref={canvasRef}
      className={`canvas ${activeEdit ? "active-edit" : ""}`}
      data-bp={breakpoint}
      style={siteTokenStyle(site)}
      onClick={(e) => e.stopPropagation()}
      onMouseMove={handleMouseMove}
    >
      {/* Peer cursors + peer selection outlines layered over the canvas. */}
      <PeersOverlay peers={peers} blocks={topLevel} />

      {topLevel.length === 0 ? (
        <div className="canvas-empty">
          <div style={{ marginBottom: 14 }}>Empty — add your first block.</div>
          <BlockPicker onPick={(type) => addAt(type)} />
        </div>
      ) : (
        <>
          <InsertSlot pageId={pageId} onPick={(type) => addAt(type, topLevel[0].sort - 1)}
            dropTargetSort={topLevel[0].sort - 0.5} />
          {topLevel.map((b, i) => {
            const nextSort = topLevel[i + 1]?.sort;
            const slotSort = nextSort !== undefined ? (b.sort + nextSort) / 2 : b.sort + 1;
            return (
              <React.Fragment key={b.id}>
                <BlockWrap
                  block={b}
                  breakpoint={breakpoint}
                  selected={selectedBlockId === b.id}
                  onSelect={onSelect}
                  pushUndo={pushUndo}
                />
                <InsertSlot
                  pageId={pageId}
                  onPick={(type) => addAt(type, b.sort)}
                  dropTargetSort={slotSort}
                />
              </React.Fragment>
            );
          })}
        </>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Peer cursor + selection overlay
// ---------------------------------------------------------------------------

function PeersOverlay({ peers, blocks }: { peers: Peer[]; blocks: Block[] }) {
  return (
    <>
      {peers.map((p) => {
        const cursor = p.data?.cursor as { x: number; y: number } | undefined;
        const selectedId = p.data?.selectedBlockId as string | undefined;
        const color = p.data?.color ?? colorForUser(p.user_id);
        const name = p.data?.name ?? "Guest";
        return (
          <React.Fragment key={p.user_id}>
            {cursor && (
              <div className="cursor" style={{ transform: `translate(${cursor.x}px, ${cursor.y}px)` }}>
                <svg width="18" height="18" viewBox="0 0 18 18">
                  <path
                    d="M1 1 L1 14 L5 10.5 L7.5 16 L10 15 L7.5 9.5 L13 9.5 Z"
                    fill={color}
                    stroke="white"
                    strokeWidth="1"
                  />
                </svg>
                <div className="cursor-label" style={{ background: color }}>{name}</div>
              </div>
            )}
            {selectedId && blocks.find((b) => b.id === selectedId) && (
              <PeerSelectionOutline selectedId={selectedId} name={name} color={color} />
            )}
          </React.Fragment>
        );
      })}
    </>
  );
}

function PeerSelectionOutline({ selectedId, name, color }: {
  selectedId: string; name: string; color: string;
}) {
  const [rect, setRect] = useState<{ x: number; y: number; w: number; h: number } | null>(null);
  useEffect(() => {
    function compute() {
      const el = document.querySelector(`[data-block-id="${selectedId}"]`) as HTMLElement | null;
      const canvas = el?.closest(".canvas") as HTMLElement | null;
      if (!el || !canvas) { setRect(null); return; }
      const eR = el.getBoundingClientRect();
      const cR = canvas.getBoundingClientRect();
      setRect({ x: eR.left - cR.left, y: eR.top - cR.top, w: eR.width, h: eR.height });
    }
    compute();
    // Re-measure on window resize / scroll — the canvas reflows with
    // breakpoint changes and peer selection needs to track it.
    window.addEventListener("resize", compute);
    window.addEventListener("scroll", compute, true);
    const interval = setInterval(compute, 400);
    return () => {
      window.removeEventListener("resize", compute);
      window.removeEventListener("scroll", compute, true);
      clearInterval(interval);
    };
  }, [selectedId]);
  if (!rect) return null;
  return (
    <div
      className="peer-selection-outline"
      style={{
        left: rect.x - 3, top: rect.y - 3,
        width: rect.w + 6, height: rect.h + 6,
        border: `2px solid ${color}`,
      }}
    >
      <div className="tag" style={{ background: color }}>{name}</div>
    </div>
  );
}

function InsertSlot({ pageId, onPick, dropTargetSort }: {
  pageId: string | null;
  onPick: (type: BlockType) => void;
  dropTargetSort: number;
}) {
  const [open, setOpen] = useState(false);
  const [dropActive, setDropActive] = useState(false);
  return (
    <div
      className={`insert-slot ${dropActive ? "drop-active" : ""}`}
      style={{ position: "relative" }}
      onDragOver={(e) => {
        if (e.dataTransfer.types.includes("application/x-stage-block")) {
          e.preventDefault();
          setDropActive(true);
        }
      }}
      onDragLeave={() => setDropActive(false)}
      onDrop={async (e) => {
        setDropActive(false);
        const blockId = e.dataTransfer.getData("application/x-stage-block");
        if (!blockId || !pageId) return;
        try {
          await callFn("reorderBlock", { blockId, newSort: dropTargetSort });
        } catch {}
      }}
    >
      <button onClick={(e) => { e.stopPropagation(); setOpen((x) => !x); }}>+</button>
      {open && (
        <div onClick={(e) => e.stopPropagation()}>
          <BlockPicker onPick={(type) => { onPick(type); setOpen(false); }} />
        </div>
      )}
    </div>
  );
}

function BlockPicker({ onPick }: { onPick: (type: BlockType) => void }) {
  // Category-tabbed block picker. Default to "Hero" because it's the
  // most common starting point — users with an empty page want a hero,
  // not a divider. Basic primitives are always available as a tab but
  // aren't the front-door behavior.
  const [category, setCategory] = useState<string>("Hero");
  const visible = BLOCK_TYPES.filter((b) => b.category === category);
  return (
    <div className="picker">
      <div style={{
        gridColumn: "1 / -1",
        display: "flex", gap: 2,
        background: "var(--color-neutral)",
        padding: 2, borderRadius: "var(--rounded-sm)",
        marginBottom: 4,
      }}>
        {BLOCK_CATEGORIES.map((c) => (
          <button
            key={c}
            onClick={() => setCategory(c)}
            style={{
              flex: 1, padding: "5px 6px", borderRadius: "var(--rounded-xs)",
              fontSize: 11, fontWeight: 500, color: "var(--color-text-muted)",
              background: c === category ? "var(--color-surface)" : "transparent",
              boxShadow: c === category ? "var(--elevation-1)" : "none",
            }}
          >{c}</button>
        ))}
      </div>
      {visible.map((b) => (
        <button key={b.type} onClick={() => onPick(b.type)} title={b.label}>
          <span className="icon" style={{ display: "inline-flex" }}>
            <Icon name={b.icon} size={14} />
          </span>
          {b.label}
        </button>
      ))}
    </div>
  );
}

function BlockWrap({ block, breakpoint, selected, onSelect, pushUndo }: {
  block: Block;
  breakpoint: Breakpoint;
  selected: boolean;
  onSelect: (id: string | null) => void;
  pushUndo: (op: UndoOp) => void;
}) {
  const props = useMemo(
    () => resolveProps(block.propsJson, breakpoint),
    [block.propsJson, breakpoint],
  );
  const [dragging, setDragging] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [promoting, setPromoting] = useState(false);
  const t = useToast();

  async function move(direction: "up" | "down") {
    try { await callFn("moveBlock", { blockId: block.id, direction }); }
    catch {}
  }
  async function doDelete() {
    try {
      await callFn("deleteBlock", { blockId: block.id });
      t.success("Block deleted");
    } catch (e) {
      t.error("Couldn't delete", e instanceof Error ? e.message : String(e));
    }
  }
  async function duplicate() {
    try {
      await callFn("addBlock", {
        pageId: block.pageId ?? undefined,
        type: block.type,
        afterSort: block.sort,
        propsJson: block.propsJson,
      });
    } catch {}
  }
  async function doPromote(name: string) {
    try {
      await callFn("createComponent", {
        siteId: block.siteId, name, fromBlockId: block.id,
      });
      t.success("Component created", `"${name}" — edit the master from the sidebar.`);
    } catch (e) {
      t.error("Couldn't create component", e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div
      className={`block-wrap ${selected ? "selected" : ""} ${dragging ? "dragging" : ""}`}
      data-block-id={block.id}
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData("application/x-stage-block", block.id);
        e.dataTransfer.effectAllowed = "move";
        setDragging(true);
      }}
      onDragEnd={() => setDragging(false)}
      onClick={(e) => { e.stopPropagation(); onSelect(block.id); }}
    >
      <div className="block-toolbar" onClick={(e) => e.stopPropagation()}>
        <span className="block-handle" title="Drag"><Icon name="GripVertical" size={12} /></span>
        <button onClick={() => move("up")} title="Move up"><Icon name="ArrowUp" size={12} /></button>
        <button onClick={() => move("down")} title="Move down"><Icon name="ArrowDown" size={12} /></button>
        <button onClick={duplicate} title="Duplicate"><Icon name="Copy" size={12} /></button>
        <button onClick={() => setPromoting(true)} title="Make component"><Icon name="Component" size={12} /></button>
        <button className="danger" onClick={() => setConfirmingDelete(true)} title="Delete"><Icon name="X" size={12} /></button>
      </div>
      <AlertDialog
        open={confirmingDelete}
        onOpenChange={setConfirmingDelete}
        title="Delete this block?"
        description={`The "${block.type}" block will be removed.${
          block.type === "container" ? " Any nested children go with it." : ""
        }`}
        confirmLabel="Delete"
        destructive
        onConfirm={doDelete}
      />
      <PromptDialog
        open={promoting}
        onOpenChange={setPromoting}
        title="Promote to component"
        description="Reuse this block anywhere on the site. Edits to the master flow live to every instance."
        defaultValue="Section"
        placeholder="Component name"
        confirmLabel="Create component"
        onConfirm={doPromote}
      />
      {block.type === "component" && block.componentId ? (
        <ComponentInstance componentId={block.componentId} breakpoint={breakpoint} />
      ) : (
        <BlockRenderer type={block.type} props={props} />
      )}
    </div>
  );
}

// Renders a component instance by walking the master block tree.
function ComponentInstance({ componentId, breakpoint }: {
  componentId: string; breakpoint: Breakpoint;
}) {
  const { data: masterBlocks } = db.useQuery<Block>("Block", {
    where: { componentId, pageId: null },
  });
  const sorted = useMemo(
    () => [...(masterBlocks ?? [])]
      .filter((b) => !b.parentId)
      .sort((a, b) => a.sort - b.sort),
    [masterBlocks],
  );
  if (sorted.length === 0) {
    return <div style={{ color: "var(--color-text-dim)", fontSize: 12 }}>Empty component</div>;
  }
  return (
    <>
      {sorted.map((b) => {
        const p = resolveProps(b.propsJson, breakpoint);
        return <div key={b.id}><BlockRenderer type={b.type} props={p} /></div>;
      })}
    </>
  );
}

// ---------------------------------------------------------------------------
// Block renderers — shared by editor canvas + public preview
// ---------------------------------------------------------------------------

function BlockRenderer({ type, props }: { type: string; props: Record<string, unknown> }) {
  // Delegate section types to the rich renderers in `./sections`.
  // Primitives (heading/text/button/…) stay inline below because they're
  // small and the inspector UI leans on exact switch-case matching.
  if (SECTION_TYPES.has(type)) {
    return <>{renderSection(type, props)}</>;
  }
  const align = (props.align as string) || "left";
  switch (type) {
    case "heading": {
      const level = (props.level as number) || 2;
      const Tag = `h${Math.max(1, Math.min(3, level))}` as "h1" | "h2" | "h3";
      return (
        <Tag className={`rb-heading h${level} align-${align}`}>
          {(props.text as string) || "Heading"}
        </Tag>
      );
    }
    case "text":
      return (
        <p className={`rb-text align-${align}`}>
          {(props.text as string) || "…"}
        </p>
      );
    case "button": {
      const variant = ((props.variant as string) || "primary");
      return (
        <div className={`align-${align}`}>
          <a
            className={`rb-button ${variant}`}
            href={(props.href as string) || "#"}
            onClick={(e) => e.preventDefault()}
          >
            {(props.text as string) || "Button"}
          </a>
        </div>
      );
    }
    case "image":
      return (
        <div className="rb-image">
          <img
            src={(props.src as string) || ""}
            alt={(props.alt as string) || ""}
            style={{ objectFit: (props.fit as string) || "cover" }}
          />
        </div>
      );
    case "divider":
      return (
        <div style={{ padding: `${(props.margin as number) ?? 16}px 0` }}>
          <div className="rb-divider" />
        </div>
      );
    case "container":
      return (
        <div
          className="rb-container"
          style={{
            padding: (props.padding as number) ?? 24,
            gap: (props.gap as number) ?? 12,
            background: (props.bg as string) || "#ffffff",
          }}
        >
          <div style={{ color: "var(--text-dim)", fontSize: 12, textAlign: "center" }}>
            Container — child blocks coming in a future version
          </div>
        </div>
      );
    default:
      return <div>Unknown block: {type}</div>;
  }
}

// ---------------------------------------------------------------------------
// Inspector
// ---------------------------------------------------------------------------

function Inspector({ site, blockId, breakpoint, tab, setTab, pushUndo }: {
  site: Site;
  blockId: string | null;
  breakpoint: Breakpoint;
  tab: "block" | "site";
  setTab: (t: "block" | "site") => void;
  pushUndo: (op: UndoOp) => void;
}) {
  return (
    <aside className="inspector">
      <div className="insp-tabs">
        <button className={tab === "block" ? "active" : ""} onClick={() => setTab("block")}>Block</button>
        <button className={tab === "site" ? "active" : ""} onClick={() => setTab("site")}>Site</button>
      </div>
      {tab === "block" ? (
        blockId ? (
          <BlockInspector
            blockId={blockId}
            breakpoint={breakpoint}
            pushUndo={pushUndo}
          />
        ) : (
          <div className="insp-section" style={{ color: "var(--color-text-dim)", fontSize: 12.5 }}>
            Select a block on the canvas to edit it.
          </div>
        )
      ) : (
        <SiteInspector site={site} />
      )}
    </aside>
  );
}

function BlockInspector({ blockId, breakpoint, pushUndo }: {
  blockId: string; breakpoint: Breakpoint; pushUndo: (op: UndoOp) => void;
}) {
  const { data: block } = db.useQueryOne<Block>("Block", blockId);
  const props = useMemo(
    () => block ? resolveProps(block.propsJson, breakpoint) : {},
    [block?.propsJson, breakpoint],
  );

  // Detect whether the active breakpoint has overrides defined (vs
  // inheriting from desktop). Renders a small indicator so users know
  // whether they're editing a shared or overridden value.
  const hasOverride = useMemo(() => {
    if (!block) return false;
    const raw = safeJson(block.propsJson) as Record<string, unknown> ?? {};
    const keyed = "desktop" in raw || "tablet" in raw || "phone" in raw;
    if (!keyed) return breakpoint === "desktop";
    return Object.keys(raw[breakpoint] as Record<string, unknown> ?? {}).length > 0;
  }, [block?.propsJson, breakpoint]);

  if (!block) return null;

  async function patch(changes: Record<string, unknown>) {
    pushUndo({ kind: "updateBlock", blockId, prevPropsJson: block!.propsJson });
    const nextJson = patchBreakpointProps(block!.propsJson, breakpoint, changes);
    await callFn("updateBlock", { blockId, propsJson: nextJson });
  }

  async function clearBreakpointOverride() {
    if (breakpoint === "desktop") return;
    pushUndo({ kind: "updateBlock", blockId, prevPropsJson: block!.propsJson });
    const raw = (safeJson(block!.propsJson) as Record<string, unknown>) ?? {};
    if ("desktop" in raw || "tablet" in raw || "phone" in raw) {
      const next = { ...raw };
      delete next[breakpoint];
      await callFn("updateBlock", { blockId, propsJson: JSON.stringify(next) });
    }
  }

  return (
    <div>
      {/* Breakpoint override indicator. Clicking an inactive tab flips
          which breakpoint the inspector writes to — users can edit
          tablet/phone overrides while still seeing desktop context on
          the canvas by switching breakpoints separately. */}
      {breakpoint !== "desktop" && (
        <div className="insp-section" style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
          <span style={{ fontSize: 12, color: hasOverride ? "var(--color-tertiary)" : "var(--color-text-dim)" }}>
            {hasOverride ? `Editing ${breakpoint} override` : `Inherits from desktop`}
          </span>
          {hasOverride && (
            <button className="btn ghost" style={{ padding: "4px 8px", fontSize: 11 }}
              onClick={clearBreakpointOverride}
              title="Reset this breakpoint to inherit from desktop"
            >Reset</button>
          )}
        </div>
      )}
      <div className="insp-section">
        <div className="insp-title">{block.type}</div>
        {block.type === "heading" && (
          <>
            <div className="insp-row">
              <label>Level</label>
              <select className="insp-select" value={(props.level as number) ?? 2}
                onChange={(e) => patch({ level: Number(e.target.value) })}>
                <option value={1}>H1</option>
                <option value={2}>H2</option>
                <option value={3}>H3</option>
              </select>
            </div>
            <TextField label="Text" value={(props.text as string) ?? ""} onCommit={(v) => patch({ text: v })} textarea />
            <AlignField value={(props.align as string) ?? "left"} onChange={(v) => patch({ align: v })} />
          </>
        )}
        {block.type === "text" && (
          <>
            <TextField label="Body" value={(props.text as string) ?? ""} onCommit={(v) => patch({ text: v })} textarea />
            <AlignField value={(props.align as string) ?? "left"} onChange={(v) => patch({ align: v })} />
          </>
        )}
        {block.type === "button" && (
          <>
            <TextField label="Label" value={(props.text as string) ?? ""} onCommit={(v) => patch({ text: v })} />
            <TextField label="Link" value={(props.href as string) ?? ""} onCommit={(v) => patch({ href: v })} />
            <div className="insp-row">
              <label>Style</label>
              <select className="insp-select" value={(props.variant as string) ?? "primary"}
                onChange={(e) => patch({ variant: e.target.value })}>
                <option value="primary">Primary</option>
                <option value="ghost">Ghost</option>
              </select>
            </div>
            <AlignField value={(props.align as string) ?? "left"} onChange={(v) => patch({ align: v })} />
          </>
        )}
        {block.type === "image" && (
          <>
            <TextField label="Source" value={(props.src as string) ?? ""} onCommit={(v) => patch({ src: v })} />
            <TextField label="Alt text" value={(props.alt as string) ?? ""} onCommit={(v) => patch({ alt: v })} />
            <div className="insp-row">
              <label>Fit</label>
              <select className="insp-select" value={(props.fit as string) ?? "cover"}
                onChange={(e) => patch({ fit: e.target.value })}>
                <option value="cover">Cover</option>
                <option value="contain">Contain</option>
              </select>
            </div>
          </>
        )}
        {block.type === "divider" && (
          <div className="insp-row">
            <label>Spacing</label>
            <input className="insp-input" type="number" value={(props.margin as number) ?? 16}
              onChange={(e) => patch({ margin: Number(e.target.value) })} />
          </div>
        )}
        {block.type === "container" && (
          <>
            <div className="insp-row">
              <label>Padding</label>
              <input className="insp-input" type="number" value={(props.padding as number) ?? 24}
                onChange={(e) => patch({ padding: Number(e.target.value) })} />
            </div>
            <ColorField label="Background" value={(props.bg as string) ?? "#ffffff"} onChange={(v) => patch({ bg: v })} />
          </>
        )}

        {/* ---- Section inspectors — top-level text props only; repeating
             items (features, tiers, FAQ) are edited as JSON in a
             textarea for now. A real editor would gain +/– row controls,
             but this ships the section library end-to-end. ---- */}
        {(block.type === "hero-centered" || block.type === "hero-split") && (
          <>
            <TextField label="Eyebrow" value={(props.eyebrow as string) ?? ""} onCommit={(v) => patch({ eyebrow: v })} />
            <TextField label="Title" value={(props.title as string) ?? ""} onCommit={(v) => patch({ title: v })} textarea />
            <TextField label="Subtitle" value={(props.subtitle as string) ?? ""} onCommit={(v) => patch({ subtitle: v })} textarea />
            <TextField label="CTA label" value={((props.primaryCta as any)?.text as string) ?? ""}
              onCommit={(v) => patch({ primaryCta: { ...((props.primaryCta as any) ?? {}), text: v } })} />
            <TextField label="CTA link" value={((props.primaryCta as any)?.href as string) ?? ""}
              onCommit={(v) => patch({ primaryCta: { ...((props.primaryCta as any) ?? {}), href: v } })} />
            {block.type === "hero-centered" && (
              <TextField label="2nd CTA" value={((props.secondaryCta as any)?.text as string) ?? ""}
                onCommit={(v) => patch({ secondaryCta: { ...((props.secondaryCta as any) ?? {}), text: v } })} />
            )}
            {block.type === "hero-split" && (
              <TextField label="Image URL" value={(props.image as string) ?? ""} onCommit={(v) => patch({ image: v })} />
            )}
          </>
        )}
        {(block.type === "feature-grid" || block.type === "pricing" || block.type === "faq") && (
          <>
            <TextField label="Eyebrow" value={(props.eyebrow as string) ?? ""} onCommit={(v) => patch({ eyebrow: v })} />
            <TextField label="Title" value={(props.title as string) ?? ""} onCommit={(v) => patch({ title: v })} textarea />
            <div className="insp-row" style={{ alignItems: "flex-start" }}>
              <label>Items JSON</label>
              <textarea className="insp-textarea"
                value={JSON.stringify(
                  block.type === "pricing" ? props.tiers : props.items, null, 2,
                )}
                onChange={(e) => {
                  try {
                    const parsed = JSON.parse(e.target.value);
                    patch(block.type === "pricing" ? { tiers: parsed } : { items: parsed });
                  } catch {}
                }}
                rows={6}
                style={{ fontFamily: "JetBrains Mono, ui-monospace, monospace", fontSize: 11 }}
              />
            </div>
          </>
        )}
        {block.type === "stats" && (
          <div className="insp-row" style={{ alignItems: "flex-start" }}>
            <label>Stats JSON</label>
            <textarea className="insp-textarea"
              value={JSON.stringify(props.items, null, 2)}
              onChange={(e) => {
                try { patch({ items: JSON.parse(e.target.value) }); } catch {}
              }}
              rows={6}
              style={{ fontFamily: "JetBrains Mono, ui-monospace, monospace", fontSize: 11 }}
            />
          </div>
        )}
        {block.type === "logo-cloud" && (
          <>
            <TextField label="Title" value={(props.title as string) ?? ""} onCommit={(v) => patch({ title: v })} />
            <TextField label="Logos" value={(props.logos as string[] ?? []).join(", ")}
              onCommit={(v) => patch({ logos: v.split(",").map((s) => s.trim()).filter(Boolean) })} textarea />
          </>
        )}
        {block.type === "testimonial" && (
          <>
            <TextField label="Quote" value={(props.quote as string) ?? ""} onCommit={(v) => patch({ quote: v })} textarea />
            <TextField label="Author" value={(props.author as string) ?? ""} onCommit={(v) => patch({ author: v })} />
            <TextField label="Role" value={(props.role as string) ?? ""} onCommit={(v) => patch({ role: v })} />
            <TextField label="Avatar" value={(props.avatar as string) ?? ""} onCommit={(v) => patch({ avatar: v })} />
          </>
        )}
        {block.type === "cta-banner" && (
          <>
            <TextField label="Title" value={(props.title as string) ?? ""} onCommit={(v) => patch({ title: v })} />
            <TextField label="Subtitle" value={(props.subtitle as string) ?? ""} onCommit={(v) => patch({ subtitle: v })} textarea />
            <TextField label="CTA label" value={((props.primaryCta as any)?.text as string) ?? ""}
              onCommit={(v) => patch({ primaryCta: { ...((props.primaryCta as any) ?? {}), text: v } })} />
          </>
        )}
        {block.type === "footer" && (
          <>
            <TextField label="Tagline" value={(props.tagline as string) ?? ""} onCommit={(v) => patch({ tagline: v })} />
            <TextField label="Copyright" value={(props.copyright as string) ?? ""} onCommit={(v) => patch({ copyright: v })} />
          </>
        )}
      </div>
    </div>
  );
}

function SiteInspector({ site }: { site: Site }) {
  const tokens = resolveSiteTokens(site);
  const t = useToast();
  const [importOpen, setImportOpen] = useState(false);

  async function patchSite(changes: Record<string, unknown>) {
    await callFn("updateSite", { siteId: site.id, ...changes });
  }

  async function patchToken(path: string[], value: string) {
    // Merge a single color/type path into the site's tokensJson. Keeps
    // the schema flexible — new token keys land without migrations.
    const existing = (safeJson(site.tokensJson ?? "{}") as Record<string, any>) ?? {};
    const next = JSON.parse(JSON.stringify(existing));
    let cur = next;
    for (let i = 0; i < path.length - 1; i++) {
      cur[path[i]] = cur[path[i]] ?? {};
      cur = cur[path[i]];
    }
    cur[path[path.length - 1]] = value;
    await callFn("updateSiteTokens", {
      siteId: site.id,
      tokensJson: JSON.stringify(next),
    });
  }

  // Apply a complete preset's tokens at once — avoids N serial
  // patchToken round-trips and keeps the preset atomic.
  async function applyPreset(preset: PresetTokens) {
    const existing = (safeJson(site.tokensJson ?? "{}") as Record<string, any>) ?? {};
    const next = {
      ...existing,
      colors: { ...(existing.colors ?? {}), ...preset.colors },
      ...(preset.radius ? { radius: preset.radius } : {}),
      ...(preset.style ? { style: preset.style } : {}),
      ...(preset.rounded ? { rounded: { ...(existing.rounded ?? {}), ...preset.rounded } } : {}),
    };
    await callFn("updateSiteTokens", {
      siteId: site.id,
      tokensJson: JSON.stringify(next),
    });
  }

  async function shuffle() {
    const pool = SHADCN_PRESETS;
    const pick = pool[Math.floor(Math.random() * pool.length)];
    await applyPreset(pick.tokens);
    t.success("Shuffled", `Applied "${pick.label}" preset.`);
  }

  return (
    <>
      <div className="insp-section">
        <div className="insp-title">Site</div>
        <TextField label="Name" value={site.name} onCommit={(v) => patchSite({ name: v })} />
        <TextField label="Icon" value={site.faviconEmoji} onCommit={(v) => patchSite({ faviconEmoji: v })} />
        <div style={{
          marginTop: 12, padding: 10, background: "var(--color-surface-raised)",
          borderRadius: "var(--rounded-sm)", fontSize: 12, color: "var(--color-text-muted)",
        }}>
          Public URL: <code style={{ fontFamily: "JetBrains Mono, ui-monospace, monospace" }}>/p/{site.slug}</code>
          {!site.publishedAt && <div style={{ color: "var(--color-text-dim)", marginTop: 4 }}>
            (publish to make this live)
          </div>}
        </div>
      </div>

      <div className="insp-section">
        <div className="insp-title">Colors</div>
        {Object.entries(tokens.colors).map(([name, value]) => (
          <div key={name} className="insp-row">
            <label>{name}</label>
            <div className="insp-color">
              <input type="color" value={value as string}
                onChange={(e) => patchToken(["colors", name], e.target.value)} />
              <input type="text" value={value as string}
                onChange={(e) => patchToken(["colors", name], e.target.value)} />
            </div>
          </div>
        ))}
      </div>

      <div className="insp-section">
        <div className="insp-title">Typography</div>
        <div className="insp-row">
          <label>Heading</label>
          <select className="insp-select" value={(tokens.typography.heading.fontFamily as string) ?? "Inter"}
            onChange={(e) => patchToken(["typography", "heading", "fontFamily"], e.target.value)}>
            <option value="Inter">Inter</option>
            <option value="'Playfair Display', serif">Playfair</option>
            <option value="'JetBrains Mono', monospace">JetBrains Mono</option>
            <option value="'Instrument Serif', serif">Instrument Serif</option>
            <option value="system-ui">System UI</option>
          </select>
        </div>
        <div className="insp-row">
          <label>Body</label>
          <select className="insp-select" value={(tokens.typography.body.fontFamily as string) ?? "Inter"}
            onChange={(e) => patchToken(["typography", "body", "fontFamily"], e.target.value)}>
            <option value="Inter">Inter</option>
            <option value="Georgia, serif">Georgia</option>
            <option value="'JetBrains Mono', monospace">JetBrains Mono</option>
            <option value="system-ui">System UI</option>
          </select>
        </div>
      </div>

      {/* shadcn/ui create-page parity — style, icon library, radius,
          menu knobs live on Site.tokensJson so they survive reloads
          without a schema change. */}
      <div className="insp-section">
        <div className="insp-title">Style</div>
        <div className="insp-row">
          <label>Preset</label>
          <select className="insp-select" value=""
            onChange={(e) => {
              const preset = SHADCN_PRESETS.find((p) => p.id === e.target.value);
              if (preset) {
                void applyPreset(preset.tokens);
                t.success("Preset applied", preset.label);
              }
            }}>
            <option value="">Pick a preset…</option>
            {SHADCN_PRESETS.map((p) => (
              <option key={p.id} value={p.id}>{p.label}</option>
            ))}
          </select>
        </div>
        <div className="insp-row" style={{ gap: 6 }}>
          <label />
          <div style={{ display: "flex", gap: 6, flex: 1 }}>
            <button className="btn ghost" style={{ flex: 1 }} onClick={() => setImportOpen(true)}>
              <Icon name="Download" size={13} /> Import
            </button>
            <button className="btn ghost" style={{ flex: 1 }} onClick={shuffle}>
              <Icon name="Shuffle" size={13} /> Shuffle
            </button>
          </div>
        </div>
        <div className="insp-row">
          <label>Icon library</label>
          <select className="insp-select" value={tokens.iconLibrary ?? "lucide"}
            onChange={(e) => patchToken(["iconLibrary"], e.target.value)}>
            {ICON_LIBRARIES.map((l) => (
              <option key={l.id} value={l.id}>{l.label}</option>
            ))}
          </select>
        </div>
        <div className="insp-row">
          <label>Radius</label>
          <div className="insp-seg">
            {(["none", "sm", "md", "lg", "xl"] as const).map((r) => (
              <button key={r} className={r === (tokens.radius ?? "md") ? "active" : ""}
                onClick={() => patchToken(["radius"], r)}
                style={{ fontSize: 10, letterSpacing: "0.04em", textTransform: "uppercase" }}>
                {r === "none" ? "0" : r}
              </button>
            ))}
          </div>
        </div>
        <div className="insp-row">
          <label>Menu</label>
          <select className="insp-select" value={tokens.menuStyle ?? "solid"}
            onChange={(e) => patchToken(["menuStyle"], e.target.value)}>
            <option value="solid">Default / Solid</option>
            <option value="outline">Outline</option>
            <option value="ghost">Ghost</option>
          </select>
        </div>
      </div>

      <ImportPresetDialog
        open={importOpen}
        onOpenChange={setImportOpen}
        onImport={async (tokens) => {
          await applyPreset(tokens);
          t.success("Preset imported", "Colors and radius applied to this site.");
        }}
      />
    </>
  );
}

function ImportPresetDialog({
  open, onOpenChange, onImport,
}: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
  onImport: (tokens: PresetTokens) => void | Promise<void>;
}) {
  const [text, setText] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open) { setText(""); setErr(null); }
  }, [open]);

  async function handleImport() {
    setErr(null);
    const parsed = parseShadcnPreset(text);
    if (!parsed) {
      setErr("Couldn't parse that — paste a shadcn theme JSON or a block of CSS variables.");
      return;
    }
    setBusy(true);
    try {
      await onImport(parsed);
      onOpenChange(false);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => !busy && onOpenChange(next)}
      title="Import preset"
      description="Paste a shadcn theme (JSON from ui.shadcn.com/create, or a CSS block with --primary, --background, --radius, etc.) and we'll apply the colors + radius."
      size="lg"
      footer={
        <>
          <button className="btn ghost" onClick={() => onOpenChange(false)} disabled={busy}>Cancel</button>
          <button className="btn primary" onClick={handleImport} disabled={busy || !text.trim()}>
            {busy ? "…" : "Import preset"}
          </button>
        </>
      }
    >
      <textarea
        className="insp-textarea"
        style={{ minHeight: 220, fontFamily: "JetBrains Mono, ui-monospace, monospace", fontSize: 12 }}
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder={`:root {\n  --background: 0 0% 100%;\n  --foreground: 240 10% 3.9%;\n  --primary: 240 5.9% 10%;\n  --radius: 0.5rem;\n}`}
      />
      {err && <div style={{ color: "var(--color-danger, #E5484D)", fontSize: 12, marginTop: 8 }}>{err}</div>}
    </Dialog>
  );
}

function TextField({ label, value, onCommit, textarea }: {
  label: string; value: string; onCommit: (v: string) => void; textarea?: boolean;
}) {
  const [v, setV] = useState(value);
  const committedRef = useRef(value);
  useEffect(() => {
    if (value !== committedRef.current) {
      setV(value);
      committedRef.current = value;
    }
  }, [value]);
  function commit() {
    if (v !== committedRef.current) {
      committedRef.current = v;
      onCommit(v);
    }
  }
  return (
    <div className="insp-row" style={{ alignItems: textarea ? "flex-start" : "center" }}>
      <label>{label}</label>
      {textarea ? (
        <textarea className="insp-textarea" value={v}
          onChange={(e) => setV(e.target.value)}
          onBlur={commit} rows={3} />
      ) : (
        <input className="insp-input" value={v}
          onChange={(e) => setV(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => { if (e.key === "Enter") (e.target as HTMLInputElement).blur(); }} />
      )}
    </div>
  );
}

function ColorField({ label, value, onChange }: {
  label: string; value: string; onChange: (v: string) => void;
}) {
  return (
    <div className="insp-row">
      <label>{label}</label>
      <div className="insp-color">
        <input type="color" value={value} onChange={(e) => onChange(e.target.value)} />
        <input type="text" value={value} onChange={(e) => onChange(e.target.value)} />
      </div>
    </div>
  );
}

function AlignField({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const iconFor: Record<string, string> = {
    left: "AlignLeft",
    center: "AlignCenter",
    right: "AlignRight",
  };
  return (
    <div className="insp-row">
      <label>Align</label>
      <div className="insp-seg">
        {["left", "center", "right"].map((a) => (
          <button key={a} className={a === value ? "active" : ""} onClick={() => onChange(a)}>
            <Icon name={iconFor[a]} size={14} />
          </button>
        ))}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Public preview — /p/:slug
// ---------------------------------------------------------------------------

function PublicPreview({ siteSlug, pageSlug }: { siteSlug: string; pageSlug: string }) {
  // The public preview runs the sync engine under the reader's session
  // (if any) and reads rows from the local replica. That sidesteps the
  // `/api/entities/X` list-read gate, which denies reads without a row
  // in scope. For this demo, the preview assumes the reader is a member
  // of the owning workspace — same tab, same token — which matches how
  // most agency/portfolio sites are shared in early-stage tools. A
  // production build would add a dedicated `renderPublicSite` action
  // running with elevated auth that returns only sites where
  // `publishedAt != null`.
  const { data: sites, loading: sitesLoading } = db.useQuery<Site>("Site");
  const site = useMemo(
    () => (sites ?? []).find((s) => s.slug === siteSlug) ?? null,
    [sites, siteSlug],
  );
  const { data: pages } = db.useQuery<Page>("Page", {
    where: site ? { siteId: site.id } : undefined,
  });
  const wantSlug = pageSlug ? `/${pageSlug}` : "/";
  const page = useMemo(() => {
    const list = pages ?? [];
    return list.find((p) => p.slug === wantSlug) ?? list.find((p) => p.slug === "/") ?? list[0] ?? null;
  }, [pages, wantSlug]);
  const { data: blocks } = db.useQuery<Block>("Block", {
    where: page ? { pageId: page.id } : undefined,
  });
  const sortedBlocks = useMemo(
    () => [...(blocks ?? [])]
      .filter((b) => !b.parentId)
      .sort((a, b) => a.sort - b.sort),
    [blocks],
  );

  if (!site) {
    // `useQuery` starts with an empty replica while sync pulls. Show a
    // neutral "loading" state until the first tick; only claim "not
    // found" once sites have arrived and this slug still isn't present.
    if (sitesLoading || !sites) {
      return <div className="preview-shell"><div className="preview-banner">Loading…</div></div>;
    }
    return (
      <div className="preview-shell">
        <div className="preview-banner">
          Site not found. <a href="/"><Icon name="ChevronLeft" size={12} /> Back to Stage</a>
        </div>
      </div>
    );
  }
  if (!site.publishedAt) {
    return (
      <div className="preview-shell">
        <div className="preview-banner">
          This site is a draft — publish it in the editor to share.
          <a href="/">Open editor <Icon name="ArrowRight" size={12} /></a>
        </div>
      </div>
    );
  }
  if (!page) return null;

  return (
    <div className="preview-shell" style={siteTokenStyle(site)}>
      <div className="preview-banner">
        Preview of <strong style={{ marginLeft: 4 }}>{site.name}</strong>
        <span style={{ opacity: 0.6 }}>·</span>
        <a href="/">Edit in Stage <Icon name="ArrowRight" size={12} /></a>
      </div>
      <div style={{ maxWidth: 820, margin: "0 auto", padding: "56px 48px" }}>
        {sortedBlocks.map((b) => {
          // Detect viewport breakpoint so responsive overrides apply in
          // the public preview too. Checked once on each render; a real
          // build would watch window resize, but the demo's preview is
          // usually opened full-width.
          const vw = typeof window === "undefined" ? 1024 : window.innerWidth;
          const bp: Breakpoint = vw < 560 ? "phone" : vw < 900 ? "tablet" : "desktop";
          const props = resolveProps(b.propsJson, bp);
          return <div key={b.id} style={{ padding: "8px 0" }}>
            {b.type === "component" && b.componentId
              ? <ComponentInstance componentId={b.componentId} breakpoint={bp} />
              : <BlockRenderer type={b.type} props={props} />}
          </div>;
        })}
      </div>
    </div>
  );
}
