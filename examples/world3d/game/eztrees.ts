/**
 * Tree species built with EZ Tree (MIT, @dgreenheck/ez-tree) — a real
 * parametric tree generator with tuned presets, textured bark, and
 * leaf billboards (textures ship embedded as data URIs, so nothing is
 * fetched at runtime).
 *
 * We generate ONE tree per species at boot and instance its two
 * meshes (wood + leaves) across the island — a whole forest costs two
 * draw calls per species.
 */
import * as THREE from "three";
import { Tree } from "@dgreenheck/ez-tree";

export interface SpeciesMeshes {
  trunkGeo: THREE.BufferGeometry;
  trunkMat: THREE.Material;
  leafGeo: THREE.BufferGeometry;
  leafMat: THREE.Material;
}

/** Generate a species' base geometry/materials from an EZ Tree preset. */
export function buildSpecies(
  preset: string,
  seed: number,
  windTime: THREE.IUniform,
): SpeciesMeshes {
  const tree = new Tree();
  tree.loadPreset(preset);
  tree.options.seed = seed;
  tree.generate();

  // ez-tree's .d.ts is generated against an older three; the runtime
  // objects are plain three meshes/materials from OUR three install —
  // cast through unknown to bridge the type-version gap.
  const trunkMat = tree.branchesMesh.material as unknown as THREE.Material;
  const leafMat = tree.leavesMesh.material as unknown as THREE.MeshStandardMaterial;

  // ez-tree's stock leaf wind shader replaces <project_vertex> with a
  // raw modelViewMatrix transform — it never applies instanceMatrix,
  // so INSTANCED leaves all collapse onto the tree-local origin and
  // the forest renders as bare branches. Swap in an instancing-safe
  // sway injected at <begin_vertex>, leaving the stock projection
  // (which handles instanceMatrix) untouched.
  leafMat.onBeforeCompile = (shader) => {
    shader.uniforms.uTime = windTime;
    shader.vertexShader =
      "uniform float uTime;\n" +
      shader.vertexShader.replace(
        "#include <begin_vertex>",
        /* glsl */ `#include <begin_vertex>
        {
          #ifdef USE_INSTANCING
            vec2 seedPos = vec2(instanceMatrix[3][0], instanceMatrix[3][2]);
          #else
            vec2 seedPos = vec2(0.0);
          #endif
          float phase = seedPos.x * 0.31 + seedPos.y * 0.27 + position.x + position.z;
          transformed += uv.y * vec3(
            sin(uTime * 1.9 + phase) * 0.05 + sin(uTime * 0.7 + phase * 1.3) * 0.04,
            0.0,
            cos(uTime * 1.7 + phase) * 0.05
          );
        }`,
      );
  };
  leafMat.customProgramCacheKey = () => `ez-leaf-instanced:${preset}`;
  // Instanced leaves draw in the opaque pass with alpha cutout —
  // depth-sorted transparency doesn't work across instances. Keep the
  // preset's own alphaTest though: bush leaf textures are soft-alpha,
  // and forcing 0.5 discards every pixel (rendering bushes as bare
  // branches).
  leafMat.transparent = false;
  if (leafMat.alphaTest < 0.25) leafMat.alphaTest = 0.25;
  leafMat.needsUpdate = true;

  const leafGeo = tree.leavesMesh.geometry as unknown as THREE.BufferGeometry;
  const leafVerts = leafGeo.attributes.position?.count ?? 0;
  if (leafVerts === 0) {
    // A species with no leaf billboards renders as bare branches —
    // surface it instead of silently shipping dead trees.
    console.warn(`[world3d] EZ species "${preset}" generated 0 leaf vertices`);
  }

  return {
    trunkGeo: tree.branchesMesh.geometry as unknown as THREE.BufferGeometry,
    trunkMat,
    leafGeo,
    leafMat,
  };
}
