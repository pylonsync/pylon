"use client";

import React, { useState } from "react";
import { db, useRouter } from "@pylonsync/react";
import { ImagePlus } from "lucide-react";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Textarea } from "../ui/textarea";
import { Label } from "../ui/label";
import { AuthGate, MarketProvider, useIdentity } from "./MarketProvider";
import { conditionLabel, makeSlug } from "./market";

const CATEGORIES = [
  "furniture", "electronics", "cameras", "bikes", "audio", "kitchen",
  "instruments", "outdoor", "apparel", "other",
];
const CONDITIONS = ["new", "like-new", "good", "fair"];

const selectClass =
  "flex min-h-11 w-full rounded-lg border border-input bg-background px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring";

async function uploadListingPhoto(file: File) {
  const initResponse = await fetch("/api/files/init", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      filename: file.name,
      mimeType: file.type,
      size: file.size,
    }),
  });
  if (!initResponse.ok) throw new Error("Could not prepare that upload.");
  const init = await initResponse.json() as { assetId: string; uploadUrl: string };

  const uploadResponse = await fetch(init.uploadUrl, {
    method: "PUT",
    headers: { "Content-Type": file.type },
    body: file,
  });
  if (!uploadResponse.ok) throw new Error("Could not upload that photo.");

  const confirmResponse = await fetch("/api/files/confirm", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ assetId: init.assetId }),
  });
  if (!confirmResponse.ok) throw new Error("Could not finish that upload.");
  return confirmResponse.json() as Promise<{ id: string; url: string; size: number }>;
}

function Form() {
  // Rendered inside <AuthGate>, so identity is guaranteed non-null here.
  const identity = useIdentity();
  const userId = identity?.userId ?? "";
  const name = identity?.name ?? "you";
  const router = useRouter();
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [imageUrl, setImageUrl] = useState("");
  const [price, setPrice] = useState("");
  const [category, setCategory] = useState(CATEGORIES[0]);
  const [condition, setCondition] = useState("good");
  const [photoBusy, setPhotoBusy] = useState(false);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function selectPhoto(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    if (!file.type.startsWith("image/")) {
      setErr("Choose an image file.");
      return;
    }
    if (file.size > 8 * 1024 * 1024) {
      setErr("Choose an image smaller than 8 MB.");
      return;
    }
    setPhotoBusy(true);
    setErr(null);
    try {
      const uploaded = await uploadListingPhoto(file);
      setImageUrl(uploaded.url);
    } catch (e) {
      setErr((e as Error).message ?? "Could not upload that photo.");
    } finally {
      setPhotoBusy(false);
    }
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    const value = Number.parseFloat(price);
    if (!title.trim()) return setErr("Give your item a title.");
    if (!imageUrl.startsWith("/")) {
      try {
        const photo = new URL(imageUrl);
        if (!["http:", "https:"].includes(photo.protocol)) throw new Error();
      } catch {
        return setErr("Upload a photo or add a valid photo URL.");
      }
    }
    if (!Number.isFinite(value) || value < 0) return setErr("Set a price.");
    setBusy(true);
    setErr(null);
    // Local-first by default — no createListing function, no opt-in
    // optimism flag. `db.insert` paints the listing into the local store
    // synchronously (it's in the "just listed" ticker before the network
    // call even leaves the tab) and pushes in the background. `sellerId`
    // is declared `field.owner()` in app.ts, so the server stamps and
    // verifies it from the session — we send our own id only so the
    // optimistic row is complete; a forged seller id would be rejected.
    const seed = Math.random().toString(36).slice(2, 8);
    const slug = makeSlug(title.trim(), seed);
    try {
      await db.insert("Listing", {
        sellerId: userId,
        sellerName: name,
        title: title.trim(),
        slug,
        description: description.trim(),
        price: Math.max(0, Math.round(value * 100) / 100),
        category,
        condition,
        status: "active",
        imageUrl,
        seed,
        createdAt: new Date().toISOString(),
      });
      router.push(`/listing/${slug}`);
    } catch (e) {
      setErr((e as Error).message ?? "Could not post your listing.");
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit} className="space-y-5">
      <div className="space-y-1.5">
        <Label htmlFor="title">Title</Label>
        <Input
          id="title"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          placeholder="e.g. Herman Miller Aeron, size B"
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="description">Description</Label>
        <Textarea
          id="description"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          placeholder="Condition details, dimensions, why you're selling…"
          rows={4}
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="listing-photo">Photo</Label>
        <label
          htmlFor="listing-photo"
          className="flex min-h-28 cursor-pointer flex-col items-center justify-center gap-2 rounded-xl border border-dashed border-border bg-muted/40 px-5 py-6 text-center transition-colors hover:bg-muted/70 focus-within:ring-2 focus-within:ring-ring"
        >
          <ImagePlus aria-hidden="true" className="size-5 text-muted-foreground" />
          <span className="text-sm font-medium">
            {photoBusy ? "Uploading photo…" : imageUrl ? "Replace photo" : "Upload a photo"}
          </span>
          <span className="text-xs text-muted-foreground">JPG, PNG, or WebP up to 8 MB</span>
          <input
            id="listing-photo"
            type="file"
            accept="image/jpeg,image/png,image/webp"
            onChange={selectPhoto}
            disabled={photoBusy}
            className="sr-only"
          />
        </label>
        <div className="flex items-center gap-3 py-1" aria-hidden="true">
          <span className="h-px flex-1 bg-border" />
          <span className="text-[11px] uppercase tracking-[0.12em] text-muted-foreground">or use a link</span>
          <span className="h-px flex-1 bg-border" />
        </div>
        <Label htmlFor="imageUrl" className="sr-only">Photo URL</Label>
        <Input
          id="imageUrl"
          type="url"
          value={imageUrl}
          onChange={(e) => setImageUrl(e.target.value)}
          placeholder="https://example.com/item.jpg"
          aria-describedby="image-help"
        />
        <p id="image-help" className="text-xs leading-5 text-muted-foreground">
          Clear, well-lit photos get more interest.
        </p>
        {imageUrl ? (
          <div className="mt-3 aspect-[16/10] overflow-hidden rounded-xl bg-muted shadow-[var(--shadow-border)]">
            <img
              src={imageUrl}
              alt="Listing photo preview"
              className="h-full w-full object-cover outline outline-1 -outline-offset-1 outline-black/10 dark:outline-white/10"
            />
          </div>
        ) : null}
      </div>
      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-1.5">
          <Label htmlFor="price">Price ($)</Label>
          <Input
            id="price"
            type="number"
            min="0"
            step="1"
            value={price}
            onChange={(e) => setPrice(e.target.value)}
            placeholder="0"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="condition">Condition</Label>
          <select
            id="condition"
            value={condition}
            onChange={(e) => setCondition(e.target.value)}
            className={selectClass}
          >
            {CONDITIONS.map((c) => (
              <option key={c} value={c}>
                {conditionLabel(c)}
              </option>
            ))}
          </select>
        </div>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="category">Category</Label>
        <select
          id="category"
          value={category}
          onChange={(e) => setCategory(e.target.value)}
          className={selectClass}
        >
          {CATEGORIES.map((c) => (
            <option key={c} value={c}>
              {c[0]?.toUpperCase()}{c.slice(1)}
            </option>
          ))}
        </select>
      </div>
      {err ? <p className="text-sm text-destructive">{err}</p> : null}
      <Button type="submit" disabled={busy || photoBusy} className="w-full">
        {busy ? "Posting…" : "Post listing"}
      </Button>
      <p className="text-center text-xs text-muted-foreground">
        Posting as <span className="font-medium">{name}</span>. Buyers'
        offers land in <a href="/me" className="underline">Dashboard</a>.
      </p>
    </form>
  );
}

export function SellForm() {
  return (
    <MarketProvider>
      <AuthGate
        title="Sign in to list an item"
        blurb="Selling needs an account so your listings stay tied to you. The demo account is ready; just select Log in."
      >
        <Form />
      </AuthGate>
    </MarketProvider>
  );
}
