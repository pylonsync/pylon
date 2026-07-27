// The support model: statuses, priority, SLA, and the queue ordering.
//
// Pure — no React, no `db`. The rules that decide what an agent sees first are
// the most important logic in a helpdesk, so they live where they can be tested
// exhaustively without a server.

export interface Status {
  id: string;
  label: string;
  /** Solved and closed leave the queue. */
  terminal?: boolean;
}

export const STATUSES: Status[] = [
  { id: "open", label: "Open" },
  { id: "pending", label: "Pending" },
  { id: "solved", label: "Solved", terminal: true },
  { id: "closed", label: "Closed", terminal: true },
];

export interface Priority {
  id: string;
  label: string;
  /** Lower sorts first. */
  rank: number;
  /** Hours allowed for a first reply. */
  firstResponseHours: number;
}

export const PRIORITIES: Priority[] = [
  { id: "urgent", label: "Urgent", rank: 0, firstResponseHours: 1 },
  { id: "high", label: "High", rank: 1, firstResponseHours: 4 },
  { id: "normal", label: "Normal", rank: 2, firstResponseHours: 24 },
  { id: "low", label: "Low", rank: 3, firstResponseHours: 72 },
];

export interface Ticket {
  id: string;
  subject: string;
  status: string;
  priority: string;
  customerId?: string | null;
  assigneeId?: string | null;
  createdAt?: string | null;
  updatedAt?: string | null;
  /** When an agent first replied; null while the customer is still waiting. */
  firstRespondedAt?: string | null;
}

export function statusById(id: string): Status | undefined {
  return STATUSES.find((s) => s.id === id);
}

export function priorityById(id: string): Priority | undefined {
  return PRIORITIES.find((p) => p.id === id);
}

export function isValidStatus(id: string): boolean {
  return STATUSES.some((s) => s.id === id);
}

export function isValidPriority(id: string): boolean {
  return PRIORITIES.some((p) => p.id === id);
}

export function isOpen(ticket: { status: string }): boolean {
  return !statusById(ticket.status)?.terminal;
}

export type SlaState = "met" | "due" | "breached" | "none";

/**
 * Where a ticket stands against its first-response target.
 *
 *   met      — an agent has already replied
 *   breached — no reply and the window has passed
 *   due      — no reply, inside the window
 *   none     — terminal, or unparseable dates; nothing to chase
 *
 * A solved ticket that was answered late is still `met`: the SLA question is
 * "did we reply", and re-flagging closed work just adds noise to the queue.
 */
export function slaState(
  ticket: Ticket,
  now: number = Date.now(),
): SlaState {
  if (!isOpen(ticket)) return "none";
  if (ticket.firstRespondedAt) return "met";
  const created = Date.parse(ticket.createdAt ?? "");
  if (!Number.isFinite(created)) return "none";
  const hours = priorityById(ticket.priority)?.firstResponseHours;
  if (hours === undefined) return "none";
  return now > created + hours * 3_600_000 ? "breached" : "due";
}

/** Minutes left before the first-response target, negative once breached. */
export function minutesToBreach(
  ticket: Ticket,
  now: number = Date.now(),
): number | null {
  if (!isOpen(ticket) || ticket.firstRespondedAt) return null;
  const created = Date.parse(ticket.createdAt ?? "");
  const hours = priorityById(ticket.priority)?.firstResponseHours;
  if (!Number.isFinite(created) || hours === undefined) return null;
  return Math.round((created + hours * 3_600_000 - now) / 60_000);
}

/**
 * Queue order: breached first, then by priority, then oldest first.
 *
 * Oldest-first within a priority is deliberate — sorting newest-first is how a
 * ticket sits at the bottom of a busy queue for a week.
 */
export function queueOrder(tickets: Ticket[], now: number = Date.now()): Ticket[] {
  return [...tickets].sort((a, b) => {
    const aBreached = slaState(a, now) === "breached" ? 0 : 1;
    const bBreached = slaState(b, now) === "breached" ? 0 : 1;
    if (aBreached !== bBreached) return aBreached - bBreached;

    const aRank = priorityById(a.priority)?.rank ?? 99;
    const bRank = priorityById(b.priority)?.rank ?? 99;
    if (aRank !== bRank) return aRank - bRank;

    return (a.createdAt ?? "").localeCompare(b.createdAt ?? "");
  });
}

export interface QueueCounts {
  open: number;
  pending: number;
  solved: number;
  closed: number;
  unassigned: number;
  breached: number;
}

export function counts(tickets: Ticket[], now: number = Date.now()): QueueCounts {
  const result: QueueCounts = {
    open: 0,
    pending: 0,
    solved: 0,
    closed: 0,
    unassigned: 0,
    breached: 0,
  };
  for (const ticket of tickets) {
    if (ticket.status in result) {
      result[ticket.status as keyof QueueCounts] += 1;
    }
    if (isOpen(ticket) && !ticket.assigneeId) result.unassigned += 1;
    if (slaState(ticket, now) === "breached") result.breached += 1;
  }
  return result;
}

export interface Filter {
  status?: string;
  priority?: string;
  assigneeId?: string;
  /** "unassigned" narrows to tickets with no agent. */
  unassigned?: boolean;
}

export function applyFilter(tickets: Ticket[], filter: Filter): Ticket[] {
  return tickets.filter((ticket) => {
    if (filter.status && ticket.status !== filter.status) return false;
    if (filter.priority && ticket.priority !== filter.priority) return false;
    if (filter.assigneeId && ticket.assigneeId !== filter.assigneeId) return false;
    if (filter.unassigned && ticket.assigneeId) return false;
    return true;
  });
}

/** "#1042" — short, stable, and greppable in an email thread. */
export function ticketNumber(id: string, createdAt?: string | null): string {
  // Derived from the id so it needs no counter and can't collide across
  // concurrent inserts; the created date is only a tiebreaker for display.
  let hash = 0;
  const seed = `${id}${createdAt ?? ""}`;
  for (let i = 0; i < seed.length; i += 1) {
    hash = (hash * 31 + seed.charCodeAt(i)) | 0;
  }
  return `#${(Math.abs(hash) % 9000) + 1000}`;
}
