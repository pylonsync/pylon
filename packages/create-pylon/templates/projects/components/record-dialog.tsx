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

export interface FieldSpec {
  name: string;
  label: string;
  type?: "text" | "email" | "tel";
  placeholder?: string;
  required?: boolean;
  /** Renders a picker instead of a text input. */
  options?: Array<{ value: string; label: string }>;
}

/**
 * A create dialog driven by a field list — used for companies and contacts,
 * which differ only in their fields. Deals get their own dialog because value,
 * stage and close date need real layout rather than a stack of text inputs.
 */
export function RecordDialog({
  open,
  title,
  fields,
  submitLabel,
  onOpenChange,
  onCreate,
}: {
  open: boolean;
  title: string;
  fields: FieldSpec[];
  submitLabel: string;
  onOpenChange: (open: boolean) => void;
  onCreate: (values: Record<string, string>) => void | Promise<void>;
}) {
  const [values, setValues] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setValues({});
    setBusy(false);
  }, [open]);

  const missing = fields.some(
    (field) => field.required && !(values[field.name] ?? "").trim(),
  );
  const canSubmit = !missing && !busy;

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    try {
      const trimmed: Record<string, string> = {};
      for (const [key, value] of Object.entries(values)) {
        const clean = value.trim();
        if (clean) trimmed[key] = clean;
      }
      await onCreate(trimmed);
      onOpenChange(false);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        <form onSubmit={submit} className="grid gap-4">
          {fields.map((field, index) => (
            <div key={field.name} className="space-y-1.5">
              <Label htmlFor={`record-${field.name}`}>{field.label}</Label>
              {field.options ? (
                <Select
                  id={`record-${field.name}`}
                  value={values[field.name] ?? ""}
                  onChange={(event) =>
                    setValues((v) => ({ ...v, [field.name]: event.target.value }))
                  }
                >
                  <option value="">—</option>
                  {field.options.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </Select>
              ) : (
                <Input
                  id={`record-${field.name}`}
                  autoFocus={index === 0}
                  type={field.type ?? "text"}
                  placeholder={field.placeholder}
                  value={values[field.name] ?? ""}
                  onChange={(event) =>
                    setValues((v) => ({ ...v, [field.name]: event.target.value }))
                  }
                />
              )}
            </div>
          ))}
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={!canSubmit}>
              {busy ? "Saving…" : submitLabel}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
