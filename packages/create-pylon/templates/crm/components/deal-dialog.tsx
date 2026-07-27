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
import { BOARD_STAGES } from "@/lib/pipeline";

export interface DealDraft {
  title: string;
  companyId: string;
  value: number;
  stage: string;
  closeDate: string;
}

/**
 * New deal. A dialog rather than an inline row: creating is deliberate and
 * occasional, and a permanent form above the board would push the pipeline —
 * the thing you came to look at — off the screen.
 */
export function DealDialog({
  open,
  companies,
  defaultStage = "lead",
  onOpenChange,
  onCreate,
}: {
  open: boolean;
  companies: Array<{ id: string; name: string }>;
  defaultStage?: string;
  onOpenChange: (open: boolean) => void;
  onCreate: (draft: DealDraft) => void | Promise<void>;
}) {
  const [title, setTitle] = useState("");
  const [companyId, setCompanyId] = useState("");
  const [value, setValue] = useState("");
  const [stage, setStage] = useState(defaultStage);
  const [closeDate, setCloseDate] = useState("");
  const [busy, setBusy] = useState(false);

  // Reset on each open so a cancelled draft doesn't reappear next time.
  useEffect(() => {
    if (!open) return;
    setTitle("");
    setCompanyId("");
    setValue("");
    setStage(defaultStage);
    setCloseDate("");
    setBusy(false);
  }, [open, defaultStage]);

  const canSubmit = title.trim().length > 0 && !busy;

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    try {
      await onCreate({
        title: title.trim(),
        companyId,
        value: Number(value) || 0,
        stage,
        closeDate,
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
          <DialogTitle>New deal</DialogTitle>
        </DialogHeader>
        <form onSubmit={submit} className="grid gap-4">
          <div className="space-y-1.5">
            <Label htmlFor="deal-title">Title</Label>
            <Input
              id="deal-title"
              autoFocus
              value={title}
              placeholder="Fleet dispatch rollout"
              onChange={(event) => setTitle(event.target.value)}
            />
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="deal-company">Company</Label>
            <Select
              id="deal-company"
              value={companyId}
              onChange={(event) => setCompanyId(event.target.value)}
            >
              <option value="">—</option>
              {companies.map((company) => (
                <option key={company.id} value={company.id}>
                  {company.name}
                </option>
              ))}
            </Select>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <Label htmlFor="deal-value">Value</Label>
              <Input
                id="deal-value"
                type="number"
                min="0"
                step="100"
                inputMode="numeric"
                value={value}
                placeholder="10000"
                onChange={(event) => setValue(event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="deal-stage">Stage</Label>
              <Select
                id="deal-stage"
                value={stage}
                onChange={(event) => setStage(event.target.value)}
              >
                {BOARD_STAGES.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.label}
                  </option>
                ))}
              </Select>
            </div>
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="deal-close">Expected close</Label>
            <Input
              id="deal-close"
              type="date"
              value={closeDate}
              onChange={(event) => setCloseDate(event.target.value)}
            />
          </div>

          <DialogFooter>
            <Button
              type="button"
              variant="ghost"
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={!canSubmit}>
              {busy ? "Creating…" : "Create deal"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
