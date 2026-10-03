# PBR materials

openOMSI can draw OMSI content with physically based materials: a normal map for small
relief, and roughness, metalness and ambient occlusion maps for how a surface reflects
light. OMSI 2 itself knows nothing of this, so these maps are purely an addition: a bus, a
map object, spline or terrain layer without them looks exactly as it always did, and content with them
still works in the original OMSI 2 (which ignores the extra files).

PBR maps are drawn with the **Enhanced** graphics (launcher → Settings → Graphics:
`graphics=enhanced`), on the computer and on Android alike.

## How maps are found

No config file is changed. The maps lie **next to the diffuse texture** and share its name,
with a suffix, as the common PBR tools (Substance, Blender, Materialize ...) export them. For
`Texture/bus.dds`:

| Map | File names | Notes |
|---|---|---|
| Normal | `bus_nn`, `bus_normal`, `bus_nrm` | tangent space; Direct3D style (green down) by default, add `_gl` for OpenGL style (green up): `bus_nn_gl`, `bus_normal_gl` |
| Roughness | `bus_rr`, `bus_rough`, `bus_roughness` | white = rough |
| Glossiness | `bus_gg`, `bus_gloss`, `bus_glossiness` | the inverse of roughness |
| Metalness | `bus_mm`, `bus_metal`, `bus_metallic`, `bus_metalness` | white = metal |
| Ambient occlusion | `bus_aa`, `bus_ao`, `bus_occlusion` | white = open |
| Packed | `bus_orm`, `bus_arm` | occlusion, roughness, metalness in red, green, blue |
| Packed | `bus_mra` | metalness, roughness, occlusion in red, green, blue |
| Height | `bus_height` | linear grayscale; needs `bus.pbr.cfg` with physical dimensions, described below |

Each can be `.png`, `.tga`, `.dds`, `.bmp` or `.jpg`; case does not matter. The short
suffixes are **doubled letters** (`_nn`, `_rr`, `_mm`, `_aa`, `_gg`) on purpose: OMSI mods
already use single letters for other things (`_n` is a night map, `_r` and `_m` mean
anything), so a single letter is never taken for a PBR map. A normal map must also look like
one (bluish), or it is ignored.

The separate maps are packed into one occlusion/roughness/metalness picture for the GPU.
PBR maps are kept at up to 4096 px and uncompressed (a normal map does not survive DXT1
compression), so use them where they are seen up close.

## How they are drawn

- The normal map bends the surface normal (a tangent frame is derived per pixel, so no
  tangents are needed in the `.o3d` model).
- Roughness sharpens or blurs the reflections of the environment and the sun's highlight;
  metalness tints the reflection with the diffuse colour and darkens the diffuse part.
- Occlusion darkens the ambient light only.
- Terrain layers use their diffuse texture's repeat coordinates for these maps. The painted
  layer's brush mask and road cutouts keep their original coordinates and coverage.
- An OMSI `[matl_envmap]` strength still scales the reflection, and the diffuse texture's
  alpha still works as OMSI's reflection mask where no roughness map is given.

## Authored height maps

An optional `road_height.png` beside `road.dds` can supply shading relief when there is no
usable normal map. This must be an authored **linear grayscale height map**, not a colour
photograph converted automatically to height. Black is the lowest point and white the
highest. Alpha is ignored. Use a seamless, repeating image for roads and terrain.

Add `road.pbr.cfg` beside it, with the following two blocks:

```text
[height_scale]
0.025

[texture_size]
2.0
2.0
```

`height_scale` is the black-to-white height difference in **metres**, greater than zero and
at most 1 metre. The two `texture_size` lines are the physical **width and height in metres
of one UV repeat**, each between 0.01 and 1000. Match the repeat size authored on the mesh
or terrain layer; these values do not change its UV mapping. In this example a 2 × 2 metre
texture represents 25 mm of relief. Missing, repeated, non-finite or out-of-range dimensions
leave the height map unused, with a diagnostic in the log.

The loader computes tangent-space normals from height differences over that physical
distance, including across repeat boundaries. Image resolution therefore does not set the
relief strength. A usable explicit normal map takes precedence; it is never combined with
the generated one. Height alone does not change roughness: add a roughness or ORM map to
describe the material's reflection.

This changes **lighting normals only** in Enhanced graphics. It does not displace mesh
vertices, alter silhouettes or wheel collisions, or provide parallax occlusion. Deep paving
gaps still need authored geometry if those effects matter. Content without PBR sidecars,
Vanilla graphics and the existing rain/snow behaviour keep their current paths.

## Tips for modders

- Export at the diffuse texture's resolution or half of it.
- Prefer one packed `_orm` file over three single maps: one file read instead of three.
- Check the green channel of a normal map: dents that look like bumps mean it needs `_gl`
  (or the other way round).
- Source: `crates/omsi-texture/src/pbr.rs`, `crates/omsi-render/src/enhanced.wgsl`.
