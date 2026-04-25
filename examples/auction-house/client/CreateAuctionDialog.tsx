/**
 * Host an auction — title, description, kind, schedule, lot list.
 * Calls `createAuction` server function which atomically creates the
 * Auction + every Lot in one transaction.
 */
import { useState } from "react";
import { db } from "@pylonsync/react";
import { Loader2, Plus, Trash2 } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@pylonsync/example-ui/dialog";
import { Input } from "@pylonsync/example-ui/input";
import { Label } from "@pylonsync/example-ui/label";
import { Textarea } from "@pylonsync/example-ui/textarea";
import { navigate } from "./lib/util";

type DraftLot = {
  title: string;
  description: string;
  startingCents: number;
};

export function CreateAuctionDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const create = db.useMutation<unknown, { auctionId: string }>("createAuction");
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [kind, setKind] = useState<"timed" | "live">("timed");
  const [durationMins, setDurationMins] = useState(15);
  const [lots, setLots] = useState<DraftLot[]>([
    { title: "", description: "", startingCents: 10000 },
  ]);

  const reset = () => {
    setTitle("");
    setDescription("");
    setKind("timed");
    setDurationMins(15);
    setLots([{ title: "", description: "", startingCents: 10000 }]);
  };

  const updateLot = (i: number, patch: Partial<DraftLot>) =>
    setLots((arr) => arr.map((l, idx) => (idx === i ? { ...l, ...patch } : l)));

  const submit = async () => {
    const payload = {
      title: title.trim(),
      description: description.trim(),
      kind,
      startsAt: new Date().toISOString(),
      durationSecs: Math.max(60, durationMins * 60),
      lots: lots
        .filter((l) => l.title.trim())
        .map((l) => ({
          title: l.title.trim(),
          description: l.description.trim(),
          startingCents: Math.max(100, Math.round(l.startingCents)),
        })),
    };
    if (!payload.title || payload.lots.length === 0) return;
    try {
      const res = await create.mutate(payload);
      if (res?.auctionId) {
        const target =
          kind === "live"
            ? `#/a/${encodeURIComponent(res.auctionId)}/live`
            : `#/a/${encodeURIComponent(res.auctionId)}`;
        navigate(target);
        reset();
        onClose();
      }
    } catch {}
  };

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle>Host an auction</DialogTitle>
          <DialogDescription>
            Bundle multiple lots into a timed or live sale. Lots open at the
            scheduled time and bidders sync live.
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-3">
          <Field label="Title">
            <Input
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="Spring Modern Design"
            />
          </Field>
          <Field label="Description">
            <Textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              rows={2}
            />
          </Field>
          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-1.5">
              <Label>Kind</Label>
              <div className="flex gap-2">
                <Button
                  type="button"
                  variant={kind === "timed" ? "default" : "outline"}
                  size="sm"
                  onClick={() => setKind("timed")}
                  className="flex-1"
                >
                  Timed
                </Button>
                <Button
                  type="button"
                  variant={kind === "live" ? "default" : "outline"}
                  size="sm"
                  onClick={() => setKind("live")}
                  className="flex-1"
                >
                  Live
                </Button>
              </div>
            </div>
            <Field label="Total duration (minutes)">
              <Input
                type="number"
                min={1}
                value={durationMins}
                onChange={(e) => setDurationMins(Number(e.target.value))}
              />
            </Field>
          </div>

          <div className="rounded-lg border bg-secondary/30 p-3">
            <div className="mb-2 flex items-center justify-between">
              <Label className="text-xs">Lots</Label>
              <Button
                type="button"
                variant="outline"
                size="xs"
                onClick={() =>
                  setLots((arr) => [
                    ...arr,
                    { title: "", description: "", startingCents: 10000 },
                  ])
                }
              >
                <Plus className="size-3" />
                Add lot
              </Button>
            </div>
            <div className="flex flex-col gap-2">
              {lots.map((l, i) => (
                <div
                  key={i}
                  className="grid grid-cols-[1fr_120px_30px] gap-2 rounded-md border bg-background p-2"
                >
                  <Input
                    placeholder={`Lot ${i + 1} title`}
                    value={l.title}
                    onChange={(e) => updateLot(i, { title: e.target.value })}
                  />
                  <Input
                    type="number"
                    min={1}
                    placeholder="Starting $"
                    value={l.startingCents / 100}
                    onChange={(e) =>
                      updateLot(i, {
                        startingCents: Math.round(Number(e.target.value) * 100),
                      })
                    }
                  />
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="size-9 text-muted-foreground hover:text-destructive"
                    onClick={() =>
                      setLots((arr) => arr.filter((_, idx) => idx !== i))
                    }
                  >
                    <Trash2 className="size-3.5" />
                  </Button>
                </div>
              ))}
            </div>
          </div>

          {create.error && (
            <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
              {create.error.message}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={submit} disabled={create.loading}>
            {create.loading && <Loader2 className="size-4 animate-spin" />}
            Create auction
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="grid gap-1.5">
      <Label>{label}</Label>
      {children}
    </div>
  );
}
