/**
 * World constants. Everything procedural derives from SEED, so every
 * client generates byte-identical terrain, vegetation, and buildings —
 * multiplayer only has to sync avatar poses and destroyed block keys.
 */
export const SEED = 1337;

/** World extent in meters: x and z span [-WORLD_HALF, WORLD_HALF]. */
export const WORLD_SIZE = 512;
export const WORLD_HALF = WORLD_SIZE / 2;

/** Sea level (world y). Terrain below this is underwater. */
export const WATER_LEVEL = 0;

/** Heightfield resolution (vertices per side). */
export const TERRAIN_RES = 257;

export const GRAVITY = 22; // m/s² — slightly arcade-y for snappy jumps

export const PLAYER = {
  eyeHeight: 1.7,
  radius: 0.45,
  walkSpeed: 6.5,
  sprintSpeed: 11.5,
  swimSpeed: 3.5,
  jumpSpeed: 8.5,
  /** Pose upload rate to the server while moving. */
  netHz: 10,
} as const;

export const WEAPON = {
  magSize: 30,
  reserveAmmo: Infinity,
  fireIntervalMs: 105, // ~570 rpm
  reloadMs: 1400,
  range: 220,
  grenadeSpeed: 26,
  grenadeRadius: 3.2, // meters of block destruction
  grenadeCooldownMs: 1200,
} as const;

/** Building block edge length in meters. */
export const BLOCK_SIZE = 1.25;

export interface QualityPreset {
  pixelRatioCap: number;
  /** Grass blades per m² inside the streamed radius around the player. */
  grassDensity: number;
  /** Radius (m) of streamed grass around the player. */
  grassRadius: number;
  shadowMapSize: number;
  cloudCount: number;
}

export const QUALITY: Record<"high" | "medium", QualityPreset> = {
  // Density is CLUMPS per m² — each instance is a 3-card tuft of ~16
  // baked blades, so 1.1/m² reads denser than 3 single blades did.
  high: { pixelRatioCap: 1.75, grassDensity: 1.1, grassRadius: 75, shadowMapSize: 2048, cloudCount: 42 },
  medium: { pixelRatioCap: 1.25, grassDensity: 0.55, grassRadius: 60, shadowMapSize: 1024, cloudCount: 28 },
};
