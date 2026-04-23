/**
 * shadcn-shaped UI primitives styled with the DESIGN.md token system.
 *
 * Keeps the component API (Dialog, AlertDialog, Toaster, useToast,
 * Input, Button) aligned with shadcn/ui so the surface feels familiar;
 * we wire them with our own CSS so the editor's chrome stays on one
 * token source. Radix isn't installed — focus trap, Escape, and
 * overlay dismiss are implemented directly. Good enough for the
 * example; swap in radix if you need screen-reader grade a11y.
 */

import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { Icon } from "./icons";

// ---------------------------------------------------------------------------
// Focus management — traps tab focus inside a container while open.
// Returns a cleanup function; call once on mount, kill on unmount.
// ---------------------------------------------------------------------------

function useFocusTrap(active: boolean, containerRef: React.RefObject<HTMLElement | null>) {
  useEffect(() => {
    if (!active) return;
    const container = containerRef.current;
    if (!container) return;

    const previouslyFocused = document.activeElement as HTMLElement | null;

    // Focus the first focusable element, preferring autoFocus / the
    // primary button so users can press Enter immediately.
    const focusables = () =>
      Array.from(
        container.querySelectorAll<HTMLElement>(
          'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
        ),
      ).filter((el) => !el.hasAttribute("disabled"));

    const first = focusables()[0];
    first?.focus();

    function onKey(e: KeyboardEvent) {
      if (e.key !== "Tab") return;
      const list = focusables();
      if (list.length === 0) return;
      const first = list[0];
      const last = list[list.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      previouslyFocused?.focus?.();
    };
  }, [active, containerRef]);
}

// ---------------------------------------------------------------------------
// Dialog — base overlay + content. shadcn API: <Dialog open onOpenChange>
// ---------------------------------------------------------------------------

export function Dialog({
  open,
  onOpenChange,
  children,
  title,
  description,
  footer,
  size = "md",
}: {
  open: boolean;
  onOpenChange: (next: boolean) => void;
  children?: React.ReactNode;
  title?: React.ReactNode;
  description?: React.ReactNode;
  footer?: React.ReactNode;
  size?: "sm" | "md" | "lg";
}) {
  const ref = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const descId = useId();
  useFocusTrap(open, ref);

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onOpenChange(false);
    }
    document.addEventListener("keydown", onKey);
    // Lock body scroll while open.
    const prev = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.removeEventListener("keydown", onKey);
      document.body.style.overflow = prev;
    };
  }, [open, onOpenChange]);

  if (!open) return null;

  const widths = { sm: 360, md: 440, lg: 560 };

  return (
    <div
      className="ui-overlay"
      onClick={(e) => {
        if (e.target === e.currentTarget) onOpenChange(false);
      }}
      role="presentation"
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby={title ? titleId : undefined}
        aria-describedby={description ? descId : undefined}
        className="ui-dialog"
        style={{ width: widths[size] }}
      >
        {(title || description) && (
          <div className="ui-dialog-head">
            {title && <h2 id={titleId} className="ui-dialog-title">{title}</h2>}
            {description && <p id={descId} className="ui-dialog-desc">{description}</p>}
          </div>
        )}
        {children && <div className="ui-dialog-body">{children}</div>}
        {footer && <div className="ui-dialog-footer">{footer}</div>}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// AlertDialog — for destructive confirmations. shadcn API mirrored.
// ---------------------------------------------------------------------------

export function AlertDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  destructive = false,
  onConfirm,
}: {
  open: boolean;
  onOpenChange: (next: boolean) => void;
  title: string;
  description?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  destructive?: boolean;
  onConfirm: () => void | Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  async function handleConfirm() {
    setBusy(true);
    try {
      await onConfirm();
      onOpenChange(false);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => !busy && onOpenChange(next)}
      title={title}
      description={description}
      size="sm"
      footer={
        <>
          <button className="btn ghost" onClick={() => onOpenChange(false)} disabled={busy}>
            {cancelLabel}
          </button>
          <button
            className={destructive ? "btn danger" : "btn primary"}
            onClick={handleConfirm}
            disabled={busy}
          >
            {busy ? "…" : confirmLabel}
          </button>
        </>
      }
    />
  );
}

// ---------------------------------------------------------------------------
// PromptDialog — asks for a single string. Used where native prompt() was.
// ---------------------------------------------------------------------------

export function PromptDialog({
  open,
  onOpenChange,
  title,
  description,
  defaultValue = "",
  placeholder,
  confirmLabel = "Save",
  onConfirm,
}: {
  open: boolean;
  onOpenChange: (next: boolean) => void;
  title: string;
  description?: string;
  defaultValue?: string;
  placeholder?: string;
  confirmLabel?: string;
  onConfirm: (value: string) => void | Promise<void>;
}) {
  const [value, setValue] = useState(defaultValue);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (open) setValue(defaultValue);
  }, [open, defaultValue]);

  async function handleConfirm() {
    if (!value.trim()) return;
    setBusy(true);
    try {
      await onConfirm(value.trim());
      onOpenChange(false);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => !busy && onOpenChange(next)}
      title={title}
      description={description}
      size="sm"
      footer={
        <>
          <button className="btn ghost" onClick={() => onOpenChange(false)} disabled={busy}>
            Cancel
          </button>
          <button className="btn primary" onClick={handleConfirm} disabled={busy || !value.trim()}>
            {busy ? "…" : confirmLabel}
          </button>
        </>
      }
    >
      <input
        className="insp-input"
        autoFocus
        value={value}
        onChange={(e) => setValue(e.target.value)}
        placeholder={placeholder}
        onKeyDown={(e) => {
          if (e.key === "Enter") handleConfirm();
        }}
      />
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// Toast — Sonner-shaped, imperative via useToast()
// ---------------------------------------------------------------------------

type ToastVariant = "default" | "success" | "danger";
type ToastMsg = { id: number; title: string; description?: string; variant: ToastVariant };

const ToastContext = createContext<{
  push: (m: Omit<ToastMsg, "id">) => void;
} | null>(null);

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = useState<ToastMsg[]>([]);
  const nextId = useRef(1);

  const push = useCallback((m: Omit<ToastMsg, "id">) => {
    const id = nextId.current++;
    setToasts((prev) => [...prev, { ...m, id }]);
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
    }, 4200);
  }, []);

  const value = useMemo(() => ({ push }), [push]);

  return (
    <ToastContext.Provider value={value}>
      {children}
      <div className="ui-toaster" aria-live="polite" aria-atomic="true">
        {toasts.map((t) => (
          <div key={t.id} className={`ui-toast variant-${t.variant}`} role="status">
            <div className="ui-toast-dot" />
            <div className="ui-toast-body">
              <div className="ui-toast-title">{t.title}</div>
              {t.description && <div className="ui-toast-desc">{t.description}</div>}
            </div>
            <button
              className="ui-toast-close"
              onClick={() => setToasts((prev) => prev.filter((x) => x.id !== t.id))}
              aria-label="Dismiss"
            ><Icon name="X" size={14} /></button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast() {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used inside <ToastProvider>");
  return {
    toast: (title: string, description?: string) => ctx.push({ title, description, variant: "default" }),
    success: (title: string, description?: string) => ctx.push({ title, description, variant: "success" }),
    error: (title: string, description?: string) => ctx.push({ title, description, variant: "danger" }),
  };
}
