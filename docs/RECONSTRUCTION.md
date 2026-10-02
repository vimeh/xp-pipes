# Reconstruction, evidence and known limits

This project targets the supplied x86 Windows XP SP3 `3D Pipes.scr`, version
**5.1.2600.5512**, importing Direct3D 8. Its SHA-256 is
`664b4b1da036eb36b13a3bce2fd572e37127223cbf6796adde12cbfb3af0d893`.
Version metadata alone does not establish authenticity; the file had no
Authenticode signature. The matching public PDB supplied names, not complete types.
Decompilation was checked against assembly where types, aliases and x87 arithmetic
were ambiguous. The Windows executable was not executed during reconstruction.

**No executable, PDB, decompiled pseudocode, Ghidra database or extracted artwork
is included or fetched by this repository.** Possession of those artifacts does
not establish redistribution rights. Only the independently written implementation,
static numeric fixtures, tests and explanatory documentation are published.

## What is preserved

- MSVC integer RNG, warm-up and call ordering—including apparently redundant draws.
- Ordered node reservation, construction-time straightness, natural trapping,
  restarts, multiple-head chasing and material/joint selection.
- Reconstructed tube/elbow/ball meshes, materials, directional lighting and camera.
  The world follows aspect ratio: 5120×2160 uses 15×6×15 nodes, not a denser world.
- Render eligibility without catch-up, retained RGB8/D16 colour/depth histories,
  shared fades and their repeated quantization, including 29 overlays in a fresh
  uninterrupted transition before terminal clear.
- Explicit texture decoding and repetition rules; native-resolution buffers on
  both platforms. There is no interpolated/smoothed growth feature yet.

Default options follow the recovered path. Exact `--pipes 1..4` counts and
`--scene-length 1..10` are optional extensions; they are not recovered XP controls.

## Deliberate exceptions and unresolved behavior

- Failed starts and retired leaders are memory-safe. Undefined original array
  accesses and dangling pointers are not emulated.
- Host floating-point/trigonometric precision is not an x87/D3DX CPU emulator.
- Original out-of-range texture wrapping is undefined by D3D8's documented
  contract. The renderer isolates a deterministic shortest-period extension;
  no claim is made that this matches every old driver. Image decoder/POT-resizing
  precision can also differ.
- Multi-output callbacks and presentation are supplied by native hosts, not the
  original D3D8 device. Separate macOS processes cannot share randomness/fades.
  Native Mac context replacement restarts its scene.
- Smoother visual growth was discussed but has not been implemented. Increasing
  tessellation changes mesh roundness, not step timing.

## What tests establish

[Static fixtures](../tests/fixtures/README.md) include independently calculated
RNG checkpoints and recovered orientation data, rather than deriving every
expected result from the implementation under test. CPU tests cover settings,
reservations, restarts, budgets, timing and replay.

The common animation matrix covers 144 settings/seed/aspect cases through three
resets. The explicit GL suite runs production shaders, 780 fixed mesh/material
cases, texture/alpha/depth/culling, retained fades/replay, and actual framebuffer
allocations/rendering/readback/deletion through 8K. Linux uses surfaceless Mesa;
Mac native validation uses CGL/FBOs on Apple M3. Linux CI runs the GPU suite;
Mac CI does not claim GPU or saver-host coverage.

The original integration repository also tested Niri/Sway/PAM lock behavior,
crashes, hotplug, scaling and configure storms. Those VM integrations remain in
that repository, not in this standalone package's CI. They are not a guarantee
for every compositor, physical display, dock, or authentication configuration.

**None of this is an independent running-XP image oracle.** Whole-scene runtime
or pixel equivalence, physical external-display acceptance and Apple's actual
sandboxed Screen Saver host remain separate checks.

## Original evidence reports

The longer research history is retained at the extraction revision of
`vimeh/nixos-config`, rather than copying host configuration and private workspaces:

- [Source/provenance](https://github.com/vimeh/nixos-config/blob/b1e5def4963efea4bbbe1e5619a470a49bbd3cd2/docs/windows-xp-pipes-source.md)
- [Fidelity analysis](https://github.com/vimeh/nixos-config/blob/b1e5def4963efea4bbbe1e5619a470a49bbd3cd2/docs/windows-xp-pipes-fidelity.md)
- [Rendering](https://github.com/vimeh/nixos-config/blob/b1e5def4963efea4bbbe1e5619a470a49bbd3cd2/docs/wlpipes-xp-rendering.md)
- [Textures](https://github.com/vimeh/nixos-config/blob/b1e5def4963efea4bbbe1e5619a470a49bbd3cd2/docs/wlpipes-xp-textures.md)
- [Validation](https://github.com/vimeh/nixos-config/blob/b1e5def4963efea4bbbe1e5619a470a49bbd3cd2/docs/wlpipes-xp-validation.md)

Wayland infrastructure attribution is preserved in [NOTICE](../NOTICE) and
[LICENSE](../LICENSE). This project is not affiliated with Microsoft.
