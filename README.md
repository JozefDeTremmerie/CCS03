# CCS03

CCS03 is a community fork of [VIctory](https://github.com/Marvisak/VIctory),
Marvisak's compatibility layer that runs the Xbox Series X|S version of Grand
Theft Auto VI on Windows, from files the user supplies. VIctory does the heavy
lifting. CCS03 is where a small team reviews, combines and tests fixes before
upstream merges them, and builds its own on top.

No game files are in this repository or needed to develop it. To run the
game you need your own copy.

## What CCS03 adds

- **Reviewed upstream pull requests, ahead of upstream.** 19 open pull
  requests are merged into the `ccs03` branch. Each one is checked first: is
  it already upstream, do its reviewers report problems, does it merge
  cleanly, and does the whole test suite pass on Windows with an NVIDIA GPU
  (32-lane waves). The list, with authors, is in [CCS03.md](CCS03.md).
- **Our own fixes.** Shader and GPU fixes from CCS03 contributors (for
  example `v_ldexp_f32` / `v_frexp_*`, and raw buffer views that do not
  start on 16 bytes), plus the changes needed to make the merged pull
  requests build together, and a correction to one of them (PR 452 counted
  buffer atomics as reads). Details in [CCS03.md](CCS03.md).
- **A focus on NVIDIA and Windows 10.** Upstream is tested on one machine
  (Windows 11, AMD). CCS03 tests on NVIDIA cards, which run 32-lane waves,
  and is working on Windows 10 support (see below).
- **Tooling for contributors.** One script sets up a clone and one runs the
  tests and Clippy as upstream's CI does. A commit hook keeps game data out
  of the history, and [AGENTS.md](AGENTS.md) gives people and AI agents the
  same rules.

## Status

CCS03 follows upstream `main` (base `ffe310e`). For what the game does today,
see [the status section of upstream's README](https://github.com/Marvisak/VIctory#readme).
CCS03 makes no claims of its own about the game until they are reproduced on
our own hardware.

## Windows 10 (in progress)

The title's root signatures use directly indexed descriptor heaps (shader
model 6.6). The D3D12 runtime built into Windows 10 refuses them, so no
pipeline can be made and the screen stays black. Windows 11 is not affected.

The fix is the DirectX 12 Agility SDK: `victory.exe` exports
`D3D12SDKVersion` and `D3D12SDKPath`, and Windows then loads a newer
`D3D12Core.dll` from a `D3D12` folder beside it. To use it, download the
`Microsoft.Direct3D.D3D12` NuGet package in version 1.614.x and copy
`build/native/bin/x64/D3D12Core.dll` into the matching build folder, for
example `target/release/D3D12/D3D12Core.dll`. Without that DLL the system
runtime is used. Microsoft's DLLs are never committed here.

## Getting started

- **Working on CCS03:** see [CONTRIBUTING.md](CONTRIBUTING.md). Work happens
  on the `ccs03` branch through pull requests.
- **Building:** the same as upstream:
  `cargo build --release -p victory`. Requirements: Windows 10/11 x64, a CPU
  with AVX2, a Direct3D 12 GPU with shader model 6.6 (and mesh shaders for the
  world), and Rust as pinned in `rust-toolchain.toml`.
- **Running, environment variables and architecture:** see
  [upstream's README](https://github.com/Marvisak/VIctory#readme) and
  [docs/architecture.md](docs/architecture.md).

## Team

| GitHub | Role |
|---|---|
| [JozefDeTremmerie](https://github.com/JozefDeTremmerie) | Maintainer: reviews and merges |
| [9hvn9yt2vk-netizen](https://github.com/9hvn9yt2vk-netizen) | Contributor: shader and GPU fixes |

The authors of the upstream pull requests in CCS03 are credited in
[CCS03.md](CCS03.md). Fixes that are useful beyond CCS03 are offered back to
upstream as pull requests.

## Licence and rules

CCS03 is licensed under GPL-3.0-or-later, like VIctory (see
[LICENSE](LICENSE)). VIctory's copyright notices stay in place.

Never add game files or anything derived from them, code under an
incompatible licence (XCL is GPL-2.0-only), tools or keys to break the
game's protection, or Rockstar or GTA artwork. The full list is in
[AGENTS.md](AGENTS.md).

CCS03 is unofficial and not affiliated with Rockstar Games, Take-Two
Interactive or Microsoft.
