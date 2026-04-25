/**
 * Pylon World3D — 3D multiplayer avatar world.
 *
 * Renders a three.js scene with a ground plane, grid, and one cube
 * per Avatar row. Your own avatar is controlled with WASD + mouse-
 * look (click to engage pointer lock). Other avatars are driven by
 * a live query that interpolates between server updates.
 *
 * Structure:
 *   - three.js setup in a single useEffect — scene, camera, renderer
 *   - Per-avatar meshes lifecycled from the useQuery result
 *   - Input handling + camera follow in the render loop
 *   - Position writes throttled to 10 Hz
 *   - Bot driver moves bots to random targets every few seconds
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as THREE from "three";
import {
  init,
  db,
  callFn,
  configureClient,
  storageKey,
} from "@pylonsync/react";
import { Button } from "@pylonsync/example-ui/button";

const BASE_URL = "http://localhost:4321";
init({ baseUrl: BASE_URL, appName: "world3d" });
configureClient({ baseUrl: BASE_URL, appName: "world3d" });

type Avatar = {
  id: string;
  userId: string;
  name: string;
  color: string;
  x: number;
  y: number;
  z: number;
  heading: number;
  emote?: string | null;
  isBot: boolean;
  lastSeenAt: string;
};

function uid() {
  return Math.random().toString(36).slice(2, 10);
}

async function ensureGuest(): Promise<string> {
  let token = localStorage.getItem(storageKey("token"));
  let userId = localStorage.getItem(storageKey("user"));
  if (!token || !userId) {
    const res = await fetch(`${BASE_URL}/api/auth/guest`, { method: "POST" });
    const body = await res.json();
    token = body.token as string;
    userId = body.user_id as string;
    localStorage.setItem(storageKey("token"), token);
    localStorage.setItem(storageKey("user"), userId);
  }
  return userId!;
}

function pct(values: number[], p: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const idx = Math.min(sorted.length - 1, Math.floor(sorted.length * p));
  return sorted[idx];
}

export function WorldApp() {
  const [userId, setUserId] = useState<string | null>(null);
  const [myId, setMyId] = useState<string | null>(null);
  const [hud, setHud] = useState({ avatars: 0, bots: 0, mutPerSec: 0, p50: 0, p95: 0 });
  const [spawning, setSpawning] = useState(false);
  const [pointerLocked, setPointerLocked] = useState(false);

  const mountRef = useRef<HTMLDivElement | null>(null);
  const avatarsLatest = useRef<Avatar[]>([]);
  const myIdLatest = useRef<string | null>(null);
  const latencies = useRef<number[]>([]);
  const mutRateWindow = useRef<number[]>([]);

  const { data: avatars } = db.useQuery<Avatar>("Avatar");

  // Keep latest refs in sync for the render loop (which closes over
  // them once — it'd be painful to rebuild the scene on every update).
  useEffect(() => { avatarsLatest.current = avatars ?? []; }, [avatars]);
  useEffect(() => { myIdLatest.current = myId; }, [myId]);

  // HUD ticker.
  useEffect(() => {
    const t = setInterval(() => {
      const now = Date.now();
      mutRateWindow.current = mutRateWindow.current.filter((ts) => now - ts < 1000);
      const last100 = latencies.current.slice(-100);
      const all = avatars ?? [];
      setHud({
        avatars: all.length,
        bots: all.filter((a) => a.isBot).length,
        mutPerSec: mutRateWindow.current.length,
        p50: Math.round(pct(last100, 0.5)),
        p95: Math.round(pct(last100, 0.95)),
      });
    }, 500);
    return () => clearInterval(t);
  }, [avatars]);

  // One-time: auth + spawn.
  useEffect(() => {
    let cancelled = false;
    ensureGuest().then(async (id) => {
      if (cancelled) return;
      setUserId(id);
      try {
        const r = await callFn<{ id: string }>("spawnAvatar", { userId: id });
        setMyId(r.id);
      } catch (e) {
        console.error("spawn failed", e);
      }
    });
    return () => { cancelled = true; };
  }, []);

  // Three.js scene + game loop.
  useEffect(() => {
    const mount = mountRef.current;
    if (!mount) return;

    // ---- Scene, camera, renderer ----
    const scene = new THREE.Scene();
    scene.background = new THREE.Color("#0a0a0c");
    scene.fog = new THREE.Fog("#0a0a0c", 25, 60);

    const camera = new THREE.PerspectiveCamera(60, 1, 0.1, 200);
    camera.position.set(0, 6, 10);

    const renderer = new THREE.WebGLRenderer({ antialias: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.shadowMap.enabled = true;
    renderer.shadowMap.type = THREE.PCFSoftShadowMap;
    mount.appendChild(renderer.domElement);

    const resize = () => {
      const r = mount.getBoundingClientRect();
      renderer.setSize(r.width, r.height, false);
      camera.aspect = r.width / r.height;
      camera.updateProjectionMatrix();
    };
    resize();
    const ro = new ResizeObserver(resize);
    ro.observe(mount);

    // ---- Lights ----
    const hemi = new THREE.HemisphereLight(0xaaaaff, 0x1a1a2a, 0.45);
    scene.add(hemi);
    const dir = new THREE.DirectionalLight(0xffffff, 1.2);
    dir.position.set(6, 14, 6);
    dir.castShadow = true;
    dir.shadow.mapSize.set(1024, 1024);
    dir.shadow.camera.left = -24;
    dir.shadow.camera.right = 24;
    dir.shadow.camera.top = 24;
    dir.shadow.camera.bottom = -24;
    scene.add(dir);

    // ---- Ground + grid ----
    const ground = new THREE.Mesh(
      new THREE.PlaneGeometry(80, 80),
      new THREE.MeshStandardMaterial({ color: "#121218", roughness: 0.95 }),
    );
    ground.rotation.x = -Math.PI / 2;
    ground.receiveShadow = true;
    scene.add(ground);

    const grid = new THREE.GridHelper(80, 80, 0x262638, 0x1a1a24);
    scene.add(grid);

    // Accent ring at origin — helps orient new players.
    const ring = new THREE.Mesh(
      new THREE.RingGeometry(0.9, 1.1, 48),
      new THREE.MeshBasicMaterial({ color: "#8b5cf6", transparent: true, opacity: 0.6, side: THREE.DoubleSide }),
    );
    ring.rotation.x = -Math.PI / 2;
    ring.position.y = 0.02;
    scene.add(ring);

    // ---- Avatar meshes managed per-id ----
    type MeshEntry = {
      group: THREE.Group;
      body: THREE.Mesh;
      label: HTMLDivElement;
      // Smoothed pose for interpolation.
      curX: number; curY: number; curZ: number; curHead: number;
    };
    const meshes = new Map<string, MeshEntry>();

    const labelLayer = document.createElement("div");
    labelLayer.className = "w3d-labels";
    mount.appendChild(labelLayer);

    function addAvatarMesh(a: Avatar): MeshEntry {
      const group = new THREE.Group();
      const geom = new THREE.BoxGeometry(0.9, 1.4, 0.9);
      const mat = new THREE.MeshStandardMaterial({
        color: a.color,
        roughness: 0.4,
        metalness: 0.1,
      });
      const body = new THREE.Mesh(geom, mat);
      body.castShadow = true;
      body.position.y = 0.7;
      group.add(body);

      // Eye indicator — shows facing direction.
      const eye = new THREE.Mesh(
        new THREE.SphereGeometry(0.08, 12, 12),
        new THREE.MeshBasicMaterial({ color: "#ffffff" }),
      );
      eye.position.set(0, 1.0, 0.45);
      group.add(eye);

      group.position.set(a.x, 0, a.z);
      group.rotation.y = a.heading;
      scene.add(group);

      const label = document.createElement("div");
      label.className = "w3d-label";
      label.textContent = a.name + (a.isBot ? " •" : "");
      label.style.color = a.color;
      labelLayer.appendChild(label);

      return { group, body, label, curX: a.x, curY: a.y, curZ: a.z, curHead: a.heading };
    }

    function removeAvatarMesh(id: string) {
      const m = meshes.get(id);
      if (!m) return;
      scene.remove(m.group);
      m.group.traverse((o) => {
        if ((o as THREE.Mesh).geometry) (o as THREE.Mesh).geometry.dispose();
        const mat = (o as THREE.Mesh).material as THREE.Material | THREE.Material[];
        if (Array.isArray(mat)) mat.forEach((mm) => mm.dispose());
        else if (mat) mat.dispose();
      });
      m.label.remove();
      meshes.delete(id);
    }

    // ---- Input ----
    const keys = new Set<string>();
    const onKeyDown = (e: KeyboardEvent) => {
      keys.add(e.key.toLowerCase());
    };
    const onKeyUp = (e: KeyboardEvent) => {
      keys.delete(e.key.toLowerCase());
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);

    let mouseYaw = 0;   // camera yaw offset (added to avatar heading)
    let mousePitch = 0.25;
    const onMouseMove = (e: MouseEvent) => {
      if (document.pointerLockElement !== renderer.domElement) return;
      mouseYaw -= e.movementX * 0.002;
      mousePitch = Math.max(-0.6, Math.min(0.8, mousePitch + e.movementY * 0.002));
    };
    document.addEventListener("mousemove", onMouseMove);

    const onCanvasClick = () => {
      if (document.pointerLockElement !== renderer.domElement) {
        renderer.domElement.requestPointerLock().catch(() => {});
      }
    };
    renderer.domElement.addEventListener("click", onCanvasClick);

    const onLockChange = () => {
      setPointerLocked(document.pointerLockElement === renderer.domElement);
    };
    document.addEventListener("pointerlockchange", onLockChange);

    // ---- Main loop ----
    let raf = 0;
    let last = performance.now();
    let lastSend = 0;
    const moveSpeed = 6;     // world units per second
    const turnSpeed = 2.2;   // rad/s from keyboard
    let myHeading = 0;

    const tick = (t: number) => {
      const dt = Math.min(0.05, (t - last) / 1000);
      last = t;

      const all = avatarsLatest.current;
      const selfId = myIdLatest.current;

      // Sync meshes with avatars.
      const seen = new Set<string>();
      for (const a of all) {
        seen.add(a.id);
        let m = meshes.get(a.id);
        if (!m) m = meshes.get(a.id) ?? addAvatarMesh(a);
        meshes.set(a.id, m);
      }
      for (const id of Array.from(meshes.keys())) {
        if (!seen.has(id)) removeAvatarMesh(id);
      }

      // For non-self avatars, smoothly interpolate toward server pose.
      for (const a of all) {
        const m = meshes.get(a.id);
        if (!m) continue;
        if (a.id === selfId) continue; // self is driven by input
        const lerp = 1 - Math.exp(-dt * 8);
        m.curX += (a.x - m.curX) * lerp;
        m.curY += (a.y - m.curY) * lerp;
        m.curZ += (a.z - m.curZ) * lerp;
        // Shortest-path heading interp.
        let dh = a.heading - m.curHead;
        while (dh > Math.PI) dh -= Math.PI * 2;
        while (dh < -Math.PI) dh += Math.PI * 2;
        m.curHead += dh * lerp;
      }

      // Self movement: WASD relative to camera yaw.
      // Camera sits at (avatar - sin(yaw)*dist, _, avatar - cos(yaw)*dist)
      // looking at the avatar, so "forward" in world is (sin yaw, cos yaw).
      let forward = 0, right = 0;
      if (keys.has("w") || keys.has("arrowup")) forward += 1;
      if (keys.has("s") || keys.has("arrowdown")) forward -= 1;
      if (keys.has("d") || keys.has("arrowright")) right += 1;
      if (keys.has("a") || keys.has("arrowleft")) right -= 1;
      const mag = Math.hypot(forward, right);
      if (mag > 0) { forward /= mag; right /= mag; }

      const rx = Math.sin(mouseYaw) * forward - Math.cos(mouseYaw) * right;
      const rz = Math.cos(mouseYaw) * forward + Math.sin(mouseYaw) * right;

      // Update self heading toward movement direction.
      const selfEntry = selfId ? meshes.get(selfId) : null;
      if (selfEntry) {
        if (mag > 0.01) {
          const target = Math.atan2(rx, rz);
          let dh = target - selfEntry.curHead;
          while (dh > Math.PI) dh -= Math.PI * 2;
          while (dh < -Math.PI) dh += Math.PI * 2;
          selfEntry.curHead += dh * Math.min(1, turnSpeed * dt);
          myHeading = selfEntry.curHead;
        }
        selfEntry.curX += rx * moveSpeed * dt;
        selfEntry.curZ += rz * moveSpeed * dt;
        selfEntry.curX = Math.max(-20, Math.min(20, selfEntry.curX));
        selfEntry.curZ = Math.max(-20, Math.min(20, selfEntry.curZ));
      }

      // Apply transforms to all meshes + update labels.
      for (const [, m] of meshes) {
        m.group.position.set(m.curX, m.curY, m.curZ);
        m.group.rotation.y = m.curHead;
      }

      // Camera — third-person trailing behind self avatar.
      if (selfEntry) {
        const dist = 6.5;
        const height = 3.5;
        const yaw = mouseYaw;
        const camX = selfEntry.curX - Math.sin(yaw) * dist;
        const camZ = selfEntry.curZ - Math.cos(yaw) * dist;
        const camY = height + mousePitch * 3;
        camera.position.lerp(new THREE.Vector3(camX, camY, camZ), 1 - Math.exp(-dt * 10));
        camera.lookAt(selfEntry.curX, 1.2, selfEntry.curZ);
      }

      // Position labels in screen space.
      const rect = renderer.domElement.getBoundingClientRect();
      for (const [, m] of meshes) {
        const v = new THREE.Vector3(m.curX, 2.0, m.curZ).project(camera);
        const sx = (v.x * 0.5 + 0.5) * rect.width;
        const sy = (-v.y * 0.5 + 0.5) * rect.height;
        const visible = v.z > 0 && v.z < 1;
        m.label.style.transform = `translate(${sx}px, ${sy}px)`;
        m.label.style.opacity = visible ? "1" : "0";
      }

      // Throttle pose uploads to ~10 Hz.
      if (selfId && t - lastSend > 100) {
        const s = meshes.get(selfId);
        if (s) {
          const t0 = performance.now();
          callFn("moveAvatar", {
            avatarId: selfId,
            x: s.curX,
            y: 0,
            z: s.curZ,
            heading: s.curHead,
          })
            .then(() => {
              latencies.current.push(performance.now() - t0);
              if (latencies.current.length > 200) latencies.current.shift();
              mutRateWindow.current.push(Date.now());
            })
            .catch(() => {});
        }
        lastSend = t;
      }

      renderer.render(scene, camera);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
      document.removeEventListener("mousemove", onMouseMove);
      renderer.domElement.removeEventListener("click", onCanvasClick);
      document.removeEventListener("pointerlockchange", onLockChange);
      ro.disconnect();
      for (const id of Array.from(meshes.keys())) removeAvatarMesh(id);
      renderer.dispose();
      if (mount.contains(renderer.domElement)) mount.removeChild(renderer.domElement);
      if (mount.contains(labelLayer)) mount.removeChild(labelLayer);
    };
  }, []);

  // Bot spawner — drops N bot avatars scattered across the plane.
  async function spawnBots(count: number) {
    setSpawning(true);
    try {
      const tasks: Promise<unknown>[] = [];
      for (let i = 0; i < count; i++) {
        tasks.push(
          callFn("spawnAvatar", {
            userId: `bot_${uid()}_${i}`,
            isBot: true,
          }),
        );
      }
      await Promise.all(tasks);
    } finally {
      setSpawning(false);
    }
  }

  async function clearBots() {
    await callFn("clearBots", {}).catch(() => {});
  }

  // Bot brains: pick random walk targets, ease toward them. Runs in
  // whichever tab spawned the bots.
  useEffect(() => {
    if (!avatars) return;
    const t = setInterval(() => {
      const bots = avatars.filter((a) => a.isBot && Math.random() < 0.12);
      for (const b of bots.slice(0, 8)) {
        const nx = b.x + (Math.random() - 0.5) * 4;
        const nz = b.z + (Math.random() - 0.5) * 4;
        const heading = Math.atan2(nx - b.x, nz - b.z);
        callFn("moveAvatar", {
          avatarId: b.id,
          x: nx, y: 0, z: nz,
          heading,
        }).catch(() => {});
      }
    }, 1400);
    return () => clearInterval(t);
  }, [avatars]);

  return (
    <div className="fixed inset-0 bg-[#0a0a0c]">
      <div ref={mountRef} className="absolute inset-0" />

      <div className="absolute left-4 top-4 flex flex-col gap-2 rounded-lg border bg-card/85 p-4 backdrop-blur-sm">
        <HudRow label="AVATARS" value={hud.avatars.toLocaleString()} subtle={hud.bots > 0 ? `${hud.bots} bot` : undefined} />
        <HudRow label="MUT/S" value={hud.mutPerSec.toString()} />
        <HudRow label="P50" value={`${hud.p50}`} unit="ms" />
        <HudRow label="P95" value={`${hud.p95}`} unit="ms" />
      </div>

      <div className="absolute right-4 top-4 flex w-56 flex-col gap-3 rounded-lg border bg-card/85 p-4 backdrop-blur-sm">
        <div className="text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
          Stress test
        </div>
        <div className="flex gap-1.5">
          <Button size="xs" variant="outline" disabled={spawning} onClick={() => spawnBots(10)} className="flex-1">+10</Button>
          <Button size="xs" variant="outline" disabled={spawning} onClick={() => spawnBots(50)} className="flex-1">+50</Button>
          <Button size="xs" variant="outline" disabled={spawning} onClick={() => spawnBots(200)} className="flex-1">+200</Button>
        </div>
        <Button size="xs" variant="ghost" onClick={clearBots} className="text-destructive hover:bg-destructive/10 hover:text-destructive">
          Clear bots
        </Button>
        <p className="text-xs text-muted-foreground">
          <strong className="text-foreground">WASD</strong> to move · <strong className="text-foreground">click</strong> for mouse-look
        </p>
        {!pointerLocked && (
          <p className="text-xs text-primary">Click the scene to engage controls.</p>
        )}
      </div>

      <div className="absolute bottom-4 left-4 flex items-center gap-2 font-mono text-xs text-muted-foreground">
        <svg viewBox="0 0 48 64" width="14" height="18" fill="currentColor" aria-hidden>
          <path d="M24 2 L10 20 L24 32 Z" />
          <path d="M24 2 L38 20 L24 32 Z" />
          <path d="M24 32 L18 48 L24 62 L30 48 Z" />
          <path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
          <path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
        </svg>
        <span>Pylon · World3D</span>
      </div>
    </div>
  );
}

function HudRow({
  label,
  value,
  subtle,
  unit,
}: {
  label: string;
  value: string;
  subtle?: string;
  unit?: string;
}) {
  return (
    <div className="flex items-baseline gap-2">
      <span className="w-14 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
        {label}
      </span>
      <span className="font-mono text-base tabular-nums">
        {value}
        {unit && <span className="ml-0.5 text-xs text-muted-foreground">{unit}</span>}
      </span>
      {subtle && <span className="text-xs text-muted-foreground">· {subtle}</span>}
    </div>
  );
}
