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
  32-lane waves, and is working on Windows 10 support (see below).
- Tooling for contributors. One script sets up a clone and one runs the
  tests and Clippy as the CI does. A commit hook keeps game data out of the
  history, and [AGENTS.md](AGENTS.md) gives people and AI agents the same
  rules.

## Status

CCS03 follows victory-fork `main` (base `4e17478`). For what the title does
today, see [the status section of victory-fork's README](https://github.com/fxceless/victory-fork#readme).
CCS03 makes no claims of its own about the title until they are reproduced
on our own hardware.

The pull requests CCS03 carried on its previous base (VIctory `main`) are
being brought back one by one; see [CCS03.md](CCS03.md). The old history is
kept in the branch `ccs03-oud`.

## Windows 10 (in progress)

The title's root signatures use directly indexed descriptor heaps (shader
model 6.6). The D3D12 runtime built into Windows 10 refuses them, so no
pipeline can be made and the screen stays black. Windows 11 is not affected.

The fix is the DirectX 12 Agility SDK: `victory.exe` exports
`D3D12SDKVersion`, and Windows then loads a newer `D3D12Core.dll` from a
`D3D12` folder beside it. VIctory PR 968 does this. It is being tested for
CCS03 and is not in `ccs03` yet. Microsoft's DLLs are never committed here.

## Getting started

- Working on CCS03: see [CONTRIBUTING.md](CONTRIBUTING.md). Work happens on
  the `ccs03` branch through pull requests.
- Building: `cargo build --release -p victory`. Requirements: Windows 10/11
  x64, a CPU with AVX2, a Direct3D 12 GPU with shader model 6.6 (and mesh
  shaders for the world), and Rust as pinned in `rust-toolchain.toml`.
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
