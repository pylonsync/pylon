"use client";

import React, { useEffect, useMemo, useRef, useState } from "react";
import { callFn, db, useRouter } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import { FolderKanban, SquareCheck } from "lucide-react";
import { AppShell } from "@/components/app-shell";
import { CommandPalette } from "@/components/command-palette";
import type { SearchItem } from "@/lib/search";
import {
  DEFAULT_PROJECT_TAB,
  minutesByTask,
  projectTabCounts,
  type Project,
  type Task,
  type TimeEntry,
} from "@/lib/work";

export interface ClientRow {
  id: string;
  name: string;
  email?: string | null;
  createdAt?: string | null;
}
export type ProjectRow = Project;
export type TaskRow = Task;
export type TimeEntryRow = TimeEntry;
export interface UserRow {
  id: string;
  email: string;
  displayName?: string | null;
}

export interface WorkspaceData {
  clients: ClientRow[];
  projects: ProjectRow[];
  tasks: TaskRow[];
  entries: TimeEntryRow[];
  /** Minutes per task, computed once per render rather than per card. */
  taskMinutes: Map<string, number>;
  clientName: (id: string | null | undefined) => string | null;
  memberName: (id: string | null | undefined) => string | null;
  members: UserRow[];
  /** Project count per list tab; see PROJECT_TABS. */
  projectCounts: Record<string, number>;
  /**
   * The number beside each nav link, keyed by href. Page headers and tabs
   * read the same counts, so the sidebar and the page can't disagree.
   */
  navCounts: Record<string, number>;
  loading: boolean;
}

/**
 * The app shell, and the ONLY place that touches `db`.
 *
 * Every view below receives plain data through the render prop and reports
 * changes through callbacks, which is what lets the components in `components/`
 * render from fixtures in a test with nothing mocked.
 *
 * The queries are live: drag a task and it moves on every teammate\'s board, so
 * a standup doesn\'t start with reconciling two views of the work.
 */
export function Workspace({
  pathname,
  children,
}: {
  pathname: string;
  children: (data: WorkspaceData) => React.ReactNode;
}) {
  const router = useRouter();
  // The signed-in identity comes from the CLIENT session, not from an SSR
  // prop: the session cookie is SameSite=Lax and browsers withhold it inside
  // the builder's cross-site preview iframe, so a server-resolved email would
  // be empty exactly where this app is most often viewed.
  const { signOut, userId } = useAuth();
  const { data: projects, loading } = db.useQuery<ProjectRow>("Project");
  const { data: clients } = db.useQuery<ClientRow>("Client");
  const { data: tasks } = db.useQuery<TaskRow>("Task");
  const { data: entries } = db.useQuery<TimeEntryRow>("TimeEntry");
  const { data: users } = db.useQuery<UserRow>("User");

  // Whoever is signed in, named from the synced User row rather than an
  // SSR prop — see the note on useAuth above.
  const me = (users ?? []).find((user) => user.id === userId);

  const [paletteOpen, setPaletteOpen] = useState(false);
  const seeded = useRef(false);

  const projectList = projects ?? [];
  const clientList = clients ?? [];
  const taskList = tasks ?? [];
  const entryList = entries ?? [];

  useEffect(() => {
    if (loading || seeded.current) return;
    if (projectList.length > 0) {
      seeded.current = true;
      return;
    }
    seeded.current = true;
    callFn("seedWorkspace", {}).catch(() => {
      seeded.current = false;
    });
  }, [loading, projectList.length]);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      const typing =
        !!target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.isContentEditable);
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen((open) => !open);
      } else if (event.key === "/" && !typing) {
        event.preventDefault();
        setPaletteOpen(true);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // One pass over the ledger for the whole render — per-card lookups would be
  // O(tasks x entries).
  const taskMinutes = useMemo(() => minutesByTask(entryList), [entryList]);
  const clientById = useMemo(
    () => new Map(clientList.map((c) => [c.id, c] as const)),
    [clientList],
  );
  const userById = useMemo(
    () => new Map((users ?? []).map((u) => [u.id, u] as const)),
    [users],
  );

  const searchIndex = useMemo<SearchItem[]>(
    () => [
      ...projectList.map((project) => ({
        id: `project:${project.id}`,
        type: "project",
        title: project.name,
        subtitle: clientById.get(project.clientId ?? "")?.name,
        href: `/projects/${project.id}`,
      })),
      ...taskList.map((task) => ({
        id: `task:${task.id}`,
        type: "task",
        title: task.title,
        href: `/projects/${task.projectId}`,
      })),
    ],
    [projectList, taskList, clientById],
  );

  const projectCounts = projectTabCounts(projectList);
  const navCounts = {
    "/": projectCounts[DEFAULT_PROJECT_TAB],
    "/clients": clientList.length,
  };

  const data: WorkspaceData = {
    clients: clientList,
    projects: projectList,
    tasks: taskList,
    entries: entryList,
    taskMinutes,
    clientName: (id) => clientById.get(id ?? "")?.name ?? null,
    memberName: (id) => {
      const user = userById.get(id ?? "");
      return user?.displayName || user?.email || null;
    },
    members: users ?? [],
    projectCounts,
    navCounts,
    loading,
  };

  return (
    <AppShell
      sidebar={{
        user: { name: me?.displayName ?? "", email: me?.email ?? "" },
        pathname,
        counts: navCounts,
        onOpenCommand: () => setPaletteOpen(true),
        onSignOut: () => void signOut(),
      }}
    >
      {children(data)}

      <CommandPalette
        open={paletteOpen}
        items={searchIndex}
        actions={[
          {
            id: "new-project",
            label: "New project",
            run: () => router.push("/?new=project"),
          },
          {
            id: "go-clients",
            label: "Go to Clients",
            run: () => router.push("/clients"),
          },
        ]}
        placeholder="Search projects and tasks…"
        icons={{ project: <FolderKanban />, task: <SquareCheck /> }}
        onClose={() => setPaletteOpen(false)}
        onSelect={(item) => router.push(item.href)}
      />
    </AppShell>
  );
}
