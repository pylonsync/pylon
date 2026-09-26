"use client";

import React, { useState } from "react";
import { db, useRouter } from "@pylonsync/react";
import { ImagePlus } from "lucide-react";
import { cn } from "@/lib/utils";
import { Spinner } from "@/components/ui/spinner";
import { CATEGORIES, CONDITIONS } from "../lib/catalog";
import { ListingCard } from "./ListingCard";
import { AuthGate, MarketProvider, useIdentity } from "./MarketProvider";
import { conditionLabel, makeSlug, type Listing } from "./market";

const INPUT =
  "h-11 w-full rounded-xl bg-card px-3.5 text-[15px] shadow-[var(--shadow-border)] outline-none placeholder:text-muted-foreground focus:ring-2 focus:ring-ring focus-visible:outline-none";

async function uploadListingPhoto(file: File) {
  const initResponse = await fetch("/api/files/init", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    // Every buyer sees the listing photo, signed in or not.
    body: JSON.stringify({
      filename: file.name,
      mimeType: file.type,
      size: file.size,
      visibility: "public",
    }),
  });
  if (!initResponse.ok) throw new Error("Could not prepare that upload.");
  const init = (await initResponse.json()) as { assetId: string; uploadUrl: string };

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

function isPhotoUrl(value: string): boolean {
  if (value.startsWith("/")) return true;
  try {
    return ["http:", "https:"].includes(new URL(value).protocol);
  } catch {
    return false;
  }
}

function Label({ htmlFor, children }: { htmlFor?: string; children: React.ReactNode }) {
  return (
    <label htmlFor={htmlFor} className="text-sm font-medium">
      {children}
    </label>
  );
}

function Form() {
  // Rendered inside <AuthGate>, so identity is non-null here.
  const identity = useIdentity();
  const userId = identity?.userId ?? "";
  const name = identity?.name ?? "you";
  const router = useRouter();
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  // The photo is either an upload (a same-origin /api/files path) or a link
  // the seller pastes. An upload clears the link field.
  const [imageUrl, setImageUrl] = useState("");
  const uploaded = imageUrl.startsWith("/");
  const [price, setPrice] = useState("");
  const [category, setCategory] = useState(CATEGORIES[0]!.id);
  const [condition, setCondition] = useState("good");
  const [location, setLocation] = useState("");
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
    if (!isPhotoUrl(imageUrl)) return setErr("Upload a photo or add a photo link.");
    if (!Number.isFinite(value) || value < 0) return setErr("Set a price.");
    setBusy(true);
    setErr(null);
    // db.insert writes the listing to the local store at once (it shows in
    // every open browse grid and activity feed) and pushes to the server in
    // the background. `sellerId` is `field.owner()` in app.ts, so the server
    // stamps and verifies it from the session; a forged id is rejected.
    const seed = Math.random().toString(36).slice(2, 8);
    const slug = makeSlug(title.trim(), seed);
    try {
      await db.insert("Listing", {
        sellerId: userId,
        sellerName: name,
        location: location.trim(),
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

  const draft: Listing = {
    id: "preview",
    sellerId: userId,
    sellerName: name,
    location: location.trim() || undefined,
    title: title.trim() || "Your item title",
    slug: "",
    description,
    price: Number.parseFloat(price) || 0,
    category,
    condition,
    status: "active",
    imageUrl: isPhotoUrl(imageUrl) ? imageUrl : undefined,
    seed: "preview",
    createdAt: new Date().toISOString(),
  };

  return (
    <div className="grid gap-10 lg:grid-cols-[minmax(0,1fr)_300px] xl:gap-16">
      <form
        onSubmit={submit}
        // Fields are checked in submit(). Native validation would reject the
        // same-origin path of an uploaded photo in the type="url" field.
        noValidate
        className="flex min-w-0 flex-col gap-7"
      >
        <div className="flex flex-col gap-2">
          <Label htmlFor="listing-photo">Photo</Label>
          <label
            htmlFor="listing-photo"
            className="group flex min-h-44 cursor-pointer flex-col items-center justify-center gap-2 rounded-2xl border border-dashed border-input bg-card/60 px-5 py-8 text-center transition-colors hover:bg-card focus-within:ring-2 focus-within:ring-ring"
          >
            {isPhotoUrl(imageUrl) && !photoBusy ? (
              <img
                src={imageUrl}
                alt="Listing photo"
                width="96"
                height="120"
                className="h-[120px] w-24 rounded-lg object-cover shadow-[var(--shadow-float)]"
              />
            ) : (
            <span className="grid size-11 place-items-center rounded-full bg-muted transition-transform duration-300 group-hover:scale-105">
              {photoBusy ? (
                <Spinner />
              ) : (
                <ImagePlus aria-hidden="true" strokeWidth={1.5} className="size-5" />
              )}
            </span>
            )}
            <span className="text-sm font-medium">
              {photoBusy
                ? "Uploading photo"
                : imageUrl
                  ? "Replace photo"
                  : "Upload a photo"}
            </span>
            <span className="text-xs text-muted-foreground">
              JPG, PNG, or WebP up to 8 MB. Natural light and a plain background work best.
            </span>
            <input
              id="listing-photo"
              name="photo"
              type="file"
              accept="image/jpeg,image/png,image/webp"
              onChange={selectPhoto}
              disabled={photoBusy}
              className="sr-only"
            />
          </label>
          <label htmlFor="imageUrl" className="sr-only">
            Photo link
          </label>
          <input
            id="imageUrl"
            name="imageUrl"
            type="url"
            autoComplete="off"
            spellCheck={false}
            value={uploaded ? "" : imageUrl}
            onChange={(e) => setImageUrl(e.target.value)}
            placeholder="Or paste a photo link: https://…"
            className={INPUT}
          />
        </div>

        <div className="flex flex-col gap-2">
          <Label htmlFor="title">Title</Label>
          <input
            id="title"
            name="title"
            autoComplete="off"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="Walnut desk lamp, 1970s"
            maxLength={80}
            className={INPUT}
          />
        </div>

        <fieldset className="flex flex-col gap-2">
          <legend className="mb-2 text-sm font-medium">Category</legend>
          <div className="flex flex-wrap gap-1.5">
            {CATEGORIES.map((c) => (
              <label
                key={c.id}
                className={cn(
                  "inline-flex min-h-9 cursor-pointer items-center rounded-full px-3.5 text-[13px] font-medium transition-[background-color,color,box-shadow] duration-200 has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring",
                  category === c.id
                    ? "bg-foreground text-background"
                    : "bg-card shadow-[var(--shadow-border)] hover:shadow-[var(--shadow-border-hover)]",
                )}
              >
                <input
                  type="radio"
                  name="category"
                  value={c.id}
                  checked={category === c.id}
                  onChange={() => setCategory(c.id)}
                  className="sr-only"
                />
                {c.label}
              </label>
            ))}
          </div>
        </fieldset>

        <fieldset className="flex flex-col gap-2">
          <legend className="mb-2 text-sm font-medium">Condition</legend>
          <div className="grid grid-cols-4 gap-1 rounded-full bg-muted p-1">
            {CONDITIONS.map((c) => (
              <label
                key={c}
                className={cn(
                  "flex min-h-9 cursor-pointer items-center justify-center rounded-full text-[13px] font-medium transition-[background-color,color,box-shadow] duration-200 has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring",
                  condition === c
                    ? "bg-card text-foreground shadow-[var(--shadow-border)]"
                    : "text-muted-foreground hover:text-foreground",
                )}
              >
                <input
                  type="radio"
                  name="condition"
                  value={c}
                  checked={condition === c}
                  onChange={() => setCondition(c)}
                  className="sr-only"
                />
                {conditionLabel(c)}
              </label>
            ))}
          </div>
        </fieldset>

        <div className="grid gap-5 sm:grid-cols-2">
          <div className="flex flex-col gap-2">
            <Label htmlFor="price">Price</Label>
            <div className="flex h-11 items-center rounded-xl bg-card px-3.5 shadow-[var(--shadow-border)] focus-within:ring-2 focus-within:ring-ring">
              <span className="text-[15px] text-muted-foreground">$</span>
              <input
                id="price"
                name="price"
                type="number"
                inputMode="decimal"
                autoComplete="off"
                min="0"
                step="1"
                value={price}
                onChange={(e) => setPrice(e.target.value)}
                placeholder="0"
                className="h-full w-full bg-transparent pl-1 text-[15px] tabular-nums outline-none focus-visible:outline-none"
              />
            </div>
          </div>
          <div className="flex flex-col gap-2">
            <Label htmlFor="location">Location</Label>
            <input
              id="location"
              name="location"
              autoComplete="address-level2"
              value={location}
              onChange={(e) => setLocation(e.target.value)}
              placeholder="Denver, CO"
              className={INPUT}
            />
          </div>
        </div>

        <div className="flex flex-col gap-2">
          <Label htmlFor="description">Description</Label>
          <textarea
            id="description"
            name="description"
            autoComplete="off"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="Condition, measurements, what is included, pickup or shipping"
            rows={5}
            className="w-full resize-y rounded-xl bg-card px-3.5 py-3 text-[15px] leading-6 shadow-[var(--shadow-border)] outline-none placeholder:text-muted-foreground focus:ring-2 focus:ring-ring focus-visible:outline-none"
          />
        </div>

        <div aria-live="polite">
          {err ? (
            <p className="rounded-xl bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {err}
            </p>
          ) : null}
        </div>

        <div className="flex flex-col gap-3 border-t border-border/70 pt-6 sm:flex-row sm:items-center sm:justify-between">
          <p className="text-sm text-muted-foreground">
            Posting as <span className="font-medium text-foreground">{name}</span>.
            Offers arrive on your{" "}
            <a href="/me" className="underline underline-offset-4">
              dashboard
            </a>
            .
          </p>
          <button
            type="submit"
            disabled={busy || photoBusy}
            className="inline-flex min-h-12 items-center justify-center gap-2 rounded-full bg-primary px-7 text-[15px] font-medium text-primary-foreground transition-[scale] active:scale-[0.98] disabled:opacity-60"
          >
            {busy ? <Spinner /> : null}
            {busy ? "Posting" : "Post listing"}
          </button>
        </div>
      </form>

      <aside className="hidden lg:block">
        <div className="sticky top-24">
          <h2 className="mb-3 text-sm font-medium text-muted-foreground">
            Preview in the grid
          </h2>
          <div className="pointer-events-none" inert>
            <ListingCard listing={draft} sizes="300px" />
          </div>
        </div>
      </aside>
    </div>
  );
}

export function SellForm() {
  return (
    <MarketProvider>
      <AuthGate
        title="Sign in to list an item"
        blurb="Listings belong to an account so offers reach you. The demo account is filled in."
      >
        <Form />
      </AuthGate>
    </MarketProvider>
  );
}
