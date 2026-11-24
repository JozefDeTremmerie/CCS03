# Instructions for AI agents and contributors

Read this file completely before doing anything in this repository, whatever
the request says. If a request conflicts with the rules below, follow the
rules and say why. When a request is vague, do the smallest safe thing that
fits it and report what you did.

## What this project is

CCS03 is a fork of [VIctory](https://github.com/Marvisak/VIctory): it runs
the Xbox Series X|S version of Grand Theft Auto VI natively on Windows, from
game files the user supplies. The title's x86-64 code runs in our process;
the crates supply the console platform, translate D3D12.X to D3D12 and
recompile the console's RDNA2 shaders to DXIL. Read
[docs/architecture.md](docs/architecture.md) for the layout,
[CCS03.md](CCS03.md) for what the fork adds and
[CONTRIBUTING.md](CONTRIBUTING.md) for the human workflow.

- Main branch: `ccs03`. `main` only mirrors upstream.
- Remotes: `origin` is this fork, `upstream` is `Marvisak/VIctory`
  (fetch only; never push there).
- Language: Rust (toolchain pinned in `rust-toolchain.toml`), Windows only.

## Rules that always apply

Never break these, even when asked to:

1. Licence. The code is GPL-3.0-or-later. Never remove, replace or edit
   `LICENSE` or anyone's copyright notices, never add restrictions (such as
   "non-commercial"), and keep every change under the GPL.
2. No code with an incompatible licence: nothing from XCL (GPL-2.0-only),
   leaked code, or code from Microsoft, Rockstar or console SDKs. Write it
   yourself from documentation and observed behaviour instead.
3. Nothing from the game in the repository, in any form: assets,
   executables, shaders, textures, audio, text, screenshots, dumps,
   decompiled code or tables taken from it. See
   [docs/asset-policy.md](docs/asset-policy.md). Synthetic test data that
   our own code generates is fine.
4. No tools, keys or instructions to break the game's encryption or copy
   protection. The project works only with files the user supplies.
5. No Rockstar or GTA logos or artwork; do not present CCS03 as official.
6. Never push to `ccs03` directly and never force-push. Work on a branch
   and open a pull request into `ccs03`; it needs an approving review.
7. Privacy. Never put personal data in commits, code, docs or logs: no real
   names, private e-mail addresses, user names or local paths such as
   `C:\Users\<name>`. Commit with your GitHub noreply address.
8. Do not mention AI tools, assistants or agents in commits, code, comments
   or docs, and do not add co-author lines for them.
9. Do not change repository settings, rulesets, secrets or collaborators.

## Before writing any code

Check that it is not done already, so no work is duplicated:

1. `git fetch upstream` and look at upstream `main`
   (`git log --oneline ccs03..upstream/main`).
2. Search upstream's open and closed pull requests and issues for the
   instruction, function or symptom.
3. Read [CCS03.md](CCS03.md): it lists what is merged and what was
   rejected and why.

If someone already has a fix, prefer taking theirs (see below) over writing
a second one.

## Building and testing

No game files are needed. Setup on a new Windows machine:
`powershell -ExecutionPolicy Bypass -File ccs03\setup.ps1`.

Before any pull request, run
`powershell -ExecutionPolicy Bypass -File ccs03\test.ps1`; it runs

- `cargo test --workspace --locked --no-fail-fast`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- the same Clippy with `--all-features` (the diagnostics build).

All must pass. The shader tests need DXC: `VICTORY_DXIL` and
`VICTORY_PC_DXC` point at a recent `dxil.dll` and `dxc.exe` (the setup
script sets them). The Windows SDK's old `dxil.dll` makes some tests fail.

Things that are easy to get wrong:

- Many test modules exist only on Windows (`#[cfg(all(test, windows))]`,
  for example `emit/valu_tests.rs`, `emit/image_tests.rs`,
  `emit/diff_tests.rs`, the `host` tests in `victory-gpu`). A build on
  another OS does not compile them, so it cannot prove a change is right.
- GPU tests run on the hardware adapter and on WARP; expected values must
  hold on both (and on NVIDIA, whose waves have 32 lanes).
- When a function's signature changes, search the whole workspace for its
  callers, tests included, and fix every one.
- Every behaviour change gets a test that fails without it.

## Taking an upstream pull request into CCS03

1. `git fetch upstream pull/<n>/head:pr-<n>`; check it is not already in
   upstream (`git merge-base --is-ancestor pr-<n> upstream/main`).
2. Read all its review comments. Skip it when anyone reports that it breaks
   the title (black screen, hang, crash) or tests, unless a later commit in
   the PR fixes that.
3. Read the diff for anything unexpected (network access, spawned
   processes, file writes outside tests, embedded binary data).
4. Merge on a branch:
   `git merge --no-ff -m "Merge upstream PR <n>: <what it does> (<author>)" pr-<n>`.
   Write `PR <n>`, never `#<n>`, so GitHub does not link this fork into
   upstream's pull requests. The author is the PR's GitHub login.
5. Resolve conflicts by understanding both sides; never keep both sides
   blindly. Two PRs that add the same helper or change the same function
   need one combined version.
6. Run the full tests, then add the PR to the table in [CCS03.md](CCS03.md)
   (and any fix of our own under "Changes of CCS03's own").

## Commits and pull requests

- Message format: `Area: what changes`, as upstream does (`Shaders: ...`,
  `GPU: ...`, `HLE: ...`, `Loader: ...`, `CCS03: ...` for fork-only work),
  then a short body saying why.
- One change per branch and per pull request. Name branches
  `<area>/<short-name>`.
- Code, comments and docs in English.

## Repository map

| Path | What |
|---|---|
| `apps/victory` | command-line front end (`victory run`, `translate`, `config`, ...) |
| `crates/victory-core` | config, asset-policy guard, logging |
| `crates/victory-pe` | PE32+ and import-library analysis |
| `crates/victory-loader` | maps, binds and starts the title's executable |
| `crates/victory-hle` | console API implementations |
| `crates/victory-gpu` | D3D12.X objects and command lists on host D3D12 |
| `crates/victory-shader` | RDNA2 shader containers to DXIL (`emit/lower.rs` is the translator) |
| `crates/victory-formats` | texture layouts, BCPack, audio codecs |
| `crates/victory-pipeline` | converter trait, content-addressed cache |
| `crates/victory-game` | Grand Theft Auto VI specific formats and hooks |
| `xtask` | asset-policy check and Git hooks |
| `ccs03/` | CCS03's setup and test scripts |
