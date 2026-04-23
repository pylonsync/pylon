/**
 * Pylon Forge — collaborative 3D scene editor.
 *
 * Spawn primitives with the toolbar, drag them on the grid to move,
 * click to select, keyboard shortcuts to color/delete. Every change
 * is a mutation; every other collaborator sees it live via the Prim
 * query. Cursor presence is a second live query on the Cursor table
 * at ~20 Hz.
 *
 * Hot-path design notes:
 *   - Local drag state is optimistic — we snap the mesh to the mouse
 *     immediately and only write movePrim every 100ms + on drag-end.
 *   - Cursor updates are throttled to 50ms.
 *   - The scene is rebuilt declaratively from the query on every
 *     frame's sync step; meshes are pooled by prim id.
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as THREE from "three";
import {
  init,
  db,
  callFn,
  configureClient,
  storageKey,
} from "@pylonsync/react";

const BASE_URL = "http://localhost:4321";
init({ baseUrl: BASE_URL, appName: "forge" });
configureClient({ baseUrl: BASE_URL, appName: "forge" });

const ROOM_ID = "main";
const CURSOR_COLORS = [
  "#8b5cf6", "#f5b946", "#7ab7ff", "#5ee6a6",
  "#ff6b9d", "#ffd166", "#80e0d8", "#c89dff",
];
const PRIM_COLORS = ["#8b5cf6", "#f5b946", "#7ab7ff", "#5ee6a6", "#ff6b9d", "#ffffff"];
const KINDS = [
  { id: "box", label: "Box" },
  { id: "sphere", label: "Sphere" },
  { id: "cone", label: "Cone" },
  { id: "torus", label: "Torus" },
] as const;

type Prim = {
  id: string;
  roomId: string;
  kind: "box" | "sphere" | "cone" | "torus";
  x: number; y: number; z: number;
  sx: number; sy: number; sz: number;
  color: string;
  createdBy: string;
  updatedAt: string;
};

type Cursor = {
  id: string;
  roomId: string;
  userId: string;
  name: string;
  color: string;
  x: number; y: number; z: number;
  updatedAt: string;
};

function uid() {
  return Math.random().toString(36).slice(2, 8);
}

function randomName() {
  const names = ["nova", "onyx", "echo", "lyra", "atlas", "rhea", "orion", "vega"];
  return `${names[Math.floor(Math.random() * names.length)]}_${Math.floor(Math.random() * 900 + 100)}`;
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

function hashColor(seed: string): string {
  let h = 0;
  for (let i = 0; i < seed.length; i++) h = (h * 31 + seed.charCodeAt(i)) | 0;
  return CURSOR_COLORS[Math.abs(h) % CURSOR_COLORS.length];
}

function geomFor(kind: string): THREE.BufferGeometry {
  switch (kind) {
    case "sphere": return new THREE.SphereGeometry(0.6, 32, 16);
    case "cone":   return new THREE.ConeGeometry(0.6, 1.2, 28);
    case "torus":  return new THREE.TorusGeometry(0.55, 0.22, 20, 48);
    default:       return new THREE.BoxGeometry(1, 1, 1);
  }
}

export function ForgeApp() {
  const [userId, setUserId] = useState<string | null>(null);
  const [myName] = useState(() => randomName());
  const [selected, setSelected] = useState<string | null>(null);
  const [presenceCount, setPresenceCount] = useState(0);

  const mountRef = useRef<HTMLDivElement | null>(null);
  const primsLatest = useRef<Prim[]>([]);
  const cursorsLatest = useRef<Cursor[]>([]);
  const selectedLatest = useRef<string | null>(null);
  const userIdLatest = useRef<string | null>(null);

  const { data: prims } = db.useQuery<Prim>("Prim", { where: { roomId: ROOM_ID } });
  const { data: cursors } = db.useQuery<Cursor>("Cursor", { where: { roomId: ROOM_ID } });

  useEffect(() => { primsLatest.current = prims ?? []; }, [prims]);
  useEffect(() => { cursorsLatest.current = cursors ?? []; }, [cursors]);
  useEffect(() => { selectedLatest.current = selected; }, [selected]);
  useEffect(() => { userIdLatest.current = userId; }, [userId]);

  useEffect(() => {
    const others = (cursors ?? []).filter((c) => c.userId !== userId);
    setPresenceCount(others.length);
  }, [cursors, userId]);

  // Auth.
  useEffect(() => {
    ensureGuest().then((id) => setUserId(id));
  }, []);

  // three.js scene setup + main loop.
  useEffect(() => {
    const mount = mountRef.current;
    if (!mount) return;

    // ---- Scene / camera / renderer ----
    const scene = new THREE.Scene();
    scene.background = new THREE.Color("#0d0d12");
    scene.fog = new THREE.Fog("#0d0d12", 40, 80);

    const camera = new THREE.PerspectiveCamera(55, 1, 0.1, 200);
    camera.position.set(10, 10, 14);

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
    scene.add(new THREE.HemisphereLight(0xaabbff, 0x202028, 0.5));
    const dir = new THREE.DirectionalLight(0xffffff, 1.1);
    dir.position.set(6, 14, 5);
    dir.castShadow = true;
    dir.shadow.mapSize.set(1024, 1024);
    dir.shadow.camera.left = -16; dir.shadow.camera.right = 16;
    dir.shadow.camera.top = 16;  dir.shadow.camera.bottom = -16;
    scene.add(dir);

    // ---- Ground + grid ----
    const ground = new THREE.Mesh(
      new THREE.PlaneGeometry(40, 40),
      new THREE.MeshStandardMaterial({ color: "#14141c", roughness: 0.9 }),
    );
    ground.rotation.x = -Math.PI / 2;
    ground.receiveShadow = true;
    ground.name = "ground";
    scene.add(ground);

    const grid = new THREE.GridHelper(40, 40, 0x2a2a3a, 0x1a1a25);
    scene.add(grid);

    // Primitives — pooled by id.
    type PrimEntry = {
      group: THREE.Group;
      mesh: THREE.Mesh;
      outline: THREE.Mesh;
      kind: string;
      color: string;
    };
    const primMeshes = new Map<string, PrimEntry>();

    // Cursors — DOM labels + tiny floating spheres.
    type CursorEntry = {
      sphere: THREE.Mesh;
      label: HTMLDivElement;
      targetX: number; targetY: number; targetZ: number;
    };
    const cursorMeshes = new Map<string, CursorEntry>();

    const cursorLayer = document.createElement("div");
    cursorLayer.className = "fg-cursors";
    mount.appendChild(cursorLayer);

    function outlineFor(mesh: THREE.Mesh): THREE.Mesh {
      const geom = mesh.geometry.clone();
      const mat = new THREE.MeshBasicMaterial({
        color: 0xffffff,
        transparent: true,
        opacity: 0.35,
        side: THREE.BackSide,
      });
      const outline = new THREE.Mesh(geom, mat);
      outline.scale.multiplyScalar(1.08);
      outline.visible = false;
      return outline;
    }

    function addPrim(p: Prim): PrimEntry {
      const group = new THREE.Group();
      const geom = geomFor(p.kind);
      const mat = new THREE.MeshStandardMaterial({
        color: p.color,
        roughness: 0.4,
        metalness: 0.08,
      });
      const mesh = new THREE.Mesh(geom, mat);
      mesh.castShadow = true;
      mesh.receiveShadow = true;
      mesh.userData.primId = p.id;
      group.add(mesh);

      const outline = outlineFor(mesh);
      group.add(outline);

      group.position.set(p.x, p.y, p.z);
      scene.add(group);
      return { group, mesh, outline, kind: p.kind, color: p.color };
    }

    function removePrim(id: string) {
      const e = primMeshes.get(id);
      if (!e) return;
      scene.remove(e.group);
      e.mesh.geometry.dispose();
      (e.mesh.material as THREE.Material).dispose();
      e.outline.geometry.dispose();
      (e.outline.material as THREE.Material).dispose();
      primMeshes.delete(id);
    }

    function addCursor(c: Cursor): CursorEntry {
      const sphere = new THREE.Mesh(
        new THREE.SphereGeometry(0.12, 12, 12),
        new THREE.MeshBasicMaterial({ color: c.color }),
      );
      sphere.position.set(c.x, c.y + 0.1, c.z);
      scene.add(sphere);

      const label = document.createElement("div");
      label.className = "fg-cursor-label";
      label.textContent = c.name;
      label.style.color = c.color;
      cursorLayer.appendChild(label);

      return {
        sphere, label,
        targetX: c.x, targetY: c.y, targetZ: c.z,
      };
    }

    function removeCursor(id: string) {
      const e = cursorMeshes.get(id);
      if (!e) return;
      scene.remove(e.sphere);
      e.sphere.geometry.dispose();
      (e.sphere.material as THREE.Material).dispose();
      e.label.remove();
      cursorMeshes.delete(id);
    }

    // ---- Input: orbit camera + drag-prim ----
    let camYaw = Math.atan2(camera.position.x, camera.position.z);
    let camPitch = Math.atan2(camera.position.y, Math.hypot(camera.position.x, camera.position.z));
    let camDist = Math.hypot(camera.position.x, camera.position.y, camera.position.z);

    function updateCamera() {
      camPitch = Math.max(0.1, Math.min(Math.PI / 2 - 0.05, camPitch));
      camera.position.set(
        Math.sin(camYaw) * Math.cos(camPitch) * camDist,
        Math.sin(camPitch) * camDist,
        Math.cos(camYaw) * Math.cos(camPitch) * camDist,
      );
      camera.lookAt(0, 0.5, 0);
    }
    updateCamera();

    const raycaster = new THREE.Raycaster();
    const ndc = new THREE.Vector2();
    let dragging: {
      primId: string;
      offsetX: number;
      offsetZ: number;
      lastSend: number;
    } | null = null;
    let orbiting: { x: number; y: number } | null = null;
    let lastCursorSend = 0;

    // Track hover point on ground for cursor presence + drag.
    const groundHit = new THREE.Vector3();

    function intersectGround(event: MouseEvent): THREE.Vector3 | null {
      const r = renderer.domElement.getBoundingClientRect();
      ndc.x = ((event.clientX - r.left) / r.width) * 2 - 1;
      ndc.y = -((event.clientY - r.top) / r.height) * 2 + 1;
      raycaster.setFromCamera(ndc, camera);
      const g = scene.getObjectByName("ground") as THREE.Mesh | undefined;
      if (!g) return null;
      const hits = raycaster.intersectObject(g);
      if (hits.length === 0) return null;
      return hits[0].point.clone();
    }

    function intersectPrim(event: MouseEvent): THREE.Intersection | null {
      const r = renderer.domElement.getBoundingClientRect();
      ndc.x = ((event.clientX - r.left) / r.width) * 2 - 1;
      ndc.y = -((event.clientY - r.top) / r.height) * 2 + 1;
      raycaster.setFromCamera(ndc, camera);
      const targets: THREE.Object3D[] = [];
      for (const [, e] of primMeshes) targets.push(e.mesh);
      const hits = raycaster.intersectObjects(targets, false);
      return hits[0] ?? null;
    }

    const onMouseDown = (event: MouseEvent) => {
      if (event.button === 2) {
        // Right button — orbit camera.
        orbiting = { x: event.clientX, y: event.clientY };
        return;
      }
      const hit = intersectPrim(event);
      if (hit) {
        const primId = hit.object.userData.primId as string;
        setSelected(primId);
        const gnd = intersectGround(event);
        if (gnd) {
          const p = primsLatest.current.find((p) => p.id === primId);
          if (p) {
            dragging = {
              primId,
              offsetX: p.x - gnd.x,
              offsetZ: p.z - gnd.z,
              lastSend: 0,
            };
          }
        }
      } else {
        setSelected(null);
      }
    };

    const onMouseMove = (event: MouseEvent) => {
      // Presence cursor update (regardless of drag state).
      const gnd = intersectGround(event);
      if (gnd) {
        groundHit.copy(gnd);
        const now = performance.now();
        if (userIdLatest.current && now - lastCursorSend > 50) {
          lastCursorSend = now;
          const color = hashColor(userIdLatest.current);
          callFn("updateCursor", {
            roomId: ROOM_ID,
            name: myName,
            color,
            x: gnd.x, y: 0, z: gnd.z,
          }).catch(() => {});
        }
      }

      if (dragging) {
        const g = intersectGround(event);
        if (!g) return;
        const nx = g.x + dragging.offsetX;
        const nz = g.z + dragging.offsetZ;
        const entry = primMeshes.get(dragging.primId);
        if (entry) {
          entry.group.position.x = nx;
          entry.group.position.z = nz;
        }
        const now = performance.now();
        if (now - dragging.lastSend > 100) {
          dragging.lastSend = now;
          const prim = primsLatest.current.find((p) => p.id === dragging!.primId);
          if (prim) {
            callFn("movePrim", {
              primId: dragging.primId,
              x: nx, y: prim.y, z: nz,
            }).catch(() => {});
          }
        }
      } else if (orbiting) {
        const dx = event.clientX - orbiting.x;
        const dy = event.clientY - orbiting.y;
        camYaw -= dx * 0.006;
        camPitch -= dy * 0.006;
        orbiting = { x: event.clientX, y: event.clientY };
        updateCamera();
      }
    };

    const onMouseUp = () => {
      if (dragging) {
        // Fire one final write with the final position.
        const entry = primMeshes.get(dragging.primId);
        const prim = primsLatest.current.find((p) => p.id === dragging!.primId);
        if (entry && prim) {
          callFn("movePrim", {
            primId: dragging.primId,
            x: entry.group.position.x,
            y: prim.y,
            z: entry.group.position.z,
          }).catch(() => {});
        }
      }
      dragging = null;
      orbiting = null;
    };

    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      camDist = Math.max(4, Math.min(40, camDist + event.deltaY * 0.02));
      updateCamera();
    };

    const onContextMenu = (e: MouseEvent) => e.preventDefault();

    renderer.domElement.addEventListener("mousedown", onMouseDown);
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    renderer.domElement.addEventListener("wheel", onWheel, { passive: false });
    renderer.domElement.addEventListener("contextmenu", onContextMenu);

    // Keyboard: Delete or Backspace removes selection; 1-6 cycle color.
    const onKeyDown = (e: KeyboardEvent) => {
      const tgt = e.target as HTMLElement;
      if (tgt && (tgt.tagName === "INPUT" || tgt.tagName === "TEXTAREA")) return;
      if (!selectedLatest.current) return;
      if (e.key === "Delete" || e.key === "Backspace") {
        callFn("deletePrim", { primId: selectedLatest.current }).catch(() => {});
      }
      const idx = Number(e.key) - 1;
      if (!Number.isNaN(idx) && idx >= 0 && idx < PRIM_COLORS.length) {
        callFn("colorPrim", {
          primId: selectedLatest.current,
          color: PRIM_COLORS[idx],
        }).catch(() => {});
      }
    };
    window.addEventListener("keydown", onKeyDown);

    // ---- Main loop ----
    let raf = 0;
    const tick = () => {
      // Sync prims.
      const seen = new Set<string>();
      for (const p of primsLatest.current) {
        seen.add(p.id);
        const existing = primMeshes.get(p.id);
        if (!existing) {
          primMeshes.set(p.id, addPrim(p));
        } else {
          // If kind changed (shouldn't normally) rebuild; otherwise
          // update position/color/outline.
          if (existing.kind !== p.kind) {
            existing.mesh.geometry.dispose();
            existing.mesh.geometry = geomFor(p.kind);
            existing.outline.geometry.dispose();
            existing.outline.geometry = existing.mesh.geometry.clone();
            existing.kind = p.kind;
          }
          if (existing.color !== p.color) {
            (existing.mesh.material as THREE.MeshStandardMaterial).color.set(p.color);
            existing.color = p.color;
          }
          // Don't override position while the user is dragging *this* prim.
          const isActivelyDragging = dragging && dragging.primId === p.id;
          if (!isActivelyDragging) {
            existing.group.position.set(p.x, p.y, p.z);
          }
          existing.outline.visible = selectedLatest.current === p.id;
        }
      }
      for (const id of Array.from(primMeshes.keys())) {
        if (!seen.has(id)) removePrim(id);
      }

      // Sync cursors (skip our own).
      const cseen = new Set<string>();
      for (const c of cursorsLatest.current) {
        if (c.userId === userIdLatest.current) continue;
        cseen.add(c.id);
        let e = cursorMeshes.get(c.id);
        if (!e) {
          e = addCursor(c);
          cursorMeshes.set(c.id, e);
        }
        e.targetX = c.x; e.targetY = c.y; e.targetZ = c.z;
        // Smooth toward target so presence doesn't jitter.
        e.sphere.position.lerp(
          new THREE.Vector3(e.targetX, e.targetY + 0.1, e.targetZ),
          0.25,
        );
      }
      for (const id of Array.from(cursorMeshes.keys())) {
        if (!cseen.has(id)) removeCursor(id);
      }

      // Project cursor labels to screen space.
      const rect = renderer.domElement.getBoundingClientRect();
      for (const [, e] of cursorMeshes) {
        const v = e.sphere.position.clone().project(camera);
        const sx = (v.x * 0.5 + 0.5) * rect.width;
        const sy = (-v.y * 0.5 + 0.5) * rect.height;
        const visible = v.z > 0 && v.z < 1;
        e.label.style.transform = `translate(${sx + 10}px, ${sy - 10}px)`;
        e.label.style.opacity = visible ? "1" : "0";
      }

      renderer.render(scene, camera);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      renderer.domElement.removeEventListener("mousedown", onMouseDown);
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
      renderer.domElement.removeEventListener("wheel", onWheel);
      renderer.domElement.removeEventListener("contextmenu", onContextMenu);
      window.removeEventListener("keydown", onKeyDown);
      for (const id of Array.from(primMeshes.keys())) removePrim(id);
      for (const id of Array.from(cursorMeshes.keys())) removeCursor(id);
      renderer.dispose();
      if (mount.contains(renderer.domElement)) mount.removeChild(renderer.domElement);
      if (mount.contains(cursorLayer)) mount.removeChild(cursorLayer);
    };
  }, [myName]);

  // ---- Toolbar actions ----
  const spawn = useCallback(async (kind: string) => {
    const x = (Math.random() - 0.5) * 6;
    const z = (Math.random() - 0.5) * 6;
    const color = PRIM_COLORS[Math.floor(Math.random() * PRIM_COLORS.length)];
    await callFn("spawnPrim", { roomId: ROOM_ID, kind, x, z, color }).catch(() => {});
  }, []);

  const clearAll = useCallback(async () => {
    const all = primsLatest.current;
    await Promise.all(all.map((p) => callFn("deletePrim", { primId: p.id }).catch(() => {})));
  }, []);

  const colorSelected = useCallback(async (color: string) => {
    if (!selected) return;
    await callFn("colorPrim", { primId: selected, color }).catch(() => {});
  }, [selected]);

  const selectedPrim = selected ? (prims ?? []).find((p) => p.id === selected) : null;

  return (
    <div className="fg">
      <div ref={mountRef} className="fg-canvas" />

      {/* Top toolbar: add primitives */}
      <div className="fg-top-toolbar">
        <div className="fg-brand">
          <svg viewBox="0 0 48 64" width="16" height="21" fill="currentColor">
            <path d="M24 2 L10 20 L24 32 Z" />
            <path d="M24 2 L38 20 L24 32 Z" />
            <path d="M24 32 L18 48 L24 62 L30 48 Z" />
            <path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
            <path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
          </svg>
          <span>Forge</span>
        </div>
        <div className="fg-toolgroup">
          {KINDS.map((k) => (
            <button key={k.id} className="fg-tool-btn" onClick={() => spawn(k.id)}>
              + {k.label}
            </button>
          ))}
        </div>
        <div className="fg-toolgroup fg-right">
          <span className="fg-presence">
            <span className="fg-presence-dot" />
            {presenceCount + 1} online
          </span>
          <button className="fg-tool-btn danger" onClick={clearAll}>Clear</button>
        </div>
      </div>

      {/* Selected-object inspector */}
      {selectedPrim && (
        <div className="fg-inspector">
          <div className="fg-insp-title">
            <span className="fg-insp-kind">{selectedPrim.kind}</span>
            <span className="fg-insp-id">#{selectedPrim.id.slice(0, 8)}</span>
          </div>
          <div className="fg-insp-row">
            <span className="fg-insp-label">POSITION</span>
            <span className="fg-insp-value">
              {selectedPrim.x.toFixed(1)}, {selectedPrim.y.toFixed(1)}, {selectedPrim.z.toFixed(1)}
            </span>
          </div>
          <div className="fg-insp-row">
            <span className="fg-insp-label">COLOR</span>
            <div className="fg-swatches">
              {PRIM_COLORS.map((c) => (
                <button
                  key={c}
                  className={`fg-swatch ${selectedPrim.color === c ? "on" : ""}`}
                  style={{ background: c }}
                  onClick={() => colorSelected(c)}
                  title={c}
                />
              ))}
            </div>
          </div>
          <div className="fg-insp-hint">
            Drag to move · <b>Del</b> to remove · <b>1-6</b> to recolor
          </div>
        </div>
      )}

      {/* Hint strip */}
      <div className="fg-hint-strip">
        <span><b>Left-click + drag</b> a primitive to move</span>
        <span><b>Right-click + drag</b> to orbit</span>
        <span><b>Scroll</b> to zoom</span>
      </div>
    </div>
  );
}
