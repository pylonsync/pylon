"use client";

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

import { useCallback, useEffect, useRef, useState } from "react";
import * as THREE from "three";
import {
  db,
  callFn,
} from "@pylonsync/react";
import {
  Box,
  Brush,
  Circle,
  Cone,
  Move,
  Mountain,
  PaintBucket,
  Sparkles,
  Torus,
  Trash2,
  X,
} from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { Card } from "@pylonsync/example-ui/card";
import { Badge } from "@pylonsync/example-ui/badge";
import { cn } from "@pylonsync/example-ui/utils";

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

type PrimRow = {
  id: string;
  roomId: string;
  kind: "box" | "sphere" | "cone" | "torus";
  x: number; y: number; z: number;
  sx: number; sy: number; sz: number;
  color: string;
  createdBy: string;
  updatedAt: string;
};

type CursorRow = {
  id: string;
  roomId: string;
  userId: string;
  name: string;
  color: string;
  x: number; y: number; z: number;
  updatedAt: string;
};

type TerrainRow = {
  id: string;
  roomId: string;
  size: number;
  heights: string;  // JSON number[][]
  layers: string;   // JSON number[][][]
  updatedAt: string;
};

const TERRAIN_WORLD_SIZE = 40;
const TERRAIN_HEIGHT_SCALE = 1.0;

const LAYER_COLORS: [THREE.Color, THREE.Color, THREE.Color, THREE.Color] = [
  new THREE.Color("#4a6b3f"), // grass
  new THREE.Color("#6b4f3a"), // dirt
  new THREE.Color("#6a6a74"), // rock
  new THREE.Color("#e8e8f0"), // snow
];
const LAYER_LABELS = ["Grass", "Dirt", "Rock", "Snow"] as const;

type Tool = "orbit" | "raise" | "lower" | "smooth" | "flatten" | "paint";

function buildTerrainMaterial(): THREE.MeshStandardMaterial {
  const mat = new THREE.MeshStandardMaterial({
    color: 0xffffff,
    roughness: 0.95,
    metalness: 0.0,
  });
  mat.onBeforeCompile = (shader) => {
    shader.uniforms.layerColor0 = { value: LAYER_COLORS[0] };
    shader.uniforms.layerColor1 = { value: LAYER_COLORS[1] };
    shader.uniforms.layerColor2 = { value: LAYER_COLORS[2] };
    shader.uniforms.layerColor3 = { value: LAYER_COLORS[3] };

    shader.vertexShader = shader.vertexShader
      .replace(
        "#include <common>",
        `#include <common>
         attribute float layerW0;
         attribute float layerW1;
         attribute float layerW2;
         attribute float layerW3;
         varying vec4 vLayerW;`,
      )
      .replace(
        "#include <begin_vertex>",
        `#include <begin_vertex>
         vLayerW = vec4(layerW0, layerW1, layerW2, layerW3);`,
      );

    shader.fragmentShader = shader.fragmentShader
      .replace(
        "#include <common>",
        `#include <common>
         uniform vec3 layerColor0;
         uniform vec3 layerColor1;
         uniform vec3 layerColor2;
         uniform vec3 layerColor3;
         varying vec4 vLayerW;`,
      )
      .replace(
        "vec4 diffuseColor = vec4( diffuse, opacity );",
        `vec3 blended =
           layerColor0 * vLayerW.x +
           layerColor1 * vLayerW.y +
           layerColor2 * vLayerW.z +
           layerColor3 * vLayerW.w;
         vec4 diffuseColor = vec4( blended, opacity );`,
      );
  };
  return mat;
}

function worldToCell(wx: number, wz: number, size: number): { cx: number; cz: number } {
  const cx = ((wx / TERRAIN_WORLD_SIZE) + 0.5) * (size - 1);
  const cz = ((wz / TERRAIN_WORLD_SIZE) + 0.5) * (size - 1);
  return { cx, cz };
}

function randomName() {
  const names = ["nova", "onyx", "echo", "lyra", "atlas", "rhea", "orion", "vega"];
  return `${names[Math.floor(Math.random() * names.length)]}_${Math.floor(Math.random() * 900 + 100)}`;
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

export function ForgeApp({ userId, myName }: { userId: string; myName: string }) {
  const [selected, setSelected] = useState<string | null>(null);
  const [presenceCount, setPresenceCount] = useState(0);
  const [spawning, setSpawning] = useState(false);

  const [tool, setTool] = useState<Tool>("orbit");
  const [paintLayer, setPaintLayer] = useState<0 | 1 | 2 | 3>(0);
  const [brushRadius, setBrushRadius] = useState(3);
  const [brushStrength, setBrushStrength] = useState(0.5);

  const mountRef = useRef<HTMLDivElement | null>(null);
  const primsLatest = useRef<PrimRow[]>([]);
  const cursorsLatest = useRef<CursorRow[]>([]);
  const terrainLatest = useRef<TerrainRow | null>(null);
  const selectedLatest = useRef<string | null>(null);
  const toolLatest = useRef<Tool>("orbit");
  const paintLayerLatest = useRef<0 | 1 | 2 | 3>(0);
  const brushRadiusLatest = useRef(3);
  const brushStrengthLatest = useRef(0.5);

  const { data: prims } = db.useQuery<PrimRow>("Prim", { where: { roomId: ROOM_ID } });
  const { data: cursors } = db.useQuery<CursorRow>("Cursor", { where: { roomId: ROOM_ID } });
  const { data: terrainRows } = db.useQuery<TerrainRow>("Terrain", { where: { roomId: ROOM_ID } });

  useEffect(() => { primsLatest.current = prims ?? []; }, [prims]);
  useEffect(() => { cursorsLatest.current = cursors ?? []; }, [cursors]);
  useEffect(() => { terrainLatest.current = (terrainRows ?? [])[0] ?? null; }, [terrainRows]);
  useEffect(() => { selectedLatest.current = selected; }, [selected]);
  useEffect(() => { toolLatest.current = tool; }, [tool]);
  useEffect(() => { paintLayerLatest.current = paintLayer; }, [paintLayer]);
  useEffect(() => { brushRadiusLatest.current = brushRadius; }, [brushRadius]);
  useEffect(() => { brushStrengthLatest.current = brushStrength; }, [brushStrength]);

  // Init terrain once on mount.
  useEffect(() => {
    callFn("initTerrain", { roomId: ROOM_ID, size: 64 }).catch((e: unknown) => {
      console.error("[forge] initTerrain failed:", e);
    });
  }, []);

  useEffect(() => {
    const others = (cursors ?? []).filter((c) => c.userId !== userId);
    setPresenceCount(others.length);
  }, [cursors, userId]);

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

    // ---- Terrain ----
    const terrainMat = buildTerrainMaterial();
    const terrain = new THREE.Mesh(
      new THREE.PlaneGeometry(TERRAIN_WORLD_SIZE, TERRAIN_WORLD_SIZE, 1, 1),
      terrainMat,
    );
    terrain.rotation.x = -Math.PI / 2;
    terrain.receiveShadow = true;
    terrain.name = "ground";
    scene.add(terrain);

    const brushCursor = new THREE.Mesh(
      new THREE.RingGeometry(0.85, 1.0, 32),
      new THREE.MeshBasicMaterial({
        color: "#a78bfa",
        transparent: true,
        opacity: 0.75,
        side: THREE.DoubleSide,
        depthTest: false,
      }),
    );
    brushCursor.rotation.x = -Math.PI / 2;
    brushCursor.visible = false;
    brushCursor.renderOrder = 999;
    scene.add(brushCursor);

    let terrainSizeCache = 0;
    let localHeights: number[][] | null = null;
    let localLayers: number[][][] | null = null;

    function rebuildTerrainGeometry(t: TerrainRow) {
      const size = t.size;
      const heights = JSON.parse(t.heights) as number[][];
      const layers = JSON.parse(t.layers) as number[][][];
      localHeights = heights;
      localLayers = layers;

      if (terrainSizeCache !== size) {
        const geom = new THREE.PlaneGeometry(
          TERRAIN_WORLD_SIZE,
          TERRAIN_WORLD_SIZE,
          size - 1,
          size - 1,
        );
        const count = geom.attributes.position.count;
        const w0 = new Float32Array(count);
        const w1 = new Float32Array(count);
        const w2 = new Float32Array(count);
        const w3 = new Float32Array(count);
        geom.setAttribute("layerW0", new THREE.BufferAttribute(w0, 1));
        geom.setAttribute("layerW1", new THREE.BufferAttribute(w1, 1));
        geom.setAttribute("layerW2", new THREE.BufferAttribute(w2, 1));
        geom.setAttribute("layerW3", new THREE.BufferAttribute(w3, 1));
        terrain.geometry.dispose();
        terrain.geometry = geom;
        terrainSizeCache = size;
      }

      const geom = terrain.geometry as THREE.PlaneGeometry;
      const pos = geom.attributes.position as THREE.BufferAttribute;
      const w0 = geom.attributes.layerW0 as THREE.BufferAttribute;
      const w1 = geom.attributes.layerW1 as THREE.BufferAttribute;
      const w2 = geom.attributes.layerW2 as THREE.BufferAttribute;
      const w3 = geom.attributes.layerW3 as THREE.BufferAttribute;

      for (let z = 0; z < size; z++) {
        for (let x = 0; x < size; x++) {
          const idx = z * size + x;
          pos.setZ(idx, (heights[z]?.[x] ?? 0) * TERRAIN_HEIGHT_SCALE);
          const l = layers[z]?.[x] ?? [1, 0, 0, 0];
          w0.setX(idx, l[0]);
          w1.setX(idx, l[1]);
          w2.setX(idx, l[2]);
          w3.setX(idx, l[3]);
        }
      }
      pos.needsUpdate = true;
      w0.needsUpdate = true; w1.needsUpdate = true;
      w2.needsUpdate = true; w3.needsUpdate = true;
      geom.computeVertexNormals();
    }

    function sculptLocal(
      cx: number, cz: number, radius: number, strength: number,
      mode: "raise" | "lower" | "smooth" | "flatten",
      targetY: number = 0,
    ) {
      if (!localHeights || terrainSizeCache === 0) return;
      const size = terrainSizeCache;
      const r = Math.max(1, radius);
      const xMin = Math.max(0, Math.floor(cx - r));
      const xMax = Math.min(size - 1, Math.ceil(cx + r));
      const zMin = Math.max(0, Math.floor(cz - r));
      const zMax = Math.min(size - 1, Math.ceil(cz + r));

      let avg = 0, count = 0;
      if (mode === "smooth") {
        for (let z = zMin; z <= zMax; z++) {
          for (let x = xMin; x <= xMax; x++) {
            if (Math.hypot(x - cx, z - cz) <= r) {
              avg += localHeights[z][x];
              count++;
            }
          }
        }
        if (count > 0) avg /= count;
      }

      const geom = terrain.geometry as THREE.PlaneGeometry;
      const pos = geom.attributes.position as THREE.BufferAttribute;

      for (let z = zMin; z <= zMax; z++) {
        for (let x = xMin; x <= xMax; x++) {
          const d = Math.hypot(x - cx, z - cz);
          if (d > r) continue;
          const falloff = 0.5 * (1 + Math.cos((Math.PI * d) / r));
          const s = strength * falloff;
          let h = localHeights[z][x];
          switch (mode) {
            case "raise":   h += s; break;
            case "lower":   h -= s; break;
            case "smooth":  h += (avg - h) * Math.min(1, s); break;
            case "flatten": h += (targetY - h) * Math.min(1, s); break;
          }
          localHeights[z][x] = h;
          pos.setZ(z * size + x, h * TERRAIN_HEIGHT_SCALE);
        }
      }
      pos.needsUpdate = true;
      geom.computeVertexNormals();
    }

    function paintLocal(
      cx: number, cz: number, radius: number, strength: number,
      layer: 0 | 1 | 2 | 3,
    ) {
      if (!localLayers || terrainSizeCache === 0) return;
      const size = terrainSizeCache;
      const r = Math.max(1, radius);
      const xMin = Math.max(0, Math.floor(cx - r));
      const xMax = Math.min(size - 1, Math.ceil(cx + r));
      const zMin = Math.max(0, Math.floor(cz - r));
      const zMax = Math.min(size - 1, Math.ceil(cz + r));

      const geom = terrain.geometry as THREE.PlaneGeometry;
      const attrs = [
        geom.attributes.layerW0 as THREE.BufferAttribute,
        geom.attributes.layerW1 as THREE.BufferAttribute,
        geom.attributes.layerW2 as THREE.BufferAttribute,
        geom.attributes.layerW3 as THREE.BufferAttribute,
      ];

      for (let z = zMin; z <= zMax; z++) {
        for (let x = xMin; x <= xMax; x++) {
          const d = Math.hypot(x - cx, z - cz);
          if (d > r) continue;
          const falloff = 0.5 * (1 + Math.cos((Math.PI * d) / r));
          const add = strength * falloff;
          const cell = localLayers[z][x];
          cell[layer] = Math.min(1, cell[layer] + add);
          const sum = cell[0] + cell[1] + cell[2] + cell[3];
          if (sum > 0) {
            cell[0] /= sum; cell[1] /= sum;
            cell[2] /= sum; cell[3] /= sum;
          }
          const idx = z * size + x;
          for (let k = 0; k < 4; k++) attrs[k].setX(idx, cell[k]);
        }
      }
      for (const a of attrs) a.needsUpdate = true;
    }

    const grid = new THREE.GridHelper(TERRAIN_WORLD_SIZE, 40, 0x2a2a3a, 0x1a1a25);
    grid.position.y = 0.002;
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
    Object.assign(cursorLayer.style, {
      position: "absolute",
      inset: "0",
      pointerEvents: "none",
    });
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

    function addPrim(p: PrimRow): PrimEntry {
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

    function addCursor(c: CursorRow): CursorEntry {
      const sphere = new THREE.Mesh(
        new THREE.SphereGeometry(0.12, 12, 12),
        new THREE.MeshBasicMaterial({ color: c.color }),
      );
      sphere.position.set(c.x, c.y + 0.1, c.z);
      scene.add(sphere);

      const label = document.createElement("div");
      Object.assign(label.style, {
        position: "absolute",
        top: "0",
        left: "0",
        fontSize: "11px",
        fontFamily: "var(--font-mono, ui-monospace, monospace)",
        fontWeight: "600",
        textShadow: "0 1px 0 rgba(0,0,0,0.6)",
        whiteSpace: "nowrap",
        color: c.color,
        transition: "opacity 120ms",
      });
      label.textContent = c.name;
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

    let brushStroke: {
      lastSend: number;
      lastGndX: number;
      lastGndZ: number;
    } | null = null;

    const onMouseDown = (event: MouseEvent) => {
      if (event.button === 2) {
        orbiting = { x: event.clientX, y: event.clientY };
        return;
      }

      const currentTool = toolLatest.current;

      if (currentTool !== "orbit") {
        const primHit = intersectPrim(event);
        if (primHit) {
          setSelected(primHit.object.userData.primId as string);
          return;
        }
        const gnd = intersectGround(event);
        if (gnd) {
          brushStroke = {
            lastSend: 0,
            lastGndX: gnd.x,
            lastGndZ: gnd.z,
          };
        }
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

    function applyBrushAt(worldX: number, worldZ: number, sendServer: boolean) {
      const t = terrainLatest.current;
      if (!t) return;
      const { cx, cz } = worldToCell(worldX, worldZ, t.size);
      const radius = brushRadiusLatest.current;
      const strength = brushStrengthLatest.current;
      const tool = toolLatest.current;

      if (tool === "paint") {
        paintLocal(cx, cz, radius, strength, paintLayerLatest.current);
        if (sendServer) {
          callFn("paintTerrain", {
            roomId: ROOM_ID,
            cx, cz,
            radius,
            strength,
            layer: paintLayerLatest.current,
          }).catch((e: unknown) => { void e; });
        }
      } else if (tool !== "orbit") {
        sculptLocal(cx, cz, radius, strength, tool as "raise" | "lower" | "smooth" | "flatten");
        if (sendServer) {
          callFn("sculptTerrain", {
            roomId: ROOM_ID,
            cx, cz,
            radius,
            strength,
            mode: tool,
          }).catch((e: unknown) => { void e; });
        }
      }
    }

    const onMouseMove = (event: MouseEvent) => {
      const gnd = intersectGround(event);
      if (gnd) {
        groundHit.copy(gnd);
        const now = performance.now();
        if (now - lastCursorSend > 50) {
          lastCursorSend = now;
          const color = hashColor(userId);
          callFn("updateCursor", {
            roomId: ROOM_ID,
            name: myName,
            color,
            x: gnd.x, y: 0, z: gnd.z,
          }).catch((e: unknown) => { void e; });
        }

        const curTool = toolLatest.current;
        if (curTool !== "orbit") {
          brushCursor.visible = true;
          brushCursor.position.set(gnd.x, gnd.y + 0.02, gnd.z);
          const t = terrainLatest.current;
          if (t) {
            const worldPerCell = TERRAIN_WORLD_SIZE / (t.size - 1);
            const r = brushRadiusLatest.current * worldPerCell;
            brushCursor.scale.setScalar(r);
          }
        } else {
          brushCursor.visible = false;
        }
      }

      if (brushStroke) {
        const hit = gnd;
        if (hit) {
          brushStroke.lastGndX = hit.x;
          brushStroke.lastGndZ = hit.z;
          const now = performance.now();
          const sendServer = now - brushStroke.lastSend > 100;
          if (sendServer) brushStroke.lastSend = now;
          applyBrushAt(hit.x, hit.z, sendServer);
        }
        return;
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
            }).catch((e: unknown) => { void e; });
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
        const entry = primMeshes.get(dragging.primId);
        const prim = primsLatest.current.find((p) => p.id === dragging!.primId);
        if (entry && prim) {
          callFn("movePrim", {
            primId: dragging.primId,
            x: entry.group.position.x,
            y: prim.y,
            z: entry.group.position.z,
          }).catch((e: unknown) => { void e; });
        }
      }
      if (brushStroke) {
        applyBrushAt(brushStroke.lastGndX, brushStroke.lastGndZ, true);
      }
      dragging = null;
      orbiting = null;
      brushStroke = null;
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

    const onKeyDown = (e: KeyboardEvent) => {
      const tgt = e.target as HTMLElement;
      if (tgt && (tgt.tagName === "INPUT" || tgt.tagName === "TEXTAREA")) return;
      if (!selectedLatest.current) return;
      if (e.key === "Delete" || e.key === "Backspace") {
        callFn("deletePrim", { primId: selectedLatest.current })
          .then(() => setSelected(null))
          .catch((e: unknown) => { void e; });
      }
      const idx = Number(e.key) - 1;
      if (!Number.isNaN(idx) && idx >= 0 && idx < PRIM_COLORS.length) {
        callFn("colorPrim", {
          primId: selectedLatest.current,
          color: PRIM_COLORS[idx],
        }).catch((e: unknown) => { void e; });
      }
    };
    window.addEventListener("keydown", onKeyDown);

    // ---- Main loop ----
    let raf = 0;
    let lastTerrainUpdatedAt = "";
    const tick = () => {
      const t = terrainLatest.current;
      if (t && t.updatedAt !== lastTerrainUpdatedAt) {
        lastTerrainUpdatedAt = t.updatedAt;
        rebuildTerrainGeometry(t);
      }

      // Sync prims.
      const seen = new Set<string>();
      for (const p of primsLatest.current) {
        seen.add(p.id);
        const existing = primMeshes.get(p.id);
        if (!existing) {
          primMeshes.set(p.id, addPrim(p));
        } else {
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
        if (c.userId === userId) continue;
        cseen.add(c.id);
        let e = cursorMeshes.get(c.id);
        if (!e) {
          e = addCursor(c);
          cursorMeshes.set(c.id, e);
        }
        e.targetX = c.x; e.targetY = c.y; e.targetZ = c.z;
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
  }, [userId, myName]);

  // ---- Toolbar actions ----
  const spawn = useCallback(async (kind: string) => {
    setSpawning(true);
    try {
      const x = (Math.random() - 0.5) * 6;
      const z = (Math.random() - 0.5) * 6;
      const color = PRIM_COLORS[Math.floor(Math.random() * PRIM_COLORS.length)];
      await callFn("spawnPrim", { roomId: ROOM_ID, kind, x, z, color });
    } catch (_e) {
      // spawn errors are non-fatal in the demo
    } finally {
      setSpawning(false);
    }
  }, []);

  const deleteSelected = useCallback(async () => {
    if (!selected) return;
    const id = selected;
    setSelected(null);
    await callFn("deletePrim", { primId: id }).catch((e: unknown) => { void e; });
  }, [selected]);

  const colorSelected = useCallback(async (color: string) => {
    if (!selected) return;
    await callFn("colorPrim", { primId: selected, color }).catch((e: unknown) => { void e; });
  }, [selected]);

  const clearAll = useCallback(async () => {
    if (!window.confirm(`Remove all ${primsLatest.current.length} primitives from the scene?`)) return;
    const all = primsLatest.current;
    setSelected(null);
    await Promise.all(all.map((p) => callFn("deletePrim", { primId: p.id }).catch((e: unknown) => { void e; })));
  }, []);

  const selectedPrim = selected ? (prims ?? []).find((p) => p.id === selected) : null;
  const primCount = (prims ?? []).length;

  return (
    <div className="fixed inset-0 bg-[#0d0d12]">
      <div ref={mountRef} className="absolute inset-0" />

      {/* Top toolbar */}
      <Card className="absolute left-1/2 top-4 flex -translate-x-1/2 items-center gap-3 rounded-full border bg-card/85 px-3 py-1.5 backdrop-blur-sm">
        <div className="flex items-center gap-2 px-2 text-sm font-medium">
          <BrandMark />
          <span>Forge</span>
        </div>
        <div className="h-5 w-px bg-border" />
        <div className="flex items-center gap-1">
          {KINDS.map((k) => (
            <Button
              key={k.id}
              size="xs"
              variant="ghost"
              onClick={() => spawn(k.id)}
              disabled={spawning}
            >
              <KindIcon kind={k.id} />
              {k.label}
            </Button>
          ))}
        </div>
        <div className="h-5 w-px bg-border" />
        <div className="flex items-center gap-2">
          <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <span className="size-1.5 animate-pulse rounded-full bg-emerald-400" />
            {presenceCount + 1} online
          </span>
          {primCount > 0 && (
            <Badge variant="secondary" className="tabular-nums text-[10px]">
              {primCount} {primCount === 1 ? "prim" : "prims"}
            </Badge>
          )}
        </div>
        {primCount > 0 && (
          <>
            <div className="h-5 w-px bg-border" />
            <Button
              size="xs"
              variant="ghost"
              onClick={clearAll}
              className="text-destructive hover:bg-destructive/10 hover:text-destructive"
            >
              <Trash2 className="size-3" />
              Clear
            </Button>
          </>
        )}
      </Card>

      {/* Selected-object inspector */}
      {selectedPrim && (
        <Card className="absolute right-4 top-20 w-64 border bg-card/85 p-4 backdrop-blur-sm">
          <div className="flex items-center justify-between">
            <span className="text-sm font-medium capitalize">
              {selectedPrim.kind}
            </span>
            <div className="flex items-center gap-1.5">
              <span className="font-mono text-[10px] text-muted-foreground">
                #{selectedPrim.id.slice(0, 8)}
              </span>
              <Button
                size="xs"
                variant="ghost"
                onClick={() => setSelected(null)}
                className="h-5 w-5 p-0"
              >
                <X className="size-3" />
              </Button>
            </div>
          </div>
          <div className="mt-2 flex items-center justify-between text-xs">
            <span className="text-muted-foreground">Position</span>
            <span className="font-mono tabular-nums text-[11px]">
              {selectedPrim.x.toFixed(1)}, {selectedPrim.y.toFixed(1)},{" "}
              {selectedPrim.z.toFixed(1)}
            </span>
          </div>
          <div className="mt-3 flex flex-col gap-1.5">
            <span className="text-xs text-muted-foreground">Color</span>
            <div className="flex flex-wrap gap-1.5">
              {PRIM_COLORS.map((c) => (
                <button
                  key={c}
                  onClick={() => colorSelected(c)}
                  title={c}
                  className={cn(
                    "size-6 rounded-full border-2 transition-all",
                    selectedPrim.color === c
                      ? "border-foreground scale-110"
                      : "border-transparent hover:scale-105",
                  )}
                  style={{ background: c }}
                />
              ))}
            </div>
          </div>
          <div className="mt-3 flex gap-2">
            <Button
              size="xs"
              variant="destructive"
              onClick={deleteSelected}
              className="flex-1"
            >
              <Trash2 className="size-3" />
              Delete
            </Button>
          </div>
          <p className="mt-2 text-[10px] text-muted-foreground">
            Drag to move · <strong className="text-foreground">Del</strong> removes ·{" "}
            <strong className="text-foreground">1–6</strong> recolors
          </p>
        </Card>
      )}

      {/* Tool palette */}
      <Card className="absolute left-4 top-20 w-56 border bg-card/85 p-3 backdrop-blur-sm">
        <div className="mb-2 flex items-center gap-1.5 px-1 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
          <Mountain className="size-3" />
          Terrain
        </div>
        <div className="grid grid-cols-3 gap-1">
          {(
            [
              ["orbit", "Move", Move],
              ["raise", "Raise", Brush],
              ["lower", "Lower", Brush],
              ["smooth", "Smooth", Sparkles],
              ["flatten", "Flatten", Brush],
              ["paint", "Paint", PaintBucket],
            ] as const
          ).map(([t, label, Icon]) => (
            <button
              key={t}
              onClick={() => setTool(t)}
              className={cn(
                "flex flex-col items-center gap-1 rounded-md border px-2 py-2 text-[11px] transition-colors",
                tool === t
                  ? "border-primary bg-primary text-primary-foreground"
                  : "border-transparent text-foreground/80 hover:bg-accent hover:text-accent-foreground",
              )}
            >
              <Icon className="size-3.5" />
              {label}
            </button>
          ))}
        </div>

        {tool === "paint" && (
          <div className="mt-3">
            <div className="mb-1.5 px-1 text-[11px] text-muted-foreground">Layer</div>
            <div className="grid grid-cols-2 gap-1">
              {LAYER_LABELS.map((l, i) => (
                <button
                  key={l}
                  onClick={() => setPaintLayer(i as 0 | 1 | 2 | 3)}
                  className={cn(
                    "rounded-md border px-2 py-1.5 text-[11px] font-medium transition-all",
                    paintLayer === i
                      ? "border-foreground/40 ring-2 ring-foreground/20"
                      : "border-transparent",
                  )}
                  style={{
                    background: `#${LAYER_COLORS[i].getHexString()}`,
                    color: i >= 2 ? "#000" : "#fff",
                  }}
                  title={l}
                >
                  {l}
                </button>
              ))}
            </div>
          </div>
        )}

        {tool !== "orbit" && (
          <div className="mt-3 space-y-2">
            <Slider
              label="Radius"
              value={brushRadius}
              min={1}
              max={15}
              step={0.5}
              format={(v) => v.toFixed(1)}
              onChange={setBrushRadius}
            />
            <Slider
              label="Strength"
              value={brushStrength}
              min={0.05}
              max={1.5}
              step={0.05}
              format={(v) => v.toFixed(2)}
              onChange={setBrushStrength}
            />
          </div>
        )}
      </Card>

      {/* Name badge — shows your presence identity */}
      <div className="absolute left-4 bottom-14 flex items-center gap-1.5 rounded-full border bg-card/85 px-3 py-1 backdrop-blur-sm text-xs">
        <span
          className="size-2 rounded-full"
          style={{ background: hashColor(userId) }}
        />
        <span className="font-mono text-muted-foreground">{myName}</span>
      </div>

      {/* Hint strip */}
      <div className="absolute bottom-4 left-1/2 flex -translate-x-1/2 items-center gap-4 rounded-full border bg-card/85 px-4 py-1.5 text-[11px] text-muted-foreground backdrop-blur-sm">
        {tool === "orbit" ? (
          <>
            <span>
              <strong className="text-foreground">Left-click + drag</strong> a
              primitive to move
            </span>
            <span>
              <strong className="text-foreground">Right-click + drag</strong> to
              orbit
            </span>
            <span>
              <strong className="text-foreground">Scroll</strong> to zoom
            </span>
          </>
        ) : (
          <>
            <span>
              <strong className="text-foreground">Left-click + drag</strong> to{" "}
              {tool === "paint" ? "paint" : tool}
            </span>
            <span>
              <strong className="text-foreground">Right-click + drag</strong> to
              orbit
            </span>
            <span>
              Switch to <strong className="text-foreground">Move</strong> to edit
              primitives
            </span>
          </>
        )}
      </div>
    </div>
  );
}

function Slider({
  label,
  value,
  min,
  max,
  step,
  format,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  format: (v: number) => string;
  onChange: (v: number) => void;
}) {
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between text-[11px]">
        <span className="text-muted-foreground">{label}</span>
        <span className="font-mono tabular-nums">{format(value)}</span>
      </div>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        className="h-1 w-full cursor-pointer appearance-none rounded-full bg-muted accent-primary"
      />
    </div>
  );
}

function KindIcon({ kind }: { kind: string }) {
  switch (kind) {
    case "sphere":
      return <Circle className="size-3.5" />;
    case "cone":
      return <Cone className="size-3.5" />;
    case "torus":
      return <Torus className="size-3.5" />;
    default:
      return <Box className="size-3.5" />;
  }
}

function BrandMark() {
  return (
    <svg viewBox="0 0 48 64" width="14" height="18" fill="currentColor" className="text-primary">
      <path d="M24 2 L10 20 L24 32 Z" />
      <path d="M24 2 L38 20 L24 32 Z" />
      <path d="M24 32 L18 48 L24 62 L30 48 Z" />
      <path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
      <path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
    </svg>
  );
}
