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

- Reviewed pull requests from victory-fork and VIctory, and fixes of our
  own. Each one is checked first: is it already in the base, does
  victory-fork already solve it another way, do reviewers report problems,
  and does the whole test suite pass on Windows with an NVIDIA GPU (32-lane
  waves). The list, with authors, is in [CCS03.md](CCS03.md).
- A focus on NVIDIA and Windows 10. CCS03 tests on NVIDIA cards, which run
  32-lane waves and often have 8 GB of memory, and runs on Windows 10
  through the DirectX 12 Agility SDK (see below).
- Tooling for contributors. One script sets up a clone and one runs the
  tests and Clippy as the CI does. A commit hook keeps game data out of the
  history, and [AGENTS.md](AGENTS.md) gives people and AI agents the same
  rules.

## Status

CCS03 follows victory-fork `main` (base `4e17478`). victory-fork's README
describes the title on its test machine (Windows 11, RTX 4070 12 GB). What
follows is what CCS03 itself has seen.

### Test run of 2026-11-25

Build: `ccs03` at `55a691f`, release build, Series X profile, default
settings (the title is told about a 1080p display).

Machine: Windows 10 22H2 (19045), Ryzen 5 3600, GeForce RTX 2060 SUPER 8 GB
(driver 621.01, PCIe 3.0 x16), 16 GB RAM with a 17.6 GB page file, DirectX
12 Agility SDK 1.614.1.

**Not playable yet: 1 to 3 frames per second in the world, and most of
what is not lit directly is black.**

| Stage | What we saw |
|-------|-------------|
| Boot, menus, loading screen | 30 fps, as upstream |
| Loading into the story | about 7.5 minutes on the loading screen (longer on the very first run, while the shaders are translated) |
| Opening cutscene | plays in order: credits text, a night exterior, a dark interior, a daytime road with characters and vehicles |
| Frame rate in the world | 1 to 3 fps; 316 ms to 3 s per frame, median about 835 ms. Per frame: 100 to 490 ms on the GPU, 130 to 310 ms preparing (copies and textures), 80 to 135 ms copying back |
| Stability | 13 minutes without a crash, a hang or a device removal |

Problems seen in that run:

- Ground, vehicles and characters out of direct light are black or nearly
  black (the title's ray-traced light is not there yet, as in victory-fork).
- In the daytime shots, black boxes float in the sky and over the distance,
  a vertical band of the sky is shaded differently from the rest, and some
  vehicle surfaces are plain white.
- Textures turn blurry once the card's 8 GB are full ("the card has no
  room for more"), even at 1080p.
- The driver refuses 4 compute pipelines: they need 64-lane waves and NVIDIA
  cards have 32. 6 compute shaders do not translate.
- The diagnostics build (`--features diagnostics`) crashed twice where the
  world begins, in the title's own code (a write to address 0); the release
  build does not. Being looked into.

### All the title's shaders on the same card

`victory translate --validate --pipelines` over the title's shader pack, as
for a host with 32-lane waves: all 26,148 programs translate; of the 14,259
vertex and pixel programs 14,168 become DXIL and 14,057 pass DXC's
validator. The largest groups left are geometry programs that read an
address no root parameter holds (195), mesh programs whose output is over
the PC's 32 KB limit (71), input and output signatures the validator
rejects (72), descriptors read as data or chosen by control flow (80) and
compute programs that need 64-lane waves (23).

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
- Display size: the title is told about a 1080p display;
  `VICTORY_DISPLAY_MODE=3840x2160` reports a 4K one (more memory).
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
