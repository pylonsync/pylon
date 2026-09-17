"use client";

import React, { useEffect, useRef, useState } from "react";
import { callFn, uploadFile, useRouter } from "@pylonsync/react";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { BIO_MAX, DISPLAY_NAME_MAX, USERNAME_MAX, normalizeUsername, usernameProblem, type Profile } from "@/lib/social";
import { Avatar } from "./avatar";

const field = "w-full rounded-lg border bg-transparent px-3 py-2 text-sm outline-none focus:border-zinc-400";

/** Photo, username, name, and bio. The server checks the username again. */
export function EditProfileDialog({ profile, open, onOpenChange }: { profile: Profile; open: boolean; onOpenChange: (open: boolean) => void }) {
  const router = useRouter();
  const input = useRef<HTMLInputElement | null>(null);
  const [username, setUsername] = useState(profile.username);
  const [displayName, setDisplayName] = useState(profile.displayName);
  const [bio, setBio] = useState(profile.bio ?? "");
  const [avatarUrl, setAvatarUrl] = useState(profile.avatarUrl ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setUsername(profile.username);
    setDisplayName(profile.displayName);
    setBio(profile.bio ?? "");
    setAvatarUrl(profile.avatarUrl ?? "");
    setError(null);
  }, [open, profile]);

  async function pickPhoto(file: File | undefined) {
    if (!file) return;
    if (!file.type.startsWith("image/")) return setError("Choose a photo.");
    setBusy(true);
    setError(null);
    try {
      const photo = await uploadFile(file, { visibility: "public" });
      setAvatarUrl(photo.url);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Could not upload that photo.");
    } finally {
      setBusy(false);
    }
  }

  async function save(e: React.FormEvent) {
    e.preventDefault();
    const next = normalizeUsername(username);
    const problem = usernameProblem(next);
    if (problem) return setError(problem);
    setBusy(true);
    setError(null);
    try {
      await callFn("updateProfile", { username: next, displayName, bio, avatarUrl });
      onOpenChange(false);
      if (next !== profile.username) router.replace(`/u/${next}`);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Could not save your profile.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(next) => (busy ? null : onOpenChange(next))}>
      <DialogContent className="max-w-md">
        <DialogTitle className="text-base font-semibold">Edit profile</DialogTitle>
        <form onSubmit={save} className="grid gap-4">
          <div className="flex items-center gap-4 rounded-xl bg-zinc-100 p-3">
            <Avatar profile={{ ...profile, avatarUrl }} size={56} />
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm font-semibold">{profile.username}</p>
              <p className="truncate text-sm text-zinc-500">{profile.displayName}</p>
            </div>
            <input ref={input} type="file" accept="image/*" className="sr-only" onChange={(e) => void pickPhoto(e.target.files?.[0])} />
            <button type="button" disabled={busy} onClick={() => input.current?.click()} className="rounded-lg bg-brand px-3 py-1.5 text-sm font-semibold text-white disabled:opacity-50">
              Change photo
            </button>
          </div>
          <label className="grid gap-1.5 text-sm">
            <span className="font-semibold">Username</span>
            <input value={username} onChange={(e) => setUsername(e.target.value)} maxLength={USERNAME_MAX + 1} className={field} autoCapitalize="none" autoCorrect="off" />
          </label>
          <label className="grid gap-1.5 text-sm">
            <span className="font-semibold">Name</span>
            <input value={displayName} onChange={(e) => setDisplayName(e.target.value)} maxLength={DISPLAY_NAME_MAX} className={field} />
          </label>
          <label className="grid gap-1.5 text-sm">
            <span className="font-semibold">Bio</span>
            <textarea value={bio} onChange={(e) => setBio(e.target.value)} maxLength={BIO_MAX} rows={3} className={`${field} resize-none`} />
            <span className="text-right text-xs text-zinc-400">
              {bio.length}/{BIO_MAX}
            </span>
          </label>
          {error ? <p className="text-sm text-red-600">{error}</p> : null}
          <button type="submit" disabled={busy} className="h-10 rounded-lg bg-brand text-sm font-semibold text-white disabled:opacity-50">
            {busy ? "Saving…" : "Save"}
          </button>
        </form>
      </DialogContent>
    </Dialog>
  );
}
