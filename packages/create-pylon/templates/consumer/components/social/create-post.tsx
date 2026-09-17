"use client";

import React, { useEffect, useRef, useState } from "react";
import { callFn, uploadFile, useRouter } from "@pylonsync/react";
import { ImagePlus } from "lucide-react";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { CAPTION_MAX } from "@/lib/social";
import { cn } from "@/lib/utils";

const MAX_BYTES = 10 * 1024 * 1024;

/**
 * Choose a photo, pick the crop, write a caption, share. The photo uploads
 * as public, so every visitor can load it, then createPost stores the post.
 */
export function CreatePostDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const router = useRouter();
  const input = useRef<HTMLInputElement | null>(null);
  const [file, setFile] = useState<File | null>(null);
  const [preview, setPreview] = useState<string | null>(null);
  const [shape, setShape] = useState<"square" | "portrait">("square");
  const [caption, setCaption] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);

  useEffect(() => {
    if (!file) {
      setPreview(null);
      return;
    }
    const url = URL.createObjectURL(file);
    setPreview(url);
    return () => URL.revokeObjectURL(url);
  }, [file]);

  useEffect(() => {
    if (open) return;
    setFile(null);
    setCaption("");
    setShape("square");
    setError(null);
    setBusy(false);
  }, [open]);

  function choose(next: File | undefined | null) {
    if (!next) return;
    if (!next.type.startsWith("image/")) return setError("Choose a photo.");
    if (next.size > MAX_BYTES) return setError("Choose a photo smaller than 10 MB.");
    setError(null);
    setFile(next);
  }

  async function share() {
    if (!file) return;
    setBusy(true);
    setError(null);
    try {
      const photo = await uploadFile(file, { visibility: "public" });
      const out = await callFn<{ id: string }>("createPost", { imageUrl: photo.url, caption, shape });
      onOpenChange(false);
      router.push(`/p/${out.id}`);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Could not share that photo.");
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(next) => (busy ? null : onOpenChange(next))}>
      <DialogContent className={cn("gap-0 overflow-hidden p-0", file ? "max-w-3xl" : "max-w-md")}>
        <div className="flex h-11 items-center justify-center border-b px-12">
          <DialogTitle className="text-base font-semibold">Create new post</DialogTitle>
          {file ? (
            <button
              type="button"
              onClick={share}
              disabled={busy}
              className="absolute right-12 text-sm font-semibold text-brand hover:text-zinc-900 disabled:opacity-50"
            >
              {busy ? "Sharing…" : "Share"}
            </button>
          ) : null}
        </div>

        {!file ? (
          <div
            onDragOver={(e) => {
              e.preventDefault();
              setDragging(true);
            }}
            onDragLeave={() => setDragging(false)}
            onDrop={(e) => {
              e.preventDefault();
              setDragging(false);
              choose(e.dataTransfer.files?.[0]);
            }}
            className={cn("flex aspect-square flex-col items-center justify-center gap-4 p-8", dragging ? "bg-zinc-50" : "")}
          >
            <ImagePlus className="size-16 text-zinc-800" strokeWidth={1} />
            <p className="text-lg">Drag a photo here</p>
            <input ref={input} type="file" accept="image/*" className="sr-only" onChange={(e) => choose(e.target.files?.[0])} />
            <button
              type="button"
              onClick={() => input.current?.click()}
              className="rounded-lg bg-brand px-4 py-1.5 text-sm font-semibold text-white hover:opacity-90"
            >
              Select from computer
            </button>
            {error ? <p className="text-sm text-red-600">{error}</p> : null}
          </div>
        ) : (
          <div className="grid md:grid-cols-[1fr_300px]">
            <div className="flex items-center justify-center bg-zinc-100">
              {preview ? (
                <img src={preview} alt="" className={cn("w-full object-cover", shape === "portrait" ? "aspect-[4/5]" : "aspect-square")} />
              ) : null}
            </div>
            <div className="flex flex-col gap-3 border-l p-4">
              <div className="flex gap-1 rounded-lg bg-zinc-100 p-1 text-sm">
                {(["square", "portrait"] as const).map((s) => (
                  <button
                    key={s}
                    type="button"
                    onClick={() => setShape(s)}
                    aria-pressed={shape === s}
                    className={cn("flex-1 rounded-md py-1 font-medium", shape === s ? "bg-white shadow-sm" : "text-zinc-500")}
                  >
                    {s === "square" ? "Square" : "Portrait"}
                  </button>
                ))}
              </div>
              <textarea
                value={caption}
                onChange={(e) => setCaption(e.target.value)}
                maxLength={CAPTION_MAX}
                rows={8}
                placeholder="Write a caption…"
                aria-label="Caption"
                className="w-full resize-none bg-transparent text-sm outline-none placeholder:text-zinc-500"
              />
              <p className="text-right text-xs text-zinc-400">
                {caption.length.toLocaleString("en-US")}/{CAPTION_MAX.toLocaleString("en-US")}
              </p>
              {error ? <p className="text-sm text-red-600">{error}</p> : null}
              <button type="button" onClick={() => setFile(null)} disabled={busy} className="mt-auto text-left text-sm text-zinc-500 hover:text-zinc-900">
                Choose another photo
              </button>
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
