# CCS03

CCS03 is a fork of [VIctory](https://github.com/Marvisak/VIctory). It follows
upstream `main` and adds open upstream pull requests that have been reviewed,
tested and found to help, before upstream merges them. Every pull request is
checked first: is it already in upstream, do reviewers report problems with it,
does it merge cleanly, and does the whole test suite pass on Windows with an
NVIDIA GPU (32-lane waves).

Base: upstream `main` at `ffe310e`.

## Upstream pull requests in CCS03

Credit goes to their authors (names as in their commits).

| PR | What it does | Author |
|---|---|---|
| 477 | DXIL validator: no undefined behaviour on null error blobs, checked status, no leaked handles | nathanbzrr |
| 684 | Tests find the PC compiler (`dxc.exe`) independently of the validator | Dathinel |
| 578 | Refuse wave64 mesh and compute programs on hosts without 64-lane waves (avoids the GPU hang on NVIDIA) | wbkepler |
| 628 | `ds_min_f32` / `ds_max_f32` with RDNA2's NaN rules | mercuryVM |
| 605 | `v_cvt_rpi_i32_f32` rounds the exact sum; NaN saturates by its sign | upipibi |
| 596 | `v_cvt_pknorm_*` with exact rounding | mercuryVM |
| 587 | Mesh output bounds guarded, all primitives initialised, no sync on a divergent `s_barrier` | FibbS6 |
| 707 | `tbuffer_store_format_*` that need no conversion | mercuryVM |
| 635 | Buffer views past the end of their host copy are null, views running past it are cut | claude-GTA6 |
| 688 | Dynamic descriptor slots of lists that ran are taken back when a heap runs short | iurysf |
| 160 | Texel offsets, comparison and gather forms of image samples | r4. |
| 568 | Table slots without a view get a null descriptor of the kind the shader reads | BastienRdz |
| 571 | LDS byte and half accesses | Saitken91 |
| 606 | `ds_swizzle_b32` rotate and FFT modes | upipibi |
| 642 | Enhanced texture barriers in `ID3D12GraphicsCommandList7::Barrier` are translated | nathanbzrr |
| 643 | Live host textures shared across aliased addresses; view descriptors canonicalised | nathanbzrr |
| 452 | Loops that only wait for memory stop after 4096 iterations; the loop limit works in release builds | AntoninMichelq |
| 284 | RDNA2's legacy multiply-adds are the fused ones | Dufftwt |
| 298 | Lane masks spilled to VGPR lanes and read back | permadiego |
| 573 | `ds_bpermute_b32` lowered without LDS (wave32 and 32-lane halves in wave64) | QB |

## From CCS03 contributors

| PR | What it does | Author |
|---|---|---|
| CCS03 PR 1 | `v_ldexp_f32`, `v_frexp_mant_f32` and `v_frexp_exp_i32_f32` | 9hvn9yt2vk-netizen |
| CCS03 PR 2 | Raw buffer views that do not start on 16 bytes become null views instead of removing the device (also upstream PR 123) | 9hvn9yt2vk-netizen |
| CCS03 PR 3 | `cargo xtask` finds the repository at run time, so the commit hook keeps working when the repository moves to another drive | Apenshot123 |

## Changes of CCS03's own

Small changes needed to make the pull requests above build together:

- `image_tests` (from PR 160) passes the program structure to
  `resource::track`, which gained that argument after the PR was written.
- `buffer_views_of_no_elements_are_null` (from PR 635) passes the null kind
  that PR 568 added to `HostHeap::make_at`.
- The valu_tests compute builder is split in two, so PR 606's tests (whole
  programs) and PR 628's (a body wrapped by the builder) both fit.
- `run_wave_cs` (from PR 573) passes `lowered.table_views` to `HostShader`,
  which PR 568 added.

- `README.md` is CCS03's own and links to upstream's README for the
  technical details. When merging upstream `main`, keep ours
  (`git checkout --ours README.md`) and check whether anything in it needs
  updating.

And one correction to a pull request:

- PR 452's spin-wait classification took buffer and image atomics for
  reads, so a loop counting with `buffer_atomic_add` stopped early (a
  reviewer of PR 452 measured this on AMD, NVIDIA and WARP). Atomics are
  effects now, with a test.

## Not taken (yet)

| PR | Why |
|---|---|
| 584 | Reported to keep the title from reaching its menu (black screen) |
| 708 | Breaks two `victory-core` tests on current `main` |
| 626 | Only needed on Windows 10 or older D3D12 runtimes |
| 968 | Windows 10 through the Agility SDK: being tested on Windows 10 with an RTX 2060 SUPER |
| 346 | Reviewers report bugs whose fixes are not in the PR yet |
| 233, 320, 439 | Running wave64 programs on 32-lane hosts: in-game reports still show driver compiler faults and a black world on NVIDIA |
| 599 | Large conflicts in the shader lowering; helps hosts with 64-lane waves only |
| 632 | 571 covers the same LDS accesses |
| 579 | A manual wave64 switch; PR 578 already refuses those programs on 32-lane hosts |
| 192, 242, 294, 32 | Conflicts with what is already in CCS03; to be looked at again |

These are looked at again whenever upstream moves.

## Building

Same as upstream (see [README.md](README.md)). The branch is `ccs03`.
