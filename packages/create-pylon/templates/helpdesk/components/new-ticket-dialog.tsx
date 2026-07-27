import React, { useEffect, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { PRIORITIES } from "@/lib/tickets";

export interface TicketDraft {
  subject: string;
  body: string;
  customerId: string;
  priority: string;
}

/**
 * Open a ticket on a customer's behalf — the "log a phone call" path. The body
 * is required as well as the subject: a ticket whose thread starts empty tells
 * the next agent nothing about what was actually reported.
 */
export function NewTicketDialog({
  open,
  customers,
  onOpenChange,
  onCreate,
}: {
  open: boolean;
  customers: Array<{ id: string; name: string }>;
  onOpenChange: (open: boolean) => void;
  onCreate: (draft: TicketDraft) => void | Promise<void>;
}) {
  const [subject, setSubject] = useState("");
  const [body, setBody] = useState("");
  const [customerId, setCustomerId] = useState("");
  const [priority, setPriority] = useState("normal");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setSubject("");
    setBody("");
    setCustomerId("");
    setPriority("normal");
    setBusy(false);
  }, [open]);

  const canSubmit = subject.trim().length > 0 && body.trim().length > 0 && !busy;

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    try {
      await onCreate({
        subject: subject.trim(),
        body: body.trim(),
        customerId,
        priority,
      });
      onOpenChange(false);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>New ticket</DialogTitle>
        </DialogHeader>
        <form onSubmit={submit} className="grid gap-4">
          <div className="space-y-1.5">
            <Label htmlFor="ticket-subject">Subject</Label>
            <Input
              id="ticket-subject"
              autoFocus
              value={subject}
              placeholder="Export is timing out on large date ranges"
              onChange={(event) => setSubject(event.target.value)}
            />
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <Label htmlFor="ticket-customer">Customer</Label>
              <Select
                id="ticket-customer"
                value={customerId}
                onChange={(event) => setCustomerId(event.target.value)}
              >
                <option value="">—</option>
                {customers.map((customer) => (
                  <option key={customer.id} value={customer.id}>
                    {customer.name}
                  </option>
                ))}
              </Select>
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="ticket-priority">Priority</Label>
              <Select
                id="ticket-priority"
                value={priority}
                onChange={(event) => setPriority(event.target.value)}
              >
                {PRIORITIES.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.label}
                  </option>
                ))}
              </Select>
            </div>
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="ticket-body">What did they report?</Label>
            <Textarea
              id="ticket-body"
              rows={4}
              value={body}
              placeholder="Pulling a 90-day export spins and eventually errors…"
              onChange={(event) => setBody(event.target.value)}
              className="resize-none"
            />
          </div>

          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={!canSubmit}>
              {busy ? "Creating…" : "Create ticket"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
