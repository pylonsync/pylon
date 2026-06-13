# City kit models

Drop **GLB** building models here and they load automatically — until
then the game renders clean procedural massing so it's never empty.

Built for the [Quaternius Downtown City MegaKit](https://quaternius.com/packs/downtowncitymegakit.html)
(CC0), but any low-poly GLBs work. Models are auto-scaled and centred to
the cell, so size/origin don't matter.

## Option A — name the files

Nine slots, three growth stages per zone:

| file              | used for                       |
| ----------------- | ------------------------------ |
| `res_small.glb`   | residential, level 1 (house)   |
| `res_mid.glb`     | residential, level 2 (block)   |
| `res_tower.glb`   | residential, level 3 (tower)   |
| `com_small.glb`   | commercial, level 1 (shop)     |
| `com_mid.glb`     | commercial, level 2 (offices)  |
| `com_tower.glb`   | commercial, level 3 (highrise) |
| `ind_small.glb`   | industrial, level 1            |
| `ind_mid.glb`     | industrial, level 2            |
| `ind_tower.glb`   | industrial, level 3            |

Any slot you leave out falls back to procedural massing for that
zone/level.

## Option B — keep original filenames, add a manifest

Drop the megakit GLBs in with their original names and add a
`models.json` next to this README mapping slots → filenames:

```json
{
  "res_small": "SM_Bld_House_01.glb",
  "res_mid":   "SM_Bld_Apartment_02.glb",
  "res_tower": "SM_Bld_Apartment_05.glb",
  "com_small": "SM_Bld_Shop_01.glb",
  "com_mid":   "SM_Bld_Office_02.glb",
  "com_tower": "SM_Bld_Skyscraper_01.glb",
  "ind_small": "SM_Bld_Warehouse_01.glb",
  "ind_mid":   "SM_Bld_Factory_01.glb",
  "ind_tower": "SM_Bld_Factory_03.glb"
}
```

> The MegaKit ships `.fbx` / `.gltf` / `.obj`. Convert the pieces you want
> to **.glb** first (Blender: File → Export → glTF Binary, or
> `gltf-transform` on the `.gltf` set).
