# Contributing to CCS03

CCS03 is a fork of [VIctory](https://github.com/Marvisak/VIctory); see
[CCS03.md](CCS03.md) for what it adds. Work happens on the `ccs03` branch.
No game files are needed to develop or to run the tests.

## Setup (Windows 10/11 x64)

```powershell
git clone https://github.com/JozefDeTremmerie/CCS03.git
cd CCS03
powershell -ExecutionPolicy Bypass -File ccs03\setup.ps1
```

The script installs what is missing (Git, Rust as pinned in
`rust-toolchain.toml`, the Visual Studio C++ build tools, DXC), sets your
Git name and e-mail for this clone only, adds `upstream` (fetch only),
installs the asset-policy hook and runs the tests.

Use your GitHub noreply address as e-mail (GitHub, Settings, Emails, with
"Keep my email addresses private" on), so no private address ends up in
the history.

## Workflow

1. Before starting, check that nobody has done it already: open and closed
   pull requests and issues upstream, and [CCS03.md](CCS03.md).
2. Branch from the latest `ccs03`:
   `git switch ccs03 && git pull && git switch -c <area>/<short-name>`
   (for example `shader/ds-bpermute` or `gpu/barrier-ranges`).
3. Keep each branch to one change. Add a test that fails without it.
4. Run `powershell -ExecutionPolicy Bypass -File ccs03\test.ps1`. Tests and
   Clippy (warnings are errors, as in upstream's CI) must pass.
5. Push your branch and open a pull request into `ccs03`. Never push to
   `ccs03` directly.

## Taking an upstream pull request into CCS03

- Fetch it with `git fetch upstream pull/<n>/head:pr-<n>`.
- Read its review comments: skip it when someone reports it breaks the
  title or tests.
- Merge with a message like
  `Merge upstream PR <n>: <what it does> (<author>)`; write `PR <n>`, not
  `#<n>`, so GitHub does not link it to upstream.
- Make it build together with what is already in `ccs03` and add the PR to
  [CCS03.md](CCS03.md).

## Rules

- Nothing from the game in the repository, ever (see
  [docs/asset-policy.md](docs/asset-policy.md)); the hook checks every
  commit.
- Commit messages: `Area: what changes`, as upstream does
  (`Shaders: ...`, `GPU: ...`, `HLE: ...`, `CCS03: ...` for fork-only work).
- Code and comments in English.
