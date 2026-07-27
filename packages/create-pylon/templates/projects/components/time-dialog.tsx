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
import { duration, parseDuration } from "@/lib/work";

/**
 * Log time against a task.
 *
 * The field accepts however people write time — "90", "1.5h", "1h30", "45m".
 * Rejecting "1h30" because it isn\'t "90" is the kind of friction that stops
 * time being logged at all, which costs far more than a lenient parser.
 */
export function TimeDialog({
  open,
  taskTitle,
  onOpenChange,
  onLog,
}: {
  open: boolean;
  taskTitle: string;
  onOpenChange: (open: boolean) => void;
  onLog: (minutes: number, note: string) => void | Promise<void>;
}) {
  const [value, setValue] = useState("");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setValue("");
    setNote("");
    setBusy(false);
  }, [open]);

  const minutes = parseDuration(value);
  const tooMuch = minutes !== null && Math.abs(minutes) > 1440;
  const canSubmit = minutes !== null && minutes !== 0 && !tooMuch && !busy;

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    try {
      await onLog(minutes as number, note.trim());
      onOpenChange(false);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Log time</DialogTitle>
        </DialogHeader>
        <form onSubmit={submit} className="grid gap-4">
          <p className="truncate text-[12px] text-muted-foreground">{taskTitle}</p>

          <div className="space-y-1.5">
            <Label htmlFor="time-value">Time</Label>
            <Input
              id="time-value"
              autoFocus
              placeholder="1h30, 90, or 45m"
              value={value}
              onChange={(event) => setValue(event.target.value)}
            />
            {minutes !== null && !tooMuch ? (
              <p className="text-[11px] text-muted-foreground">{duration(minutes)}</p>
            ) : null}
            {tooMuch ? (
              <p className="text-[11px] text-destructive">
                That\'s more than a day — check the value.
              </p>
            ) : null}
            {minutes === null && value.trim() ? (
              <p className="text-[11px] text-destructive">
                Try 90, 1.5h, 1h30, or 45m.
              </p>
            ) : null}
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="time-note">Note</Label>
            <Input
              id="time-note"
              placeholder="Optional"
              value={note}
              onChange={(event) => setNote(event.target.value)}
            />
          </div>

          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={!canSubmit}>
              {busy ? "Logging…" : "Log time"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
