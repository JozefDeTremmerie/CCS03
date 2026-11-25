# CCS03

CCS03 is a community fork that builds on
[victory-fork](https://github.com/fxceless/victory-fork), fxceless's fork of
Marvisak's [VIctory](https://github.com/Marvisak/VIctory). VIctory is a
compatibility layer that runs an Xbox Series title natively on Windows, from
files the user supplies; victory-fork adds a large amount of rendering,
lighting, frame-rate and memory work on top. CCS03 is where a small team
reviews, combines and tests fixes, and builds its own on top of
victory-fork.

No game files are in this repository or needed to develop it. To run the
title you need your own copy.

## What CCS03 adds

- Reviewed pull requests from victory-fork, VIctory, ZETTROX and
  Belobug's fork, and fixes of our own. Each one is checked first: is it
  already in the base, does victory-fork already solve it another way, do
  reviewers report problems, and does the whole test suite pass on Windows
  with an NVIDIA GPU (32-lane waves). The list, with authors, is in
  [CCS03.md](CCS03.md).
- A focus on NVIDIA cards with 8 GB and on laptops. The title's Series S
  profile runs with its own upscaler and interface plane, large textures
  leave out their largest level when the card's memory is small, the
  high-performance GPU of a laptop is used, and the title's missing
  ray-traced light is stood in for by a constant.
- Windows 10 through the DirectX 12 Agility SDK (see below).
- Tooling for contributors. One script sets up a clone and one runs the
  tests and Clippy as the CI does. A commit hook keeps game data out of the
  history, and [AGENTS.md](AGENTS.md) gives people and AI agents the same
  rules.

## Status

CCS03 follows victory-fork `main` (base `ca8829f`, 25 November). What
follows is what CCS03 itself has seen.

### Test runs of 2026-11-25, evening: in control of the character

Build: `ccs03` at `4632f81` with victory-fork `ca8829f` merged, release and diagnostics
builds, `--console series-s` (and once `series-x`), a 1080p display, the
switches under "Running" below.

Machine: the same laptop, GeForce RTX 4060 Laptop GPU (8 GB), Ryzen 9
7845HX. Game files: `game_scarlett_master.exe` and its packages, dumped
from our own console.

**Playable at a low frame rate: the player walks the character around the
prison yard with a pad or the keyboard, at 5.5 to 7.6 frames a second.**

| Stage | What we saw |
|-------|-------------|
| Boot, menus, start screen | 30 fps |
| Opening cutscene (the diner) | plays with sound; characters, faces, hair and the room drawn and lit; 1 to 3 fps. The first seconds of a shot are washed out, nearly white, while its textures stream onto the 8 GB card (up to 1.7 GB in five seconds), then the picture settles |
| Prologue, prison | the game hands control over ("Move around / Move the camera"); the player walks the character and turns the camera in the yard; 5.5 to 7.6 fps, 130 to 190 ms a frame, about 3,400 to 4,600 draws |
| Stability | four runs of 2.5 to 11 minutes on this build, each closed by us: no crash, out-of-memory or device removal |
| Draws left out | almost all are the stages of the ray-tracing structure builder, which is off (700 to 3,200 a frame in cutscenes); besides those, 3 compute programs: 1 that does not translate, 2 the driver refuses |

Seen and still open in the picture:

- Series S: white speckles on skin in close-ups; the same shot with
  `--console series-x` is clean, so it comes from the Series S path.
- The yard in sunlight is washed out: the constant that stands in for the
  ray-traced indirect light adds to reflections of the open sky.
- Some tattooed arms are drawn black with blue lines.
- At startup the title asks two console device methods (`0x1c8`, about
  16,000 calls, and `0x218`, about 2,000) for pipelines; neither is
  written yet anywhere, so the title goes without those pipelines.

### Test run of 2026-11-25, afternoon: Series S on an 8 GB laptop

Build: `ccs03` at `c060848`, release build, `--console series-s`, a 1080p
display, the switches under "Running" below.

Machine: a laptop with a GeForce RTX 4060 Laptop GPU (8 GB) and a Ryzen 9
7845HX.

**Still not fully playable, but the first rooms look close to the
console: 3 to 6 frames a second.**

| Stage | What we saw |
|-------|-------------|
| Boot, menus, loading screen | 30 fps |
| The cell and the first rooms | lit, with textures on walls, beds and posters, characters with skin, faces and hair, the yard and the guard tower through the window; 3 to 6 fps, 170 to 350 ms a frame, the card busy only 30 to 100 ms of it |
| Cutscenes | play in order; the first seconds of a new one run at about 1 frame a second while its textures stream in, and faces stay dark until theirs are there |
| Stability | no crash, out-of-memory or device removal in five runs of 3 to 35 minutes |

What changed since the morning (same day, RTX 2060 SUPER 8 GB, Series X
profile: 1 to 3 fps, most of what was not lit directly black, textures
blurry or missing):

- The Series S profile: the title renders inside at 1280x720 and keeps its
  own upscaler (victory-fork #798), its interface plane is drawn (#188).
  About twice the frame rate of Series X on the same card, and it fits 8 GB.
- victory-fork's frame-rate work of 25 November with its switches on: from
  about 4 to 6 fps on the laptop.
- Textures without their largest level on 8 GB cards (from victory-fork
  #814): before, about 450 textures waited for room on the card and figures
  and objects were white.
- A constant for the ray-traced indirect light (from Belobug's fork): what
  the sun does not reach is no longer black.
- 50 more shader programs translate (victory-fork #25 and #215): cascade
  shadows and characters' cut-out clothing and hair.

Still open:

- The processor side is the limit: one submit thread spends about 130 ms a
  frame preparing, 80 ms copying the title's memory to the card and 60 ms
  copying back (measured with victory-fork's frame log).
- Faces in a cutscene's first seconds, and dark scenes.
- Ray tracing: victory-fork #814 runs live ray-traced light on 8 GB cards,
  at about 1 frame a second; not in CCS03.
- 9 of the title's programs: 5 ray-traced shadow programs, 2 water volume
  programs, 1 tiled cascade-shadow program and 1 voxelisation program; and 3
  compute pipelines the driver refuses (they need 64-lane waves).

### All the title's shaders on the same card

`victory translate` over the title's shader pack and executable, as for a
host with 32-lane waves and a 1,000,000-view heap: all 26,096 programs
translate; 14,258 of 14,259 vertex and pixel programs, 2,674 of 2,682
compute programs and all 8,765 mesh programs become DXIL. The 195 geometry
programs that read an address no root parameter holds are not lowered by
`translate`.

### Running

The recommended way on an 8 GB card, as a set of environment variables:

```
VICTORY_DISPLAY_MODE=1920x1080
VICTORY_RELAY=2 VICTORY_ASYNC_SUBMIT=1 VICTORY_QUEUE2=1 VICTORY_INDIRECT_WINDOW=1
VICTORY_COPY_PLAIN=1 VICTORY_TEXTURE_NO_WAIT=1 VICTORY_WIDE_TEXTURES=1
VICTORY_TEXTURE_DROP_MIPS=1 VICTORY_TEXTURE_ROOM_PERCENT=86
VICTORY_CARD_ROOM_FOR_TARGETS_MB=1024 VICTORY_CARD_BUFFERS_MB=768 VICTORY_AMBIENT=0.9
VICTORY_JOIN_PARTS=1 VICTORY_BACK_OWN_VIEW=1 VICTORY_INDIRECT_GATHER=1 VICTORY_CARD_REHASH_RUNS=1
victory.exe run <game folder>\game_scarlett_master.exe --execute --console series-s
```

A shader and pipeline cache per PC (`VICTORY_CACHE_DIR`) keeps two
machines with different cards from rebuilding each other's pipelines.
Without the Series S profile, an 8 GB card runs out of memory in the world
at 1080p; `VICTORY_DISPLAY_MODE=1280x720` keeps it inside.

### Earlier base

The pull requests CCS03 carried on its previous base (VIctory `main`) are
being brought back one by one; see [CCS03.md](CCS03.md). The old history is
kept in the branch `ccs03-oud`.

## Windows 10

The title's root signatures use directly indexed descriptor heaps (shader
model 6.6). The D3D12 runtime built into Windows 10 refuses them, so without
help no pipeline can be made and the screen stays black. Windows 11 is not
affected.

`victory.exe` exports `D3D12SDKVersion` (614), so Windows loads a newer
`D3D12Core.dll` from a `D3D12` folder beside it (CCS03 PR 7, from VIctory PR
968). On Windows 10, put `D3D12Core.dll` and `d3d12SDKLayers.dll` from the
DirectX 12 Agility SDK 1.614 (NuGet package `Microsoft.Direct3D.D3D12`) in
that folder. Microsoft's DLLs are never committed here.

The test programs do not load the Agility SDK yet, so on Windows 10 the GPU
tests that need shader model 6.6 fail ("no SM 6.6 device"). Run them on
Windows 11, or compare against `ccs03` as the maintainers do.

## Getting started

- Working on CCS03: see [CONTRIBUTING.md](CONTRIBUTING.md). Work happens on
  the `ccs03` branch through pull requests.
- Building: `cargo build --release -p victory`. Requirements: Windows 10/11
  x64, a CPU with AVX2, a Direct3D 12 GPU with shader model 6.6 (and mesh
  shaders for the world), and Rust as pinned in `rust-toolchain.toml`. On
  Windows 10 also the `D3D12` folder described above.
- GPU: the high-performance card is used (on a laptop the NVIDIA or AMD
  card, not the one in the processor); `VICTORY_D3D12_ADAPTER=<index|name>`
  picks another, and the log names the one in use.
- Display size: the title is told about a 1080p display;
  `VICTORY_DISPLAY_MODE=3840x2160` reports a 4K one (more memory).
- Console profile: `--console series-s` for cards with 8 GB (see Running
  above); the Series X profile renders at 2560x1440 inside.
- Running, environment variables and architecture: see
  [victory-fork's README](https://github.com/fxceless/victory-fork#readme),
  [VIctory's README](https://github.com/Marvisak/VIctory#readme) and
  [docs/architecture.md](docs/architecture.md). victory-fork's speed
  switches (`VICTORY_LATE_WAIT`, `VICTORY_RELAY`, ...) are described in a
  comment where each is read.

## Team

| GitHub | Role |
|---|---|
| [JozefDeTremmerie](https://github.com/JozefDeTremmerie) | Maintainer: reviews and merges |
| [9hvn9yt2vk-netizen](https://github.com/9hvn9yt2vk-netizen) | Contributor: shader and GPU fixes |

## Credits

- [VIctory](https://github.com/Marvisak/VIctory) by Marvisak and its
  contributors: the loader, the runtime services, the Direct3D 12 back end
  and the shader recompiler.
- [victory-fork](https://github.com/fxceless/victory-fork) by fxceless and
  its contributors: the rendering, lighting, frame-rate and memory work
  CCS03 is based on. victory-fork used [XCL](https://github.com/ivall/xcl)
  by ivall as a reference.
- The authors of the pull requests in CCS03 are credited in
  [CCS03.md](CCS03.md). Fixes that are useful beyond CCS03 are offered back
  as pull requests.

## Licence and rules

CCS03 is licensed under GPL-3.0-or-later, like VIctory and victory-fork (see
[LICENSE](LICENSE)). Their copyright notices stay in place.

Never add game files or anything derived from them, code under an
incompatible licence (XCL is GPL-2.0-only), tools or keys to break the
title's protection, or artwork of the title or its publisher. The full list
is in [AGENTS.md](AGENTS.md).

CCS03 is unofficial and not affiliated with the title's developer or
publisher, or with Microsoft.
