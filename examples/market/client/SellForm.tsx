"use client";

import React, { useState } from "react";
import { callFn, useRouter } from "@pylonsync/react";
import { Button } from "@pylonsync/example-ui/button";
import { Input } from "@pylonsync/example-ui/input";
import { Textarea } from "@pylonsync/example-ui/textarea";
import { Label } from "@pylonsync/example-ui/label";
import { MarketProvider, useIdentity } from "./MarketProvider";

const CATEGORIES = [
  "furniture", "electronics", "cameras", "bikes", "audio", "kitchen",
  "instruments", "outdoor", "apparel", "other",
];
const CONDITIONS = ["new", "like-new", "good", "fair"];

const selectClass =
  "flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm outline-none focus-visible:ring-1 focus-visible:ring-ring";

function Form() {
  const { name } = useIdentity();
  const router = useRouter();
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [price, setPrice] = useState("");
  const [category, setCategory] = useState(CATEGORIES[0]);
  const [condition, setCondition] = useState("good");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    const value = Number.parseFloat(price);
    if (!title.trim()) return setErr("Give your item a title.");
    if (!Number.isFinite(value) || value < 0) return setErr("Set a price.");
    setBusy(true);
    setErr(null);
    try {
      const res = await callFn<{ id: string }>("createListing", {
        title,
        description,
        price: value,
        category,
        condition,
        sellerName: name,
      });
      router.push(`/listing/${res.id}`);
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
                {c}
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
              {c}
            </option>
          ))}
        </select>
      </div>
      {err ? <p className="text-sm text-destructive">{err}</p> : null}
      <Button type="submit" disabled={busy} className="w-full">
        {busy ? "Posting…" : "Post listing"}
      </Button>
      <p className="text-center text-xs text-muted-foreground">
        Posting as <span className="font-medium">{name}</span> — buyers'
        offers land in <a href="/me" className="underline">My Market</a>.
      </p>
    </form>
  );
}

export function SellForm() {
  return (
    <MarketProvider>
      <Form />
    </MarketProvider>
  );
}
