"use client";

import React, { useEffect, useMemo, useRef, useState } from "react";
import { callFn, db, useRouter } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import { Sidebar } from "@/components/sidebar";
import { CommandPalette } from "@/components/command-palette";
import type { SearchItem } from "@/lib/search";
import { minutesByTask, type Project, type Task, type TimeEntry } from "@/lib/work";

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
  email,
  pathname,
  children,
}: {
  email: string;
  pathname: string;
  children: (data: WorkspaceData) => React.ReactNode;
}) {
  const router = useRouter();
  const { signOut } = useAuth();
  const { data: projects, loading } = db.useQuery<ProjectRow>("Project");
  const { data: clients } = db.useQuery<ClientRow>("Client");
  const { data: tasks } = db.useQuery<TaskRow>("Task");
  const { data: entries } = db.useQuery<TimeEntryRow>("TimeEntry");
  const { data: users } = db.useQuery<UserRow>("User");

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
        type: "company" as const,
        title: project.name,
        subtitle: clientById.get(project.clientId ?? "")?.name,
        href: `/projects/${project.id}`,
      })),
      ...taskList.map((task) => ({
        id: `task:${task.id}`,
        type: "deal" as const,
        title: task.title,
        href: `/projects/${task.projectId}`,
      })),
    ],
    [projectList, taskList, clientById],
  );

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
    loading,
  };

  return (
    <div className="flex h-screen overflow-hidden">
      <Sidebar
        workspace="Projects"
        email={email}
        pathname={pathname}
        counts={{
          "/": projectList.filter((p) => p.status === "active").length,
          "/clients": clientList.length,
        }}
        onOpenCommand={() => setPaletteOpen(true)}
        onSignOut={async () => {
          await signOut();
          window.location.assign("/login");
        }}
      />
      <main className="flex min-w-0 flex-1 flex-col">{children(data)}</main>

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
        onClose={() => setPaletteOpen(false)}
        onSelect={(item) => router.push(item.href)}
      />
    </div>
  );
}
