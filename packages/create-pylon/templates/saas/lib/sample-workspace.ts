import type { TaskPriority, TaskStatus } from "./tasks";
import { isoDay } from "./tasks";

// The sample projects `seedWorkspace` adds to a new workspace, so the first
// dashboard a person sees has real rows to click through. Replace them with
// your own domain's data, or delete this file and the "Start with sample
// projects" button in app/onboarding/onboarding-client.tsx.
//
// Two projects are active and one is archived, so a free workspace still has
// room for one project of its own under FREE_PROJECT_LIMIT (lib/plans.ts).

interface SampleTask {
  title: string;
  status: TaskStatus;
  priority: TaskPriority;
  /** Days from today. Negative is in the past. */
  due: number;
  /** Index into the member list, taken modulo its length; null = unassigned. */
  assignee: number | null;
}

interface SampleProject {
  name: string;
  description: string;
  status: "active" | "archived";
  tasks: SampleTask[];
}

export const SAMPLE_PROJECTS: SampleProject[] = [
  {
    name: "Website relaunch",
    description: "New marketing site, pricing page, and docs, live by the end of the month.",
    status: "active",
    tasks: [
      { title: "Fix the mobile nav overlap on Safari", status: "in_progress", priority: "urgent", due: 0, assignee: 0 },
      { title: "Build the pricing page", status: "in_review", priority: "high", due: 1, assignee: 1 },
      { title: "Move the docs to the new layout", status: "in_progress", priority: "medium", due: 4, assignee: 2 },
      { title: "Set up redirects from the old URLs", status: "todo", priority: "high", due: 6, assignee: 0 },
      { title: "Record the product walkthrough video", status: "todo", priority: "medium", due: 9, assignee: 3 },
      { title: "Update the Open Graph images", status: "todo", priority: "low", due: 12, assignee: null },
      { title: "Write the homepage copy", status: "done", priority: "high", due: -6, assignee: 1 },
      { title: "Pick the new type scale", status: "done", priority: "medium", due: -9, assignee: 2 },
      { title: "Compress the hero images", status: "done", priority: "low", due: -3, assignee: 0 },
    ],
  },
  {
    name: "Mobile app beta",
    description: "A TestFlight build for 50 customers, with crash reporting and a feedback form.",
    status: "active",
    tasks: [
      { title: "Ship build 0.9 to TestFlight", status: "in_progress", priority: "urgent", due: 2, assignee: 3 },
      { title: "Write the beta invite email", status: "in_review", priority: "medium", due: 3, assignee: 1 },
      { title: "Add a notification settings screen", status: "todo", priority: "medium", due: 10, assignee: 2 },
      { title: "Offline mode for the task list", status: "todo", priority: "high", due: 14, assignee: null },
      { title: "Take the App Store screenshots", status: "todo", priority: "low", due: 18, assignee: 0 },
      { title: "Add crash reporting", status: "done", priority: "high", due: -4, assignee: 3 },
      { title: "Fix the login loop on Android 13", status: "done", priority: "urgent", due: -1, assignee: 0 },
    ],
  },
  {
    name: "Onboarding emails",
    description: "A five-email sequence for new workspaces.",
    status: "archived",
    tasks: [
      { title: "Draft the welcome email", status: "done", priority: "medium", due: -30, assignee: 1 },
      { title: "Write the day-3 tips email", status: "done", priority: "low", due: -26, assignee: 2 },
      { title: "Set up the send schedule", status: "done", priority: "medium", due: -24, assignee: 0 },
    ],
  },
];

export interface SampleMember {
  userId: string;
  name: string;
}

export interface PlannedProject {
  name: string;
  description: string;
  status: "active" | "archived";
  tasks: {
    title: string;
    status: TaskStatus;
    priority: TaskPriority;
    dueDate: string;
    completedAt: string | null;
    assigneeId: string | null;
    assigneeName: string | null;
  }[];
}

/**
 * The rows to insert for `members` as of `now`. Pure, so tests/
 * sample-workspace.test.ts checks dates and assignment without a server.
 * Tasks are only assigned to people in `members`; with no members every
 * task is unassigned.
 */
export function planSampleWorkspace(members: SampleMember[], now: Date): PlannedProject[] {
  const day = (offset: number) => isoDay(new Date(now.getTime() + offset * 86_400_000));
  return SAMPLE_PROJECTS.map((p) => ({
    name: p.name,
    description: p.description,
    status: p.status,
    tasks: p.tasks.map((t) => {
      const who = t.assignee === null || members.length === 0 ? null : members[t.assignee % members.length];
      return {
        title: t.title,
        status: t.status,
        priority: t.priority,
        dueDate: day(t.due),
        completedAt: t.status === "done" ? `${day(Math.min(t.due, 0))}T16:00:00.000Z` : null,
        assigneeId: who?.userId ?? null,
        assigneeName: who?.name ?? null,
      };
    }),
  }));
}
