/**
 * Player character — a rigged, animated glTF.
 *
 * The model is Quaternius' "Animated Platformer Character" (CC0,
 * vendored at public/models/character.glb, served by Pylon's static
 * `public/` convention). It ships gun-ready clips — Idle_Gun /
 * Walk_Gun / Run_Gun — which the mixer crossfades by ground speed,
 * and a full bone rig: we mount the rifle on the `Fist.R` bone and
 * pitch the `Torso` bone to aim.
 *
 * Loading is async, so every soldier starts as an instant procedural
 * placeholder (simple jointed humanoid) and swaps to a
 * SkeletonUtils-cloned model the moment the shared template resolves.
 * If the fetch fails (offline), the placeholder simply stays.
 *
 * Convention: the group origin sits at EYE level (1.62 m above the
 * soles), matching the synced avatar y. Forward is -Z, like the camera.
 */
import * as THREE from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import * as SkeletonUtils from "three/addons/utils/SkeletonUtils.js";

const EYE_HEIGHT = 1.62;

// ---------------------------------------------------------------------------
// Shared template — fetched once, cloned per soldier
// ---------------------------------------------------------------------------

interface CharacterTemplate {
  scene: THREE.Object3D;
  animations: THREE.AnimationClip[];
  /** Uniform scale that normalizes the model to EYE_HEIGHT. */
  scale: number;
  /** Model-local soles offset (pre-scale bbox min y). */
  minY: number;
}

let templatePromise: Promise<CharacterTemplate | null> | null = null;
let gunPromise: Promise<THREE.Object3D | null> | null = null;

/**
 * The hand weapon — Quaternius' submachine gun (CC0, vendored at
 * public/models/rifle.glb). Normalized once here: longest bbox axis
 * becomes the barrel, pointing -Z, ~0.62 m long, origin at the grip.
 */
function loadGunTemplate(): Promise<THREE.Object3D | null> {
  if (gunPromise) return gunPromise;
  gunPromise = new GLTFLoader()
    .loadAsync("/models/rifle.glb")
    .then((gltf) => {
      const gun = gltf.scene;
      const bbox = new THREE.Box3().setFromObject(gun);
      const size = bbox.getSize(new THREE.Vector3());
      // Rotate the longest axis onto Z.
      if (size.x >= size.y && size.x >= size.z) gun.rotation.y = Math.PI / 2;
      else if (size.y >= size.x && size.y >= size.z) gun.rotation.x = Math.PI / 2;
      const wrapper = new THREE.Group();
      wrapper.add(gun);
      const oriented = new THREE.Box3().setFromObject(wrapper);
      const length = Math.max(0.01, oriented.max.z - oriented.min.z);
      // Oversized relative to a real SMG on purpose — the character is
      // a chunky 1.7 m critter and a to-scale gun reads like a toy.
      const s = 0.9 / length;
      wrapper.scale.setScalar(s);
      // Recenter: grip near origin, barrel toward -Z.
      const center = oriented.getCenter(new THREE.Vector3()).multiplyScalar(s);
      wrapper.position.sub(center);
      const root = new THREE.Group();
      root.add(wrapper);
      root.traverse((o) => {
        const mesh = o as THREE.Mesh;
        if (mesh.isMesh) mesh.castShadow = true;
      });
      return root as THREE.Object3D;
    })
    .catch((err) => {
      console.warn("[world3d] rifle model failed to load — keeping the box rifle", err);
      return null;
    });
  return gunPromise;
}

function loadTemplate(): Promise<CharacterTemplate | null> {
  if (templatePromise) return templatePromise;
  templatePromise = new GLTFLoader()
    .loadAsync("/models/character.glb")
    .then((gltf) => {
      const bbox = new THREE.Box3().setFromObject(gltf.scene);
      const height = Math.max(0.01, bbox.max.y - bbox.min.y);
      gltf.scene.traverse((o) => {
        const mesh = o as THREE.Mesh;
        if (mesh.isMesh) {
          mesh.castShadow = true;
          mesh.frustumCulled = false; // skinned bounds lag the pose
        }
      });
      return {
        scene: gltf.scene,
        animations: gltf.animations,
        // Eye height ≈ 93% of total model height.
        scale: (EYE_HEIGHT / 0.93) / height,
        minY: bbox.min.y,
      };
    })
    .catch((err) => {
      console.warn("[world3d] character model failed to load — using placeholder rig", err);
      return null;
    });
  return templatePromise;
}

/** Find a clip whose name ends with `suffix` ("Armature|Idle_Gun" → "Idle_Gun"). */
function clipNamed(clips: THREE.AnimationClip[], suffix: string): THREE.AnimationClip | null {
  return clips.find((c) => c.name.endsWith(`|${suffix}`) || c.name === suffix) ?? null;
}

// ---------------------------------------------------------------------------
// Rifle (mounted on the hand bone; also used by the placeholder)
// ---------------------------------------------------------------------------

const RIFLE_GEO = {
  body: new THREE.BoxGeometry(0.05, 0.08, 0.46),
  barrel: new THREE.CylinderGeometry(0.016, 0.016, 0.3, 8),
  mag: new THREE.BoxGeometry(0.035, 0.11, 0.05),
  stock: new THREE.BoxGeometry(0.04, 0.09, 0.14),
};
const RIFLE_MAT = new THREE.MeshStandardMaterial({
  color: "#26282c",
  roughness: 0.5,
  metalness: 0.35,
});

function buildRifle(): { rifle: THREE.Group; muzzle: THREE.Object3D } {
  const rifle = new THREE.Group();
  const add = (geo: THREE.BufferGeometry, x: number, y: number, z: number) => {
    const mesh = new THREE.Mesh(geo, RIFLE_MAT);
    mesh.position.set(x, y, z);
    mesh.castShadow = true;
    rifle.add(mesh);
    return mesh;
  };
  add(RIFLE_GEO.stock, 0, -0.01, 0.22);
  add(RIFLE_GEO.body, 0, 0, -0.05);
  const barrel = add(RIFLE_GEO.barrel, 0, 0.01, -0.4);
  barrel.rotation.x = Math.PI / 2;
  add(RIFLE_GEO.mag, 0, -0.09, -0.05);
  const muzzle = new THREE.Object3D();
  muzzle.position.set(0, 0.01, -0.56);
  rifle.add(muzzle);
  return { rifle, muzzle };
}

// ---------------------------------------------------------------------------
// Placeholder rig (instant; replaced when the template lands)
// ---------------------------------------------------------------------------

const PH_GEO = {
  head: new THREE.SphereGeometry(0.13, 12, 10),
  torso: new THREE.CapsuleGeometry(0.2, 0.4, 4, 10),
  leg: new THREE.CapsuleGeometry(0.075, 0.6, 4, 8),
};
const PH_LEG_MAT = new THREE.MeshStandardMaterial({ color: "#2e3328", roughness: 0.92 });

function buildPlaceholder(bodyMat: THREE.Material): THREE.Group {
  const g = new THREE.Group();
  const add = (geo: THREE.BufferGeometry, mat: THREE.Material, y: number, x = 0) => {
    const mesh = new THREE.Mesh(geo, mat);
    mesh.position.set(x, y, 0);
    mesh.castShadow = true;
    g.add(mesh);
  };
  add(PH_GEO.head, bodyMat, 0);
  add(PH_GEO.torso, bodyMat, -0.45);
  add(PH_GEO.leg, PH_LEG_MAT, -1.15, -0.11);
  add(PH_GEO.leg, PH_LEG_MAT, -1.15, 0.11);
  return g;
}

// ---------------------------------------------------------------------------
// Character
// ---------------------------------------------------------------------------

export interface Character {
  /** Origin at eye level; rotate .rotation.y for heading. */
  group: THREE.Group;
  /** Rotate (YXZ) to aim the upper body + rifle at the crosshair. */
  aimPivot: THREE.Object3D;
  /** World-space muzzle anchor — tracers start here. */
  muzzle: THREE.Object3D;
  /** Per-instance tint (the model's Main material follows this color). */
  bodyMat: THREE.MeshStandardMaterial;
  /** Drive animation: t = seconds, speed = m/s ground speed. */
  animate(t: number, speed: number): void;
  /** Kick the weapon (recoil) — call on every shot. */
  fire(): void;
  dispose(): void;
}

type GaitName = "idle" | "walk" | "run";

export function buildCharacter(color: string): Character {
  const group = new THREE.Group();
  const bodyMat = new THREE.MeshStandardMaterial({ color, roughness: 0.8 });

  // The aim pivot is a plain node the callers rotate; both the
  // placeholder rifle and the model's torso bone read from it.
  const aimPivot = new THREE.Group();
  aimPivot.rotation.order = "YXZ";
  aimPivot.position.set(0, -0.3, 0);
  group.add(aimPivot);

  // Instant placeholder + rifle on the pivot.
  let placeholder: THREE.Group | null = buildPlaceholder(bodyMat);
  group.add(placeholder);
  const { rifle, muzzle } = buildRifle();
  rifle.position.set(0.16, -0.1, -0.2);
  aimPivot.add(rifle);
  let recoil = 0;
  // Mount baseline — recoil offsets relative to wherever the rifle is
  // currently parented (aim pivot now, fist bone once the model lands).
  let rifleBaseZ = rifle.position.z;
  let rifleBaseRotX = rifle.rotation.x;
  let recoilStroke = 0.07; // shrinks with the bone-mount scale

  // Swap the box rifle for the modeled gun the moment it loads: the
  // `rifle` group (transform + muzzle anchor) stays, its box children
  // get replaced by a clone of the gun model.
  void loadGunTemplate().then((gun) => {
    if (!gun || disposed) return;
    for (const child of [...rifle.children]) {
      if (child !== muzzle) rifle.remove(child);
    }
    rifle.add(gun.clone(true));
    // Gun model is normalized to 0.9 m centered on the grip — the
    // nozzle sits at the far -Z end, where tracers must originate.
    muzzle.position.set(0, 0.03, -0.48);
  });

  // Model state, populated when the template resolves.
  let mixer: THREE.AnimationMixer | null = null;
  let actions: Partial<Record<GaitName, THREE.AnimationAction>> = {};
  let activeGait: GaitName = "idle";
  let torsoBone: THREE.Object3D | null = null;
  // The bone's pose as the mixer left it, BEFORE our aim offset. The
  // offset must layer on the animated pose per-frame; clips that don't
  // animate the torso never rewrite it, so naive `rotation.x +=` would
  // ACCUMULATE (symptom: the idle character slowly corkscrews).
  const torsoPostMixer = new THREE.Quaternion();
  let modelMainMat: THREE.MeshStandardMaterial | null = null;
  let disposed = false;
  let lastT = 0;

  void loadTemplate().then((template) => {
    if (!template || disposed) return;

    const model = SkeletonUtils.clone(template.scene);
    const root = new THREE.Group();
    root.add(model);
    root.scale.setScalar(template.scale);
    // Quaternius rigs face +Z; our forward convention is -Z.
    root.rotation.y = Math.PI;
    root.position.y = -EYE_HEIGHT - template.minY * template.scale;
    group.add(root);

    // Per-instance tint: clone the "Main" material once and keep it
    // following bodyMat.color (consumers retint via bodyMat).
    model.traverse((o) => {
      const mesh = o as THREE.Mesh;
      if (!mesh.isMesh) return;
      const mat = mesh.material as THREE.MeshStandardMaterial;
      if (mat?.name === "Main") {
        if (!modelMainMat) {
          modelMainMat = mat.clone();
          modelMainMat.color.copy(bodyMat.color);
        }
        mesh.material = modelMainMat;
      }
    });

    // Mount the rifle on the right fist BONE so it follows the hand
    // through the walk/idle clips (a static pivot mount floats beside
    // the body mid-stride). The rig's bones carry a ~49× scale, so
    // both the rifle's scale AND its local offset are countered by the
    // measured bone world scale — uncountered offsets put the gun 7 m
    // away, which cost us a full debugging round.
    torsoBone = model.getObjectByName("Torso") ?? model.getObjectByName("Spine") ?? null;
    // Seed the post-mixer snapshot with the REST pose so the first
    // frame's restore doesn't zero an un-animated bone.
    if (torsoBone) torsoPostMixer.copy(torsoBone.quaternion);
    const fist =
      model.getObjectByName("FistR") ??
      model.getObjectByName("Fist_R") ??
      model.getObjectByName("Fist.R");
    if (fist) {
      aimPivot.remove(rifle);
      fist.add(rifle);
      group.updateMatrixWorld(true);
      const ws = new THREE.Vector3();
      fist.getWorldScale(ws);
      const inv = 1 / Math.max(1e-4, ws.x);
      rifle.scale.setScalar(inv);
      // Blender bones run +Y from joint to tip; a small slide along
      // the bone seats the grip in the fist.
      rifle.position.set(0, 0.12 * inv, 0);
      rifle.rotation.set(Math.PI / 2, 0, 0);
      rifleBaseZ = rifle.position.z;
      rifleBaseRotX = rifle.rotation.x;
      recoilStroke = 0.07 * inv;
    } else {
      rifle.position.set(0.3, -0.58, -0.28);
      rifleBaseZ = rifle.position.z;
    }

    // Animations: gun-ready gait set, crossfaded by speed.
    mixer = new THREE.AnimationMixer(model);
    const clips = template.animations;
    const pick = (...names: string[]) => {
      for (const n of names) {
        const clip = clipNamed(clips, n);
        if (clip) return mixer!.clipAction(clip);
      }
      return undefined;
    };
    actions = {
      idle: pick("Idle_Gun", "Idle"),
      walk: pick("Walk_Gun", "Walk"),
      run: pick("Run_Gun", "Run"),
    };
    actions.idle?.play();
    activeGait = "idle";

    // Model's in — drop the placeholder.
    if (placeholder) {
      group.remove(placeholder);
      placeholder = null;
    }
  });

  // Placeholder leg swing (only used until the model lands).
  let phPhase = 0;

  return {
    group,
    aimPivot,
    muzzle,
    bodyMat,
    animate(t: number, speed: number) {
      const dt = Math.min(0.05, Math.max(0, t - lastT));
      lastT = t;

      // Recoil spring: the rifle kicks back along its local +Z on
      // fire() and eases home.
      recoil *= Math.exp(-dt * 16);
      rifle.position.z = rifleBaseZ + recoil * recoilStroke;
      rifle.rotation.x = rifleBaseRotX + recoil * -0.1;

      if (mixer) {
        // Gait selection with hysteresis-free thresholds + crossfade.
        const gait: GaitName = speed > 7 ? "run" : speed > 0.6 ? "walk" : "idle";
        if (gait !== activeGait && actions[gait]) {
          const from = actions[activeGait];
          const to = actions[gait]!;
          to.reset().play();
          if (from) from.crossFadeTo(to, 0.22, false);
          activeGait = gait;
        }
        // Restore the bone to its last post-mixer pose so our aim
        // offset never accumulates across frames, then let the mixer
        // write this frame's pose (a no-op for un-animated bones),
        // snapshot it, and layer the aim on top.
        if (torsoBone) torsoBone.quaternion.copy(torsoPostMixer);
        mixer.update(dt);
        if (torsoBone) {
          torsoPostMixer.copy(torsoBone.quaternion);
          // Model faces +Z under the PI-flipped root, so aim signs invert.
          torsoBone.rotateX(-aimPivot.rotation.x);
          torsoBone.rotateY(-aimPivot.rotation.y);
        }
        if (modelMainMat && !modelMainMat.color.equals(bodyMat.color)) {
          modelMainMat.color.copy(bodyMat.color);
        }
        return;
      }

      // Placeholder: minimal leg-swing walk cycle.
      if (placeholder) {
        const stride = Math.min(1, speed / 6.5);
        phPhase += dt * (4 + speed * 1.35);
        const legs = placeholder.children.filter((c) => c.position.y < -1);
        const swing = Math.sin(phPhase) * 0.5 * stride;
        if (legs[0]) legs[0].rotation.x = swing;
        if (legs[1]) legs[1].rotation.x = -swing;
      }
    },
    fire() {
      recoil = 1;
    },
    dispose() {
      disposed = true;
      mixer?.stopAllAction();
      bodyMat.dispose();
      modelMainMat?.dispose();
      // Template scene/geometries and the shared rifle/placeholder
      // geos stay alive for other characters.
    },
  };
}
