//! End-to-end tests of synthetic GFX10 VALU shaders lowered to DXIL, validated
//! by `dxil.dll` when present, and executed on the host GPU and WARP.
//!
//! Covers RDNA2 hardware semantics that differ from naive DXIL lowering:
//! `v_med3_f32`/`v_med3_f16` with NaN operands, `v_fract_f32`/`v_fract_f16`
//! near `1.0`, turn-range reduction for `v_sin_*`/`v_cos_*`, the LDS float
//! atomics `ds_min_f32`/`ds_max_f32`, the exact rounding of
//! `v_cvt_pknorm_*`, and the layouts `tbuffer_store_format_*` writes.

use super::host::{Dispatch, Host, Root, View};
use super::{HostShader, lower, validator};
use crate::native::{
    ComputeIo, HwStage, NativeProgram, ResourceType, RootParam, RootType, StageIo, UsedResource,
};
use crate::{io, ir, resource, structure};

const S_ENDPGM: u32 = 0xBF81_0000;

/// SOP1 `s_mov_b32 sdst, src0`.
const fn s_mov_b32(sdst: u8, src0: u8) -> u32 {
    0xBE80_0000 | (sdst as u32) << 16 | 0x03 << 8 | src0 as u32
}

/// SOP2 `op sdst, src0, src1`.
const fn sop2(op: u8, sdst: u8, src0: u8, src1: u8) -> u32 {
    0x8000_0000
        | (op as u32 & 0x7F) << 23
        | (sdst as u32 & 0x7F) << 16
        | (src1 as u32) << 8
        | src0 as u32
}

/// SMEM `s_load_dwordx4 s[dst:dst+3], s[base:base+1], 0` (soffset = null).
const fn s_load_dwordx4(dst: u8, base: u8) -> [u32; 2] {
    [
        0x3D << 26 | 0x02 << 18 | (dst as u32) << 6 | (base as u32 >> 1),
        125 << 25,
    ]
}

/// VOP1 `op vdst, src0` (`src0`: 0..105 = SGPR, 256..511 = VGPR).
const fn vop1(op: u16, vdst: u8, src0: u16) -> u32 {
    0x7E00_0000 | (vdst as u32) << 17 | (op as u32) << 9 | (src0 as u32 & 0x1FF)
}

/// VOP3 `op vdst, src0, src1, src2` (no modifiers).
const fn vop3(op: u16, vdst: u8, src0: u16, src1: u16, src2: u16) -> [u32; 2] {
    [
        0x35 << 26 | (op as u32 & 0x3FF) << 16 | vdst as u32,
        (src0 as u32 & 0x1FF) | (src1 as u32 & 0x1FF) << 9 | (src2 as u32 & 0x1FF) << 18,
    ]
}

/// MUBUF `buffer_store_dword vdata, off, s[srsrc:srsrc+3], 0 offset:byte_off`.
const fn buffer_store_dword(vdata: u8, srsrc: u8, byte_off: u16) -> [u32; 2] {
    [
        0x38 << 26 | 0x1C << 18 | (byte_off as u32 & 0xFFF),
        0x80 << 24 | ((srsrc as u32 >> 2) & 0x1F) << 16 | (vdata as u32) << 8,
    ]
}

/// DS `ds_swizzle_b32 vdst, vsrc offset:offset`.
const fn ds_swizzle_b32(vdst: u8, vsrc: u8, offset: u16) -> [u32; 2] {
    [
        0x36 << 26 | 0x35 << 18 | offset as u32,
        (vdst as u32) << 24 | vsrc as u32,
    ]
}

/// MUBUF `buffer_store_dword vdata, voff, s[srsrc:srsrc+3], 0 offen`.
const fn buffer_store_dword_offen(vdata: u8, voff: u8, srsrc: u8) -> [u32; 2] {
    [
        0x38 << 26 | 0x1C << 18 | 1 << 12,
        0x80 << 24 | ((srsrc as u32 >> 2) & 0x1F) << 16 | (vdata as u32) << 8 | voff as u32,
    ]
}

const fn sgpr(n: u8) -> u16 {
    n as u16
}

const fn vgpr(n: u8) -> u16 {
    256 + n as u16
}

/// Builds and lowers a 1-thread synthetic RDNA2 compute shader that receives
/// 4 dwords of root constants in `s0..s3`, runs `body` (leaving its result in
/// `v0`), and stores `v0` to dword 0 of the raw UAV at heap index 0.
fn build_valu_cs(body: &[u32]) -> HostShader {
    build_cs(body, 1, 0)
}

/// [`build_valu_cs`], with the lowering's refusal as an error.
fn try_build_valu_cs(body: &[u32]) -> Result<HostShader, String> {
    try_build_cs(body, 1, 0)
}

/// [`build_valu_cs`] with `threads` threads in x (thread index in `v0`
/// on entry, before `body`) and `lds_bytes` of LDS (a multiple of 512).
fn build_cs(body: &[u32], threads: u32, lds_bytes: u32) -> HostShader {
    try_build_cs(body, threads, lds_bytes).expect("lower")
}

/// [`try_build_words`] for a whole program, refusals as panics.
fn build_raw_cs(words: &[u32], threads: u32) -> HostShader {
    try_build_words(words, threads, 0).expect("lower")
}

/// [`build_cs`], with the lowering's refusal as an error.
fn try_build_cs(body: &[u32], threads: u32, lds_bytes: u32) -> Result<HostShader, String> {
    let mut words = Vec::new();
    // s[4:5] = descriptor table address (heap index 0 -> byte offset 0).
    words.push(s_mov_b32(5, 0x80)); // inline constant 0
    words.extend_from_slice(&s_load_dwordx4(8, 4));
    // Copy root constants s0..s3 into v1..v4 so VALU instructions operate on
    // VGPRs just as they do in real shaders.
    for i in 0..4u8 {
        words.push(vop1(0x001, i + 1, sgpr(i))); // v_mov_b32
    }
    words.extend_from_slice(body);
    words.extend_from_slice(&buffer_store_dword(0, 8, 0));
    words.push(S_ENDPGM);
    try_build_words(&words, threads, lds_bytes)
}

/// Lowers `words`, a whole synthetic RDNA2 compute program (thread id in
/// `v0`, root constants in `s0..s3`, the descriptor table address in `s4`,
/// a raw UAV at heap index 0), with `threads` threads in x and `lds_bytes`
/// of LDS.
fn try_build_words(words: &[u32], threads: u32, lds_bytes: u32) -> Result<HostShader, String> {
    let code: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let native = NativeProgram {
        stage: HwStage::Cs,
        wave64: false,
        vgprs: 8,
        layout: vec![
            RootParam {
                root: RootType::Constants,
                table_offset_dwords: 0,
                sgprs: 4,
                base_register: 0,
                user_data: 0,
                slot: 0,
            },
            RootParam {
                root: RootType::Table,
                table_offset_dwords: 0,
                sgprs: 1,
                base_register: 0,
                user_data: 4,
                slot: 1,
            },
        ],
        used: vec![UsedResource {
            kind: ResourceType::Uav,
            root: RootType::Table,
            table_offset_dwords: 0,
            lower_bound: 0,
            stride: 0,
            dwords: 4,
            user_data: 4,
            slot: 1,
        }],
        io: StageIo::Cs(ComputeIo {
            rsrc2: (5 << 1) | ((lds_bytes / 512) << 15), // 5 user SGPRs: s0..s4
            threads: [threads, 1, 1],
        }),
        code,
    };

    let p = ir::build(&native.code, native.wave64, native.vgprs).expect("ir::build");
    let st = structure::structure(&p).expect("structure");
    let res =
        resource::track(&p, &native.layout, native.stage, Some(&st)).expect("resource::track");
    let io = io::raise(&p, &native).expect("io::raise");
    let lowered = lower::lower(&lower::Parts {
        native: &native,
        p: &p,
        st: &st,
        io: &io,
        res: &res,
        inputs: Vec::new(),
        outputs: Vec::new(),
        prims: Vec::new(),
        mesh_stride: 0,
    })
    .map_err(|e| format!("{e:?}"))?;

    let shader = HostShader {
        stage: HwStage::Cs,
        dxil: lowered.dxil,
        user_data: lowered.user_data,
        samplers: lowered.samplers,
        static_samplers: Vec::new(),
        root_cbvs: lowered.root_cbvs,
        root_srvs: lowered.root_srvs,
        table_views: lowered.table_views,
    };

    if let Ok(mut v) = validator::Validator::load(validator::Validator::default_path().as_deref()) {
        let mut copy = shader.dxil.clone();
        let ours = copy[4..20].to_vec();
        v.validate(&mut copy).expect("DXIL validation failed");
        assert_eq!(copy[4..20], ours[..], "container hash mismatch");
    }

    Ok(shader)
}

/// One GPU test at a time. With these tests on the hardware adapter and WARP
/// from several test threads at once, now and then none of their dispatches
/// ever completes and the test binary hangs; and two GPU tests at once on
/// WARP can end the test process with an access violation. Other GPU test
/// modules take this lock too.
pub(super) static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs `shader` on every available SM 6.6 host (hardware GPU and WARP) for
/// each 4-dword input set and invokes `check(backend, case_index, inputs, result_dword)`.
fn run_cases(
    shader: &HostShader,
    cases: &[[u32; 4]],
    check: impl FnMut(&str, usize, [u32; 4], u32),
) {
    run_cases_on(shader, cases, &[("hardware", false), ("warp", true)], check);
}

fn run_cases_on(
    shader: &HostShader,
    cases: &[[u32; 4]],
    devices: &[(&str, bool)],
    mut check: impl FnMut(&str, usize, [u32; 4], u32),
) {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let mut ran = 0;
    for &(label, warp) in devices {
        let Ok(host) = Host::new(warp) else {
            eprintln!("no SM 6.6 {label} device; skipped");
            continue;
        };
        let (root, pso) = host
            .compute_pipeline(shader)
            .unwrap_or_else(|e| panic!("{label}: compute_pipeline failed: {e}"));
        for (i, &inputs) in cases.iter().enumerate() {
            let mut ud = vec![0u32; shader.user_data_dwords().max(1) as usize];
            for u in &shader.user_data {
                ud[u.index as usize] = match (u.root, u.slot) {
                    (RootType::Constants, 0) => inputs[u.dword as usize],
                    (RootType::Table, 1) => 0,
                    other => panic!("unexpected user data {other:?}"),
                };
            }
            let out = host
                .dispatch(
                    &root,
                    &pso,
                    &Dispatch {
                        roots: vec![(0, Root::Constants(ud))],
                        views: &[View::RawUav(1)],
                        samplers: &[],
                        groups: [1, 1, 1],
                    },
                )
                .unwrap_or_else(|e| panic!("{label}: dispatch failed: {e}"));
            check(label, i, inputs, out[0][0]);
        }
        ran += 1;
    }
    assert!(ran > 0, "no SM 6.6 device available ({devices:?})");
}

fn is_f16_nan(bits: u16) -> bool {
    (bits & 0x7FFF) > 0x7C00
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits >> 15) << 31;
    let exp = (bits >> 10) & 0x1F;
    let mant = bits & 0x3FF;
    let f32_bits = match exp {
        0 => {
            if mant == 0 {
                sign
            } else {
                let mut m = u32::from(mant);
                let mut e = 127 - 14;
                while (m & 0x400) == 0 {
                    m <<= 1;
                    e -= 1;
                }
                sign | (e << 23) | ((m & 0x3FF) << 13)
            }
        }
        0x1F => sign | (0xFF << 23) | (u32::from(mant) << 13),
        _ => sign | ((u32::from(exp) + 112) << 23) | (u32::from(mant) << 13),
    };
    f32::from_bits(f32_bits)
}

#[test]
fn packed_add_selects_halves_of_inline_constants() {
    // Add the inline constant 1/(2*pi) to v1 (zero) to read back its two halves.
    // The same GFX10 source field (248) has a different bit pattern for a
    // packed floating-point and a packed integer instruction.
    let cases = [
        // v_pk_add_f16 v0, 0x3118, v1
        ([0xcc0f4000, 0x1a0202f8], 0x0000_3118),
        // v_pk_add_f16 v0, 0x3118, v1 op_sel_hi:[0,1] (use low twice)
        ([0xcc0f4000, 0x120202f8], 0x3118_3118),
        // v_pk_add_u16 v0, 0x3e22f983, v1
        ([0xcc0a4000, 0x1a0202f8], 0x3e22_f983),
    ];
    for (words, expected) in cases {
        let shader = build_valu_cs(&words);
        run_cases(&shader, &[[0; 4]], |backend, _, _, got| {
            assert_eq!(got, expected, "{backend}: {words:08x?}");
        });
    }
}

#[test]
fn v_med3_f32_matches_rdna2_with_nan() {
    // v_med3_f32 v0, v1, v2, v3 (VOP3 opcode 0x157)
    let shader = build_valu_cs(&vop3(0x157, 0, vgpr(1), vgpr(2), vgpr(3)));
    let nan = f32::NAN.to_bits();
    let f = f32::to_bits;

    let cases: &[([u32; 4], Option<f32>)] = &[
        // Normal medians (all orderings).
        ([f(1.0), f(2.0), f(3.0), 0], Some(2.0)),
        ([f(3.0), f(1.0), f(2.0), 0], Some(2.0)),
        ([f(-5.0), f(0.0), f(5.0), 0], Some(0.0)),
        ([f(1.0), f(1.0), f(2.0), 0], Some(1.0)),
        // One NaN: RDNA2 returns min3 of the non-NaN operands.
        ([f(1.0), f(2.0), nan, 0], Some(1.0)),
        ([nan, f(1.0), f(0.0), 0], Some(0.0)),
        ([nan, f(0.0), f(1.0), 0], Some(0.0)),
        ([f(2.0), nan, f(1.0), 0], Some(1.0)),
        ([f(-3.0), nan, f(4.0), 0], Some(-3.0)),
        // Two NaNs: returns the sole non-NaN operand.
        ([nan, f(5.0), nan, 0], Some(5.0)),
        ([nan, nan, f(-2.0), 0], Some(-2.0)),
        ([f(7.0), nan, nan, 0], Some(7.0)),
        // Three NaNs: returns NaN.
        ([nan, nan, nan, 0], None),
    ];

    let inputs: Vec<[u32; 4]> = cases.iter().map(|c| c.0).collect();
    run_cases(&shader, &inputs, |backend, i, inp, got| {
        let expect = cases[i].1;
        let got_f = f32::from_bits(got);
        match expect {
            Some(want) => assert_eq!(
                got_f,
                want,
                "{backend}: v_med3_f32({:?}, {:?}, {:?}) = {got_f} ({got:#010x}), want {want}",
                f32::from_bits(inp[0]),
                f32::from_bits(inp[1]),
                f32::from_bits(inp[2]),
            ),
            None => assert!(
                got_f.is_nan(),
                "{backend}: v_med3_f32(NaN, NaN, NaN) = {got_f} ({got:#010x}), want NaN",
            ),
        }
    });
}

/// RDNA2 orders -0 below +0 in its maximums and minimums; DXIL's FMax and
/// FMin compare and select, which can give either zero.
#[test]
fn max_and_min_order_negative_zero_below_positive_zero() {
    let (pz, nz) = (0_u32, 0x8000_0000_u32);
    // Each case has a -0 and a +0 among its first two operands.
    let cases = [
        [nz, pz, nz, 0],
        [pz, nz, nz, 0],
        [nz, pz, pz, 0],
        [pz, nz, pz, 0],
    ];
    let ops = [
        ("v_max_f32", 0x110, pz),
        ("v_min_f32", 0x10F, nz),
        ("v_max3_f32", 0x154, pz),
        ("v_min3_f32", 0x151, nz),
    ];
    for (name, op, want) in ops {
        let shader = build_valu_cs(&vop3(op, 0, vgpr(1), vgpr(2), vgpr(3)));
        run_cases(&shader, &cases, |backend, _, inp, got| {
            assert_eq!(got, want, "{backend}: {name} {inp:x?}");
        });
    }
}

#[test]
fn v_med3_f16_matches_rdna2_with_nan() {
    // v_med3_f16 v0, v1, v2, v3 (VOP3 opcode 0x357)
    let shader = build_valu_cs(&vop3(0x357, 0, vgpr(1), vgpr(2), vgpr(3)));
    let h0 = 0x0000u32; // 0.0
    let h1 = 0x3C00u32; // 1.0
    let h2 = 0x4000u32; // 2.0
    let h3 = 0x4200u32; // 3.0
    let h5 = 0x4500u32; // 5.0
    let hn2 = 0xC000u32; // -2.0
    let nan = 0x7E00u32;

    let cases: &[([u32; 4], Option<u16>)] = &[
        ([h1, h2, h3, 0], Some(h2 as u16)),
        ([hn2, h0, h3, 0], Some(h0 as u16)),
        ([h1, h2, nan, 0], Some(h1 as u16)),
        ([nan, h1, h0, 0], Some(h0 as u16)),
        ([nan, h0, h1, 0], Some(h0 as u16)),
        ([h2, nan, h1, 0], Some(h1 as u16)),
        ([nan, h5, nan, 0], Some(h5 as u16)),
        ([nan, nan, nan, 0], None),
    ];

    let inputs: Vec<[u32; 4]> = cases.iter().map(|c| c.0).collect();
    run_cases(&shader, &inputs, |backend, i, inp, got| {
        let expect = cases[i].1;
        let got_h = got as u16;
        match expect {
            Some(want) => assert_eq!(
                got_h, want,
                "{backend}: v_med3_f16({:#06x}, {:#06x}, {:#06x}) = {got_h:#06x}, want {want:#06x}",
                inp[0], inp[1], inp[2],
            ),
            None => assert!(
                is_f16_nan(got_h),
                "{backend}: v_med3_f16(NaN, NaN, NaN) = {got_h:#06x}, want NaN",
            ),
        }
    });
}

/// `v_cmp_lg_*` is "less or greater": ordered and not equal, so false when
/// either operand is a NaN (RDNA2 ISA; the same on gfx1036 hardware).
#[test]
fn v_cmp_lg_is_false_for_nan() {
    // v_cmp_lg_{f32,f16}_e64 s20, v1, v2 (VOPC 0x005 / 0x0cd in VOP3);
    // v_cndmask_b32_e64 v0, 0, 1, s20 (VOP3 0x101)
    let nan = f32::NAN.to_bits();
    let (inf, ninf) = (f32::INFINITY.to_bits(), f32::NEG_INFINITY.to_bits());
    let f = f32::to_bits;
    let f32_cases: &[([u32; 4], u32)] = &[
        ([f(1.0), f(2.0), 0, 0], 1),
        ([f(2.0), f(1.0), 0, 0], 1),
        ([f(1.0), f(1.0), 0, 0], 0),
        ([f(0.0), f(-0.0), 0, 0], 0),
        ([inf, ninf, 0, 0], 1),
        ([inf, inf, 0, 0], 0),
        ([nan, f(1.0), 0, 0], 0),
        ([f(1.0), nan, 0, 0], 0),
        ([nan, nan, 0, 0], 0),
        ([0x7F80_0001, f(0.0), 0, 0], 0), // a signalling NaN
    ];
    let f16_cases: &[([u32; 4], u32)] = &[
        ([0x3C00, 0x4000, 0, 0], 1),
        ([0x4000, 0x3C00, 0, 0], 1),
        ([0x3C00, 0x3C00, 0, 0], 0),
        ([0x0000, 0x8000, 0, 0], 0),
        ([0x7C00, 0xFC00, 0, 0], 1),
        ([0x7E00, 0x3C00, 0, 0], 0),
        ([0x3C00, 0x7E00, 0, 0], 0),
        ([0x7E00, 0x7E00, 0, 0], 0),
        ([0x7C01, 0x0000, 0, 0], 0),           // a signalling NaN
        ([0xABCD_3C00, 0x1234_7E00, 0, 0], 0), // only the low halves are read
    ];
    for (op, what, cases) in [
        (0x005u16, "v_cmp_lg_f32", f32_cases),
        (0x0cd, "v_cmp_lg_f16", f16_cases),
    ] {
        let mut body = vop3(op, 20, vgpr(1), vgpr(2), 0x80).to_vec();
        body.extend_from_slice(&vop3(0x101, 0, 0x80, 0x81, sgpr(20)));
        let shader = build_valu_cs(&body);
        let inputs: Vec<[u32; 4]> = cases.iter().map(|c| c.0).collect();
        run_cases(&shader, &inputs, |backend, i, inp, got| {
            let want = cases[i].1;
            assert_eq!(
                got, want,
                "{backend}: {what}({:#010x}, {:#010x}) = {got}, want {want}",
                inp[0], inp[1],
            );
        });
    }
}

#[test]
fn v_fma_legacy_f32_follows_the_d3d9_zero_rule() {
    // v_fma_legacy_f32 v0, v1, v2, v3 (VOP3 opcode 0x140 on RDNA2)
    let shader = build_valu_cs(&vop3(0x140, 0, vgpr(1), vgpr(2), vgpr(3)));
    let f = f32::to_bits;
    let cases: &[([u32; 4], u32)] = &[
        ([f(2.0), f(3.0), f(1.0), 0], f(7.0)),
        // A zero factor gives a zero product, whatever the other one is.
        ([0, f(f32::INFINITY), f(1.5), 0], f(1.5)),
        ([f(f32::NAN), 0, f(-2.0), 0], f(-2.0)),
    ];
    let inputs: Vec<[u32; 4]> = cases.iter().map(|c| c.0).collect();
    run_cases(&shader, &inputs, |backend, i, _, got| {
        let want = cases[i].1;
        assert_eq!(got, want, "{backend}: case {i}: {got:#x}, want {want:#x}");
    });
}

#[test]
fn v_fract_f32_never_returns_one() {
    // v_fract_f32 v0, v1 (VOP1 opcode 0x020)
    let shader = build_valu_cs(&[vop1(0x020, 0, vgpr(1))]);
    let f = f32::to_bits;
    let below_one = 0x3F7F_FFFFu32;

    let cases: &[(u32, Option<u32>)] = &[
        // Tiny negatives: must clamp strictly below 1.0 (not 0x3f800000).
        (f(-1e-10), Some(below_one)),
        (f(-1e-8), Some(below_one)),
        (f(-2.0f32.powi(-25)), Some(below_one)),
        (f(-2.0f32.powi(-24)), Some(below_one)),
        // Normal values.
        (f(0.0), Some(f(0.0))),
        (f(-0.0), Some(f(0.0))),
        (f(0.25), Some(f(0.25))),
        (f(3.75), Some(f(0.75))),
        (f(-0.25), Some(f(0.75))),
        (f(-3.25), Some(f(0.75))),
        (f(1.0), Some(f(0.0))),
        (f(-1.0), Some(f(0.0))),
        // Non-finite inputs yield NaN.
        (f(f32::NAN), None),
        (f(f32::INFINITY), None),
        (f(f32::NEG_INFINITY), None),
    ];

    let inputs: Vec<[u32; 4]> = cases.iter().map(|&(x, _)| [x, 0, 0, 0]).collect();
    run_cases(&shader, &inputs, |backend, i, inp, got| {
        let expect = cases[i].1;
        match expect {
            Some(want) => assert_eq!(
                got,
                want,
                "{backend}: v_fract_f32({:?} / {:#010x}) = {got:#010x} ({:?}), want {want:#010x}",
                f32::from_bits(inp[0]),
                inp[0],
                f32::from_bits(got),
            ),
            None => assert!(
                f32::from_bits(got).is_nan(),
                "{backend}: v_fract_f32({:?}) = {got:#010x}, want NaN",
                f32::from_bits(inp[0]),
            ),
        }
    });
}

#[test]
fn v_fract_f16_never_returns_one() {
    // v_fract_f16 v0, v1 (VOP1 opcode 0x05f)
    let shader = build_valu_cs(&[vop1(0x05f, 0, vgpr(1))]);
    let below_one = 0x3BFFu16; // 1.0 - 2^-11

    assert_eq!(f16_to_f32(0x9400), -2.0f32.powi(-10));
    assert_eq!(f16_to_f32(0x9000), -2.0f32.powi(-11));
    assert_eq!(f16_to_f32(0x8C00), -2.0f32.powi(-12));

    let cases: &[(u16, Option<u16>)] = &[
        // Negatives in (-2^-11, 0): must clamp to 0x3BFF, never 0x3C00 (1.0).
        (0x8C00, Some(below_one)), // -2^-12
        (0x8400, Some(below_one)), // -2^-14
        (0x8001, Some(below_one)), // -2^-24 (smallest negative f16 subnormal)
        (0x9000, Some(below_one)), // -2^-11 (exact 1 - 2^-11 = 0x3BFF)
        (0x9400, Some(0x3BFE)),    // -2^-10 (exact 1 - 2^-10 = 0x3BFE)
        // Normal values.
        (0x0000, Some(0x0000)), // 0.0 -> 0.0
        (0x3400, Some(0x3400)), // 0.25 -> 0.25
        (0xB400, Some(0x3A00)), // -0.25 -> 0.75
        (0x4300, Some(0x3800)), // 3.5 -> 0.5
        (0x3C00, Some(0x0000)), // 1.0 -> 0.0
        // Non-finite inputs yield NaN.
        (0x7E00, None), // NaN
        (0x7C00, None), // +inf
        (0xFC00, None), // -inf
    ];

    let inputs: Vec<[u32; 4]> = cases
        .iter()
        .map(|&(x, _)| [u32::from(x), 0, 0, 0])
        .collect();
    run_cases(&shader, &inputs, |backend, i, inp, got| {
        let expect = cases[i].1;
        let got_h = got as u16;
        match expect {
            Some(want) => assert_eq!(
                got_h, want,
                "{backend}: v_fract_f16({:#06x}) = {got_h:#06x}, want {want:#06x}",
                inp[0],
            ),
            None => assert!(
                is_f16_nan(got_h),
                "{backend}: v_fract_f16({:#06x}) = {got_h:#06x}, want NaN",
                inp[0],
            ),
        }
    });
}

/// RDNA2's flt16_to_snorm16 / flt16_to_unorm16: clamp, scale by 32767 or
/// 65535, round the exact product to nearest even; NaN gives 0.
fn cvt_norm16_reference(bits: u16, signed: bool) -> u16 {
    let x = f16_to_f32(bits);
    if x.is_nan() {
        return 0;
    }
    let (lo, scale) = if signed {
        (-1.0, 32767.0)
    } else {
        (0.0, 65535.0)
    };
    // f64 holds the product of an f16 and a 16-bit scale exactly.
    let p = f64::from(x.clamp(lo, 1.0)) * scale;
    p.round_ties_even() as i32 as u16
}

#[test]
fn v_cvt_norm_i16_and_u16_f16_round_the_exact_product() {
    // Every 251st f16 pattern, plus the values where an f32 product would
    // round before the rounding to an integer, ties, and the edges.
    let mut bits: Vec<u16> = (0..=u16::MAX).step_by(251).collect();
    bits.extend([
        0x0000, 0x8000, 0x0001, 0x8001, 0x03FF, 0x0400, 0x3800, 0xB800, 0x3801, 0xB801, 0x37FF,
        0x3BFF, 0xBBFF, 0x3C00, 0xBC00, 0x3C01, 0x7BFF, 0x7C00, 0xFC00, 0x7E00, 0xFE00, 0x7C01,
        0x3A9B, 0xBA9B, 0x2E66, 0x1400,
    ]);
    for (op, signed, what) in [
        (0x063u16, true, "v_cvt_norm_i16_f16"),
        (0x064, false, "v_cvt_norm_u16_f16"),
    ] {
        // v_cvt_norm_*16_f16 v0, v1 (VOP1); v0's high half starts at 0.
        let shader = build_valu_cs(&[vop1(0x001, 0, 128), vop1(op, 0, vgpr(1))]);
        let inputs: Vec<[u32; 4]> = bits.iter().map(|&b| [u32::from(b), 0, 0, 0]).collect();
        run_cases(&shader, &inputs, |backend, i, _, got| {
            let want = cvt_norm16_reference(bits[i], signed);
            assert_eq!(
                got,
                u32::from(want),
                "{backend}: {what}({:#06x}) = {got:#010x}, want {want:#06x}",
                bits[i],
            );
        });
    }
}

#[test]
fn v_sin_and_v_cos_f32_reduce_large_turns() {
    let sin_shader = build_valu_cs(&[vop1(0x035, 0, vgpr(1))]);
    let cos_shader = build_valu_cs(&[vop1(0x036, 0, vgpr(1))]);
    let f = f32::to_bits;
    let half_sqrt2 = std::f32::consts::FRAC_1_SQRT_2;

    let sin_cases: &[(f32, Option<f32>)] = &[
        (0.0, Some(0.0)),
        (0.25, Some(1.0)),
        (255.0, Some(0.0)),
        (255.25, Some(1.0)),
        (255.5, Some(0.0)),
        (-255.25, Some(-1.0)),
        (10_000.125, Some(half_sqrt2)),
        (f32::NAN, None),
        (f32::INFINITY, None),
        (f32::NEG_INFINITY, None),
    ];
    let sin_inputs: Vec<[u32; 4]> = sin_cases.iter().map(|&(x, _)| [f(x), 0, 0, 0]).collect();
    run_cases(&sin_shader, &sin_inputs, |backend, i, inp, got| {
        let expect = sin_cases[i].1;
        let got_f = f32::from_bits(got);
        match expect {
            Some(want) => assert!(
                (got_f - want).abs() < 1e-5,
                "{backend}: v_sin_f32({}) = {got_f}, want {want} (err = {})",
                f32::from_bits(inp[0]),
                (got_f - want).abs(),
            ),
            None => assert!(
                got_f.is_nan(),
                "{backend}: v_sin_f32({}) = {got_f}, want NaN",
                f32::from_bits(inp[0]),
            ),
        }
    });

    let cos_cases: &[(f32, Option<f32>)] = &[
        (0.0, Some(1.0)),
        (0.25, Some(0.0)),
        (255.0, Some(1.0)),
        (255.25, Some(0.0)),
        (255.5, Some(-1.0)),
        (-255.5, Some(-1.0)),
        (10_000.125, Some(half_sqrt2)),
        (f32::NAN, None),
        (f32::INFINITY, None),
        (f32::NEG_INFINITY, None),
    ];
    let cos_inputs: Vec<[u32; 4]> = cos_cases.iter().map(|&(x, _)| [f(x), 0, 0, 0]).collect();
    run_cases(&cos_shader, &cos_inputs, |backend, i, inp, got| {
        let expect = cos_cases[i].1;
        let got_f = f32::from_bits(got);
        match expect {
            Some(want) => assert!(
                (got_f - want).abs() < 1e-5,
                "{backend}: v_cos_f32({}) = {got_f}, want {want} (err = {})",
                f32::from_bits(inp[0]),
                (got_f - want).abs(),
            ),
            None => assert!(
                got_f.is_nan(),
                "{backend}: v_cos_f32({}) = {got_f}, want NaN",
                f32::from_bits(inp[0]),
            ),
        }
    });

    // Main's `trig_turns` preserves sin(-0.0) = -0.0 and produces exact +0.0 at
    // half-turns (sin) and quarter-turns (cos).
    let exact_sin: &[(u32, u32)] = &[
        (0x8000_0000, 0x8000_0000), // sin(-0.0) = -0.0
        (f(255.0), 0),              // sin(255.0) = +0.0
        (f(255.5), 0),              // sin(255.5) = +0.0
    ];
    let exact_sin_in: Vec<[u32; 4]> = exact_sin.iter().map(|&(x, _)| [x, 0, 0, 0]).collect();
    run_cases(&sin_shader, &exact_sin_in, |backend, i, inp, got| {
        let want = exact_sin[i].1;
        assert_eq!(
            got, want,
            "{backend}: v_sin_f32({:#010x}) = {got:#010x}, want {want:#010x}",
            inp[0],
        );
    });
    let exact_cos: &[(u32, u32)] = &[
        (f(0.25), 0),   // cos(0.25) = +0.0
        (f(255.25), 0), // cos(255.25) = +0.0
    ];
    let exact_cos_in: Vec<[u32; 4]> = exact_cos.iter().map(|&(x, _)| [x, 0, 0, 0]).collect();
    run_cases(&cos_shader, &exact_cos_in, |backend, i, inp, got| {
        let want = exact_cos[i].1;
        assert_eq!(
            got, want,
            "{backend}: v_cos_f32({:#010x}) = {got:#010x}, want {want:#010x}",
            inp[0],
        );
    });
}

#[test]
fn v_cvt_u16_and_i16_f16_saturate() {
    // v_mov_b32 v0, 0; v_cvt_u16_f16 / v_cvt_i16_f16 v0, v1 (VOP1 0x052 / 0x053).
    let cases: &[(u16, u16, u16)] = &[
        // (f16 input, u16 result, i16 result)
        (0x0000, 0, 0),          // 0
        (0x8000, 0, 0),          // -0
        (0x3E00, 1, 1),          // 1.5 -> 1 (toward zero)
        (0xBE00, 0, 0xFFFF),     // -1.5 -> 0 / -1
        (0x57D0, 125, 125),      // 125.0
        (0x7800, 32768, 32767),  // 32768 saturates i16
        (0x77FF, 32752, 32752),  // 32752, the largest f16 below 32768
        (0xF800, 0, 0x8000),     // -32768 fits i16
        (0xF801, 0, 0x8000),     // -32800 saturates i16
        (0x7BFF, 65504, 32767),  // max finite f16
        (0xFBFF, 0, 0x8000),     // -65504
        (0x7C00, 0xFFFF, 32767), // +inf
        (0xFC00, 0, 0x8000),     // -inf
        (0x7E00, 0, 0),          // NaN
        (0xFE00, 0, 0),          // -NaN
        (0x0001, 0, 0),          // smallest subnormal
    ];
    for (op, what, pick) in [
        (0x052u16, "v_cvt_u16_f16", 0usize),
        (0x053, "v_cvt_i16_f16", 1),
    ] {
        let shader = build_valu_cs(&[vop1(0x001, 0, 128), vop1(op, 0, vgpr(1))]);
        let inputs: Vec<[u32; 4]> = cases.iter().map(|c| [u32::from(c.0), 0, 0, 0]).collect();
        run_cases(&shader, &inputs, |backend, i, _, got| {
            let want = if pick == 0 { cases[i].1 } else { cases[i].2 };
            assert_eq!(
                got,
                u32::from(want),
                "{backend}: {what}({:#06x}) = {got:#010x}, want {want:#06x}",
                cases[i].0,
            );
        });
    }
}

#[test]
fn v_sin_and_v_cos_f16_reduce_large_turns() {
    let sin_shader = build_valu_cs(&[vop1(0x060, 0, vgpr(1))]);
    let cos_shader = build_valu_cs(&[vop1(0x061, 0, vgpr(1))]);

    // Sanity-check the f16 bit encodings of large turns.
    assert_eq!(f16_to_f32(0x5BF8), 255.0);
    assert_eq!(f16_to_f32(0x5BFA), 255.25);
    assert_eq!(f16_to_f32(0x5BFC), 255.5);
    assert_eq!(f16_to_f32(0x7BFF), 65504.0);

    let sin_cases: &[(u16, Option<f32>)] = &[
        (0x5BF8, Some(0.0)),  // 255.0
        (0x5BFA, Some(1.0)),  // 255.25
        (0x5BFC, Some(0.0)),  // 255.5
        (0xDBFA, Some(-1.0)), // -255.25
        (0x7BFF, Some(0.0)),  // 65504.0 (largest finite f16, exact integer)
        (0x7E00, None),       // NaN
        (0x7C00, None),       // +inf
        (0xFC00, None),       // -inf
    ];
    let sin_inputs: Vec<[u32; 4]> = sin_cases
        .iter()
        .map(|&(x, _)| [u32::from(x), 0, 0, 0])
        .collect();
    run_cases(&sin_shader, &sin_inputs, |backend, i, inp, got| {
        let expect = sin_cases[i].1;
        let got_h = got as u16;
        match expect {
            Some(want) => {
                let got_f = f16_to_f32(got_h);
                assert!(
                    (got_f - want).abs() < 1e-3,
                    "{backend}: v_sin_f16({:#06x} = {}) = {got_f} ({got_h:#06x}), want {want}",
                    inp[0],
                    f16_to_f32(inp[0] as u16),
                );
            }
            None => assert!(
                is_f16_nan(got_h),
                "{backend}: v_sin_f16({:#06x}) = {got_h:#06x}, want NaN",
                inp[0],
            ),
        }
    });

    let cos_cases: &[(u16, Option<f32>)] = &[
        (0x5BF8, Some(1.0)),  // 255.0
        (0x5BFA, Some(0.0)),  // 255.25
        (0x5BFC, Some(-1.0)), // 255.5
        (0x7BFF, Some(1.0)),  // 65504.0 (largest finite f16, exact integer)
        (0x7E00, None),       // NaN
        (0x7C00, None),       // +inf
        (0xFC00, None),       // -inf
    ];
    let cos_inputs: Vec<[u32; 4]> = cos_cases
        .iter()
        .map(|&(x, _)| [u32::from(x), 0, 0, 0])
        .collect();
    run_cases(&cos_shader, &cos_inputs, |backend, i, inp, got| {
        let expect = cos_cases[i].1;
        let got_h = got as u16;
        match expect {
            Some(want) => {
                let got_f = f16_to_f32(got_h);
                assert!(
                    (got_f - want).abs() < 1e-3,
                    "{backend}: v_cos_f16({:#06x} = {}) = {got_f} ({got_h:#06x}), want {want}",
                    inp[0],
                    f16_to_f32(inp[0] as u16),
                );
            }
            None => assert!(
                is_f16_nan(got_h),
                "{backend}: v_cos_f16({:#06x}) = {got_h:#06x}, want NaN",
                inp[0],
            ),
        }
    });
}

#[test]
fn bfe_signed_and_unsigned_match_rdna2() {
    // v_bfe_i32 v0, v1, v2, v3 (VOP3 opcode 0x149)
    let v_signed = build_valu_cs(&vop3(0x149, 0, vgpr(1), vgpr(2), vgpr(3)));
    // v_bfe_u32 v0, v1, v2, v3 (VOP3 opcode 0x148)
    let v_unsigned = build_valu_cs(&vop3(0x148, 0, vgpr(1), vgpr(2), vgpr(3)));
    // s_bfe_i32 s6, s0, s1 (SOP2 opcode 0x28: s1 = off[4:0] | width[22:16]); v_mov_b32 v0, s6
    let s_signed = build_valu_cs(&[sop2(0x28, 6, 0, 1), vop1(0x001, 0, sgpr(6))]);

    let signed_cases: &[([u32; 4], u32)] = &[
        // Field crosses bit 31: extends from bit 31.
        ([0xd2f9_0159, 28, 16, 0], 0xffff_fffd),
        ([0x8000_0000, 31, 4, 0], 0xffff_ffff),
        ([0x7000_0000, 28, 8, 0], 0x0000_0007),
        // Normal fields inside 32 bits.
        ([0x0000_0080, 4, 4, 0], 0xffff_fff8),
        ([0x0000_0070, 4, 4, 0], 0x0000_0007),
        // Edge widths: 0 yields 0; v_bfe masks width to 5 bits (32 -> 0),
        // while 31 extracts all remaining bits from offset 4.
        ([0xffff_ffff, 4, 0, 0], 0),
        ([0x8000_0000, 4, 31, 0], 0xf800_0000),
    ];
    let v_in: Vec<[u32; 4]> = signed_cases.iter().map(|c| c.0).collect();
    run_cases(&v_signed, &v_in, |backend, i, inp, got| {
        let want = signed_cases[i].1;
        assert_eq!(
            got, want,
            "{backend}: v_bfe_i32({:#010x}, {}, {}) = {got:#010x}, want {want:#010x}",
            inp[0], inp[1], inp[2],
        );
    });
    // v_bfe_i32 width is 5-bit (src2[4:0]), so width=32 wraps to 0.
    run_cases(
        &v_signed,
        &[[0x8000_0000, 4, 32, 0]],
        |backend, _, _, got| {
            assert_eq!(got, 0, "{backend}: v_bfe_i32 width 32 should mask to 0");
        },
    );

    // s_bfe_i32 width is 7-bit (src1[22:16]), so width=32 keeps all 32 bits.
    let mut s_cases = signed_cases.to_vec();
    s_cases.push(([0x8000_0000, 4, 32, 0], 0xf800_0000));
    let s_in: Vec<[u32; 4]> = s_cases
        .iter()
        .map(|&(inp, _)| [inp[0], (inp[1] & 31) | ((inp[2] & 0x7F) << 16), 0, 0])
        .collect();
    run_cases(&s_signed, &s_in, |backend, i, inp, got| {
        let want = s_cases[i].1;
        assert_eq!(
            got, want,
            "{backend}: s_bfe_i32({:#010x}, {:#010x}) = {got:#010x}, want {want:#010x}",
            inp[0], inp[1],
        );
    });

    let unsigned_cases: &[([u32; 4], u32)] = &[
        ([0xd2f9_0159, 28, 16, 0], 0x0000_000d),
        ([0x8000_0000, 31, 4, 0], 0x0000_0001),
        ([0xffff_ffff, 4, 0, 0], 0),
        ([0x8000_0000, 4, 31, 0], 0x0800_0000),
    ];
    let u_in: Vec<[u32; 4]> = unsigned_cases.iter().map(|c| c.0).collect();
    run_cases(&v_unsigned, &u_in, |backend, i, inp, got| {
        let want = unsigned_cases[i].1;
        assert_eq!(
            got, want,
            "{backend}: v_bfe_u32({:#010x}, {}, {}) = {got:#010x}, want {want:#010x}",
            inp[0], inp[1], inp[2],
        );
    });
}

#[test]
fn v_cmp_class_f16_matches_rdna2() {
    // v_cmp_class_f16_e64 s6, v1, v2 (VOP3 opcode 0x08f); v_cndmask_b32_e64 v0, 0, 1, s6
    let cmp_lo = build_valu_cs(&[
        vop3(0x08f, 6, vgpr(1), vgpr(2), 0)[0],
        vop3(0x08f, 6, vgpr(1), vgpr(2), 0)[1],
        vop3(0x101, 0, 0x80, 0x81, sgpr(6))[0],
        vop3(0x101, 0, 0x80, 0x81, sgpr(6))[1],
    ]);
    // Same with op_sel:[1,0] (high half of v1) + neg on src0 (bit 29 of w1).
    let [mut w0, mut w1] = vop3(0x08f, 6, vgpr(1), vgpr(2), 0);
    w0 |= 1 << 11; // opsel[0] = 1 (high half)
    w1 |= 1 << 29; // neg[0] = 1
    let cmp_hi_neg = build_valu_cs(&[
        w0,
        w1,
        vop3(0x101, 0, 0x80, 0x81, sgpr(6))[0],
        vop3(0x101, 0, 0x80, 0x81, sgpr(6))[1],
    ]);

    // All 10 IEEE-754 binary16 classes: bit k corresponds to class k.
    let classes: &[(u32, u32)] = &[
        (0x7C01, 0), // sNaN (+sNaN, quiet bit 0x0200 clear)
        (0xFDFF, 0), // -sNaN (sign ignored for NaNs)
        (0x7E00, 1), // +qNaN
        (0xFE01, 1), // -qNaN
        (0xFC00, 2), // -inf
        (0xBC00, 3), // -normal (-1.0)
        (0x8001, 4), // -denormal
        (0x8000, 5), // -0.0
        (0x0000, 6), // +0.0
        (0x0001, 7), // +denormal
        (0x3C00, 8), // +normal (+1.0)
        (0x7C00, 9), // +inf
    ];
    let mut cases = Vec::new();
    for &(bits, cls) in classes {
        // Exact class bit -> true; complement of class bit across all 10 bits -> false.
        cases.push(([bits, 1u32 << cls, 0, 0], 1u32));
        cases.push(([bits, 0x3FF & !(1u32 << cls), 0, 0], 0u32));
    }
    let inputs: Vec<[u32; 4]> = cases.iter().map(|c| c.0).collect();
    run_cases(&cmp_lo, &inputs, |backend, i, inp, got| {
        let want = cases[i].1;
        assert_eq!(
            got, want,
            "{backend}: v_cmp_class_f16({:#06x}, {:#05x}) = {got}, want {want}",
            inp[0], inp[1],
        );
    });

    // High half with neg: high half 0x3C00 (+1.0) negated -> -normal (class 3);
    // high half 0x7C01 (+sNaN) negated -> still sNaN (class 0, not quieted).
    let hi_cases: &[([u32; 4], u32)] = &[
        ([0x3C00_0000, 1 << 3, 0, 0], 1),
        ([0x3C00_0000, 1 << 8, 0, 0], 0),
        ([0x7C01_FFFF, 1 << 0, 0, 0], 1),
        ([0x7C01_FFFF, 1 << 1, 0, 0], 0),
    ];
    let hi_in: Vec<[u32; 4]> = hi_cases.iter().map(|c| c.0).collect();
    run_cases(&cmp_hi_neg, &hi_in, |backend, i, inp, got| {
        let want = hi_cases[i].1;
        assert_eq!(
            got, want,
            "{backend}: v_cmp_class_f16(-hi({:#010x}), {:#05x}) = {got}, want {want}",
            inp[0], inp[1],
        );
    });
}

const S_BARRIER: u32 = 0xBF8A_0000;

/// VOP2 `op vdst, src0, vsrc1`.
const fn vop2(op: u8, vdst: u8, src0: u16, vsrc1: u8) -> u32 {
    (op as u32 & 0x3F) << 25 | (vdst as u32) << 17 | (vsrc1 as u32) << 9 | (src0 as u32 & 0x1FF)
}

/// DS `op vdst, addr, data0` at offset 0.
const fn ds(op: u8, vdst: u8, addr: u8, data0: u8) -> [u32; 2] {
    [
        0x36 << 26 | (op as u32) << 18,
        (vdst as u32) << 24 | (data0 as u32) << 8 | addr as u32,
    ]
}

const DS_WRITE_B32: u8 = 0x0D;
const DS_MIN_F32: u8 = 0x12;
const DS_MAX_F32: u8 = 0x13;
const DS_MIN_RTN_F32: u8 = 0x32;
const DS_MAX_RTN_F32: u8 = 0x33;
const DS_READ_B32: u8 = 0x36;

/// What RDNA2 writes for `ds_max_f32` / `ds_min_f32` over a dword holding
/// `old` with `data` (bits), as measured on gfx1036 with f32 denormals kept:
/// an sNaN, data first, then memory, is written quieted; otherwise a NaN
/// loses to a number (two quiet NaNs give the data); numbers compare as
/// floats with -0 below +0.
fn lds_min_max_ref(old: u32, data: u32, max: bool) -> u32 {
    let nan = |x: u32| x & 0x7FFF_FFFF > 0x7F80_0000;
    let snan = |x: u32| nan(x) && x & 0x40_0000 == 0;
    let key = |x: u32| {
        let b = x as i32;
        if b < 0 { b ^ 0x7FFF_FFFF } else { b }
    };
    if snan(data) {
        return data | 0x40_0000;
    }
    if snan(old) {
        return old | 0x40_0000;
    }
    match (nan(data), nan(old)) {
        (true, true) | (false, true) => return data,
        (true, false) => return old,
        (false, false) => {}
    }
    let take = if max {
        key(data) > key(old)
    } else {
        key(data) < key(old)
    };
    if take { data } else { old }
}

#[test]
fn ds_min_max_f32_order_floats_and_return_the_old_value() {
    let values = [
        0.0f32,
        -0.0,
        1.0,
        -1.0,
        2.5,
        -2.5,
        1e-30,
        -1e-30,
        f32::from_bits(1), // denormal
        f32::from_bits(0x8000_0001),
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::MAX,
        f32::MIN,
    ];
    // Quiet and signalling NaNs of both signs, in memory and as data.
    let nans = [
        0x7FC0_0000u32,
        0xFFC0_0000,
        0x7F80_0001,
        0xFFA0_0000,
        0x7FBF_FFFF,
    ];
    let bits: Vec<u32> = values.iter().map(|v| v.to_bits()).chain(nans).collect();
    let mut cases: Vec<[u32; 4]> = Vec::new();
    for &old in &bits {
        for &data in &bits {
            cases.push([old, data, 0, 0]);
        }
    }
    for (rtn, plain, max, what) in [
        (DS_MAX_RTN_F32, DS_MAX_F32, true, "ds_max"),
        (DS_MIN_RTN_F32, DS_MIN_F32, false, "ds_min"),
    ] {
        // v5 = 0; LDS[v5] = v1; the atomic with v2; v0 = the old value
        // (rtn) or the dword afterwards (read back).
        let mut returns_old = vec![vop1(0x001, 5, 0x80)];
        returns_old.extend(ds(DS_WRITE_B32, 0, 5, 1));
        returns_old.extend(ds(rtn, 0, 5, 2));
        let mut returns_new = vec![vop1(0x001, 5, 0x80)];
        returns_new.extend(ds(DS_WRITE_B32, 0, 5, 1));
        returns_new.extend(ds(plain, 0, 5, 2));
        returns_new.extend(ds(DS_READ_B32, 0, 5, 0));
        for (body, old_result) in [(returns_old, true), (returns_new, false)] {
            // One thread with LDS declares WaveSize(32), which WARP's
            // pipeline creation refuses: hardware only (the 64-thread test
            // below runs on WARP too).
            let shader = build_cs(&body, 1, 512);
            run_cases_on(
                &shader,
                &cases,
                &[("hardware", false)],
                |backend, _, inp, got| {
                    let (old, data) = (f32::from_bits(inp[0]), f32::from_bits(inp[1]));
                    let want = if old_result {
                        inp[0]
                    } else {
                        lds_min_max_ref(inp[0], inp[1], max)
                    };
                    assert_eq!(
                        got,
                        want,
                        "{backend}: {what}_f32 on {old:e} [{:#010x}] with {data:e} [{:#010x}] gave {got:#010x}, want {want:#010x} ({})",
                        inp[0],
                        inp[1],
                        if old_result {
                            "old value"
                        } else {
                            "dword after"
                        },
                    );
                },
            );
        }
    }
}

#[test]
fn ds_min_max_f32_are_atomic_across_waves() {
    // 64 threads (two host waves of 32 or one of 64): LDS[0] = v1, then
    // every thread applies (thread - 32) as a float; v0 = LDS[0].
    for (op, max, init, want) in [
        (DS_MAX_F32, true, f32::NEG_INFINITY, 31.0f32),
        (DS_MAX_F32, true, 100.0, 100.0),
        (DS_MIN_F32, false, f32::INFINITY, -32.0),
        (DS_MIN_F32, false, -0.0, -32.0),
    ] {
        let mut body = vec![vop1(0x001, 5, 0x80)];
        body.extend(ds(DS_WRITE_B32, 0, 5, 1));
        body.push(S_BARRIER);
        body.push(vop2(0x26, 6, vgpr(0), 3)); // v_sub_nc_u32 v6, v0, v3
        body.push(vop1(0x005, 6, vgpr(6))); // v_cvt_f32_i32
        body.extend(ds(op, 0, 5, 6));
        body.push(S_BARRIER);
        body.extend(ds(DS_READ_B32, 0, 5, 0));
        let shader = build_cs(&body, 64, 512);
        run_cases(
            &shader,
            &[[init.to_bits(), 0, 32, 0]],
            |backend, _, _, got| {
                assert_eq!(
                    f32::from_bits(got),
                    want,
                    "{backend}: {} from {init} over 64 threads",
                    if max { "max" } else { "min" },
                );
            },
        );
    }
}

/// Runs VOP1 `op` on each input and compares the result dword.
fn check_cvt(name: &str, op: u16, cases: &[(u32, u32)]) {
    let shader = build_valu_cs(&[vop1(op, 0, vgpr(1))]);
    let inputs: Vec<[u32; 4]> = cases.iter().map(|c| [c.0, 0, 0, 0]).collect();
    run_cases(&shader, &inputs, |backend, i, inp, got| {
        let want = cases[i].1;
        assert_eq!(
            got,
            want,
            "{backend}: {name}({:#010x} = {:e}) = {got:#010x}, want {want:#010x}",
            inp[0],
            f32::from_bits(inp[0]),
        );
    });
}

#[test]
fn v_cvt_rpi_i32_f32_rounds_to_nearest() {
    // floor(x + 0.5) of the exact sum: an f32 sum rounds to even first, so
    // it is 1 too high just below 0.5 and for odd integers from 2^23 up.
    check_cvt(
        "v_cvt_rpi_i32_f32",
        0x00C,
        &[
            (0x0000_0000, 0),
            (0x3EFF_FFFF, 0), // 0.5 - 2^-25
            (0x3F00_0000, 1), // 0.5: ties go up
            (0xBF00_0000, 0), // -0.5
            (0xBF00_0001, 0xFFFF_FFFF),
            (0x3FC0_0000, 2),           // 1.5
            (0x4020_0000, 3),           // 2.5
            (0xBFC0_0000, 0xFFFF_FFFF), // -1.5
            (0xC020_0000, 0xFFFF_FFFE), // -2.5
            (0x0000_0001, 0),           // denormals, flushed or not
            (0x8000_0001, 0),
            (0x4A80_0001, 0x0040_0001), // 2^22 + 0.5
            (0x4B00_0001, 0x0080_0001), // 2^23 + 1
            (0x4B7F_FFFF, 0x00FF_FFFF), // 2^24 - 1
            (0xCB00_0001, 0xFF7F_FFFF), // -(2^23 + 1)
            (0xCB7F_FFFF, 0xFF00_0001),
            (0x4EFF_FFFF, 0x7FFF_FF80),
            (0x4F00_0000, 0x7FFF_FFFF), // 2^31 and up saturate
            (0x7F80_0000, 0x7FFF_FFFF),
            (0xCF00_0000, 0x8000_0000),
            (0xCF00_0001, 0x8000_0000),
            (0xFF80_0000, 0x8000_0000),
        ],
    );
}

#[test]
fn v_cvt_flr_i32_f32_rounds_down() {
    check_cvt(
        "v_cvt_flr_i32_f32",
        0x00D,
        &[
            (0x0000_0000, 0),
            (0x8000_0000, 0),
            (0x3FC0_0000, 1),           // 1.5
            (0xBFC0_0000, 0xFFFF_FFFE), // -1.5
            (0x3F7F_FFFF, 0),
            (0xBF80_0000, 0xFFFF_FFFF), // -1
            (0xB300_0000, 0xFFFF_FFFF), // -2^-25
            (0x4B7F_FFFF, 0x00FF_FFFF),
            (0x4F00_0000, 0x7FFF_FFFF),
            (0x7F80_0000, 0x7FFF_FFFF),
            (0xCF00_0001, 0x8000_0000),
            (0xFF80_0000, 0x8000_0000),
        ],
    );
}

#[test]
fn v_cvt_flr_and_rpi_i32_f32_saturate_nan_by_sign() {
    // Unlike v_cvt_i32_f32 (NaN gives 0), these saturate whatever has an
    // exponent above 30, a NaN too: to the largest or the smallest integer
    // by its sign.
    let cases = [
        (0x7FC0_0000, 0x7FFF_FFFF),
        (0x7F80_0001, 0x7FFF_FFFF),
        (0x7FFF_FFFF, 0x7FFF_FFFF),
        (0xFFC0_0000, 0x8000_0000),
        (0xFF80_0001, 0x8000_0000),
        (0xFFFF_FFFF, 0x8000_0000),
    ];
    check_cvt("v_cvt_rpi_i32_f32", 0x00C, &cases);
    check_cvt("v_cvt_flr_i32_f32", 0x00D, &cases);
    // v_cvt_i32_f32 keeps giving 0.
    check_cvt(
        "v_cvt_i32_f32",
        0x008,
        &[(0x7FC0_0000, 0), (0xFFC0_0000, 0), (0x7F80_0001, 0)],
    );
}

/// A 16-bit snorm (`signed`) or unorm of `x`, as the RDNA2 ISA reference
/// gives `v_cvt_pknorm_*`: clamped, scaled, rounded to nearest even from the
/// exact product (exact in f64), NaN as 0.
fn norm16_ref(x: f32, signed: bool) -> u16 {
    if x.is_nan() {
        return 0;
    }
    let (lo, k) = if signed {
        (-1.0, 32767.0)
    } else {
        (0.0, 65535.0)
    };
    let n = (f64::from(x.clamp(lo, 1.0)) * k).round_ties_even() as i32;
    n as u16
}

/// Inputs in [0.5, 1) and near 0.001 whose f32 product with the scale lands
/// exactly on a tie although the exact product does not: a translation that
/// multiplies in f32 rounds them the wrong way.
fn f32_tie_traps(signed: bool) -> Vec<f32> {
    let k: f32 = if signed { 32767.0 } else { 65535.0 };
    let mut traps = Vec::new();
    for start in [0.5f32, 0.001] {
        let first = start.to_bits();
        for b in first..first + 4_000_000 {
            let x = f32::from_bits(b);
            let p = x * k;
            if p.fract() == 0.5 && f64::from(x) * f64::from(k) != f64::from(p) {
                traps.push(x);
                if traps.len() % 3 == 0 {
                    break;
                }
            }
        }
    }
    traps
}

fn pknorm_f32_inputs(signed: bool) -> Vec<f32> {
    let mut xs = vec![
        0.0,
        -0.0,
        1.0,
        -1.0,
        2.0,
        -2.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
        -f32::NAN,
        0.5,
        -0.5,
        0.25,
        1.0 / 3.0,
        -0.7,
        0.999_99,
        f32::MIN_POSITIVE,
        1e-6,
        -1e-6,
        f32::from_bits(1), // denormal
        1.0 / 65535.0,
        0.5 / 65535.0,
        1.5 / 32767.0,
        f32::from_bits(0x3F7F_FFFF), // just below 1
    ];
    xs.extend(f32_tie_traps(signed));
    xs
}

#[test]
fn v_cvt_pknorm_f32_rounds_the_exact_product() {
    for (op, signed, what) in [
        (0x368u16, true, "v_cvt_pknorm_i16_f32"),
        (0x369, false, "v_cvt_pknorm_u16_f32"),
    ] {
        // The traps must tell an f32 product apart from the exact one.
        let k: f32 = if signed { 32767.0 } else { 65535.0 };
        let f32_product = |x: f32| (x * k).round_ties_even() as i32 as u16;
        let traps = f32_tie_traps(signed);
        assert!(
            traps
                .iter()
                .any(|&x| f32_product(x) != norm16_ref(x, signed)),
            "{what}: no input separates an f32 product from the exact one"
        );
        let shader = build_valu_cs(&vop3(op, 0, vgpr(1), vgpr(2), 0));
        let xs = pknorm_f32_inputs(signed);
        // Each input once in each half, paired with its neighbour.
        let cases: Vec<[u32; 4]> = (0..xs.len())
            .map(|i| [xs[i].to_bits(), xs[(i + 1) % xs.len()].to_bits(), 0, 0])
            .collect();
        run_cases(&shader, &cases, |backend, _, inp, got| {
            let (a, b) = (f32::from_bits(inp[0]), f32::from_bits(inp[1]));
            let want = u32::from(norm16_ref(a, signed)) | u32::from(norm16_ref(b, signed)) << 16;
            assert_eq!(
                got, want,
                "{backend}: {what}({a:e} [{:#010x}], {b:e} [{:#010x}]) = {got:#010x}, want {want:#010x}",
                inp[0], inp[1],
            );
        });
    }
}

#[test]
fn v_cvt_pknorm_f16_rounds_the_exact_product() {
    let halves: Vec<u16> = vec![
        0x0000, 0x8000, 0x3C00, 0xBC00, 0x4000, 0xC000, 0x7C00, 0xFC00, 0x7E00, 0xFE00,
        0x3800, // 0.5: a tie in both scales
        0xB800, 0x3555, 0x0001, // smallest denormal
        0x8001, 0x0400, // smallest normal
        0x3BFF, // just below 1
        0x1001, 0x2E66, 0xB266, 0x3A00, 0x3401,
        // Large denormals, which scale to 1 or 2: kept, not flushed (the
        // FP16 input-denormal mode RDNA2 measurements matched).
        0x03D6, 0x83D6, 0x823F, 0x03FF,
    ];
    // Measured on RDNA2 (gfx1036) with FP16 input denormals kept.
    assert_eq!(norm16_ref(f16_to_f32(0x83D6), true), 0xFFFE);
    assert_eq!(norm16_ref(f16_to_f32(0x823F), true), 0xFFFF);
    for (op, signed, what) in [
        (0x312u16, true, "v_cvt_pknorm_i16_f16"),
        (0x313, false, "v_cvt_pknorm_u16_f16"),
    ] {
        let shader = build_valu_cs(&vop3(op, 0, vgpr(1), vgpr(2), 0));
        let cases: Vec<[u32; 4]> = (0..halves.len())
            .map(|i| {
                let next = halves[(i + 1) % halves.len()];
                // Junk in the high halves: only the low halves are read.
                [
                    u32::from(halves[i]) | 0xABCD_0000,
                    u32::from(next) | 0x1234_0000,
                    0,
                    0,
                ]
            })
            .collect();
        run_cases(&shader, &cases, |backend, _, inp, got| {
            let (a, b) = (inp[0] as u16, inp[1] as u16);
            let (fa, fb) = (f16_to_f32(a), f16_to_f32(b));
            let want = u32::from(norm16_ref(fa, signed)) | u32::from(norm16_ref(fb, signed)) << 16;
            assert_eq!(
                got, want,
                "{backend}: {what}({a:#06x}, {b:#06x}) = {got:#010x}, want {want:#010x}",
            );
        });
    }
}

/// MTBUF `op vdata, v5, s[8:11], 0 format:fmt offset:16` (no offen/idxen;
/// `op` 4..7 stores x..xyzw, 12..15 their d16 forms).
const fn mtbuf_store(op: u8, fmt: u8, vdata: u8) -> [u32; 2] {
    [
        0x3A << 26 | (fmt as u32) << 19 | (op as u32 & 7) << 16 | 16,
        0x80 << 24 | (op as u32 >> 3 & 1) << 21 | (8 >> 2) << 16 | (vdata as u32) << 8 | 5,
    ]
}

/// Runs `shader` once with root constants `inputs` on every available
/// SM 6.6 host and returns the first `dwords` of its raw UAV, per device.
fn run_dwords(shader: &HostShader, inputs: [u32; 4], dwords: u32) -> Vec<(&'static str, Vec<u32>)> {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = Vec::new();
    for (label, warp) in [("hardware", false), ("warp", true)] {
        let Ok(host) = Host::new(warp) else {
            eprintln!("no SM 6.6 {label} device; skipped");
            continue;
        };
        let (root, pso) = host
            .compute_pipeline(shader)
            .unwrap_or_else(|e| panic!("{label}: compute_pipeline failed: {e}"));
        let mut ud = vec![0u32; shader.user_data_dwords().max(1) as usize];
        for u in &shader.user_data {
            ud[u.index as usize] = match (u.root, u.slot) {
                (RootType::Constants, 0) => inputs[u.dword as usize],
                (RootType::Table, 1) => 0,
                other => panic!("unexpected user data {other:?}"),
            };
        }
        let r = host
            .dispatch(
                &root,
                &pso,
                &Dispatch {
                    roots: vec![(0, Root::Constants(ud))],
                    views: &[View::RawUav(dwords)],
                    samplers: &[],
                    groups: [1, 1, 1],
                },
            )
            .unwrap_or_else(|e| panic!("{label}: dispatch failed: {e}"));
        out.push((label, r[0].clone()));
    }
    assert!(
        !out.is_empty(),
        "neither hardware nor WARP SM 6.6 device available"
    );
    out
}

#[test]
fn tbuffer_store_format_writes_the_formats_layout() {
    let f = |x: f32| x.to_bits();
    // (what, op, format, data in v1..v4, dwords 4..8 afterwards)
    type Case = (&'static str, u8, u8, [u32; 4], [u32; 4]);
    let cases: &[Case] = &[
        (
            "32_32_32_32 float xyzw",
            7,
            77,
            [f(1.5), f(-0.0), 0x7FC0_1234, 0x0000_0001],
            [f(1.5), f(-0.0), 0x7FC0_1234, 0x0000_0001],
        ),
        (
            "32_32_32_32 sint xyzw",
            7,
            76,
            [u32::MAX, 0x8000_0000, 7, 0x1234_5678],
            [u32::MAX, 0x8000_0000, 7, 0x1234_5678],
        ),
        (
            "32_32_32 float xyz",
            6,
            74,
            [f(2.0), f(3.0), f(4.0), 0xDEAD_BEEF],
            [f(2.0), f(3.0), f(4.0), 0],
        ),
        (
            "32_32 uint xy",
            5,
            63,
            [0xAABB_CCDD, 9, 0xDEAD_BEEF, 0xDEAD_BEEF],
            [0xAABB_CCDD, 9, 0, 0],
        ),
        (
            "32 float x",
            4,
            22,
            [f(-7.25), 1, 2, 3],
            [f(-7.25), 0, 0, 0],
        ),
        (
            // d16: the halves are stored as they are.
            "16_16 float d16_xy",
            13,
            29,
            [0xBC00_3555, 0xDEAD_BEEF, 0, 0],
            [0xBC00_3555, 0, 0, 0],
        ),
        (
            "16_16_16_16 float d16_xyzw",
            15,
            71,
            [0x1234_5678, 0x7C00_FC00, 0, 0],
            [0x1234_5678, 0x7C00_FC00, 0, 0],
        ),
    ];
    for &(what, op, fmt, inputs, want) in cases {
        let mut body = vec![vop1(0x001, 5, 0x80)]; // v_mov_b32 v5, 0
        body.extend(mtbuf_store(op, fmt, 1));
        let shader = build_valu_cs(&body);
        for (backend, out) in run_dwords(&shader, inputs, 8) {
            assert_eq!(
                out[4..8],
                want,
                "{backend}: tbuffer_store {what}: wrote {:#010x?}, want {want:#010x?}",
                &out[4..8],
            );
        }
    }
}

#[test]
fn tbuffer_store_format_refuses_what_it_does_not_convert() {
    for (what, op, fmt, reason) in [
        ("8_8_8_8 unorm", 7, 56, "a converted or narrow format"),
        ("16_16 uint", 5, 27, "a converted or narrow format"),
        ("16 float", 4, 13, "an element under a dword"),
        (
            "32_32_32_32 float, xy only",
            5,
            77,
            "some of its format's channels",
        ),
        ("32 float d16_x", 12, 22, "a converted or narrow format"),
        // f32 to f16 waits for nearest-even narrowing (#422).
        ("16_16 float xy", 5, 29, "a converted or narrow format"),
        (
            "16_16_16_16 float xyzw",
            7,
            71,
            "a converted or narrow format",
        ),
    ] {
        let mut body = vec![vop1(0x001, 5, 0x80)];
        body.extend(mtbuf_store(op, fmt, 1));
        match try_build_valu_cs(&body) {
            Ok(_) => panic!("tbuffer_store {what} lowered; it should be refused"),
            Err(e) => assert!(e.contains(reason), "tbuffer_store {what}: refused with {e}"),
        }
    }
}

/// Contributed by @aziaconsal2026 on the pull request: the vector (`offen`)
/// and scalar (`soffset`) byte offsets add to the immediate one, and the
/// neighbouring dwords stay as they were.
#[test]
fn tbuffer_store_adds_scalar_and_vector_byte_offsets() {
    // v1 = payload, v2 = vector byte offset, s2 = scalar byte offset,
    // v4 = sentinel. Fill the window first so stray writes are observable.
    let mut body = Vec::new();
    for offset in (16..44).step_by(4) {
        body.extend(buffer_store_dword(4, 8, offset));
    }
    body.push(vop1(0x001, 5, vgpr(2))); // v_mov_b32 v5, v2
    let [opcode, operands] = mtbuf_store(4, 20, 1); // 32_UINT, immediate byte offset 16
    body.extend([opcode | (1 << 12), (operands & 0x00ff_ffff) | (2 << 24)]); // offen, soffset = s2
    let shader = build_valu_cs(&body);

    let payload = 0x1357_9bdf;
    let sentinel = 0xa5a5_a5a5;
    for (vector, scalar) in [(0, 0), (4, 0), (0, 8), (4, 8), (12, 8)] {
        let mut expected = [sentinel; 7];
        expected[((vector + scalar) / 4) as usize] = payload;
        for (backend, words) in run_dwords(&shader, [payload, vector, scalar, sentinel], 11) {
            assert_eq!(
                &words[4..],
                &expected,
                "{backend}: vector={vector} scalar={scalar}"
            );
        }
    }
}

/// One wave of 32 lanes holding 1..=32 (the input of the ISA's examples)
/// through `ds_swizzle_b32` with `offset`: what each lane reads.
fn swizzle_32(offset: u16) -> Vec<Vec<u32>> {
    let mut words = vec![s_mov_b32(5, 0x80)];
    words.extend_from_slice(&s_load_dwordx4(8, 4));
    words.push(vop2(0x25, 1, 129, 0)); // v_add_nc_u32 v1, 1, v0
    words.extend_from_slice(&ds_swizzle_b32(2, 1, offset));
    words.push(vop2(0x1A, 3, 130, 0)); // v_lshlrev_b32 v3, 2, v0
    words.extend_from_slice(&buffer_store_dword_offen(2, 3, 8));
    words.push(S_ENDPGM);
    let shader = build_raw_cs(&words, 32);

    let mut out = Vec::new();
    for (label, warp) in [("hardware", false), ("warp", true)] {
        let Ok(host) = Host::new(warp) else {
            eprintln!("no SM 6.6 {label} device; skipped");
            continue;
        };
        let (root, pso) = match host.compute_pipeline(&shader) {
            Ok(p) => p,
            // A host without 32-lane waves cannot run it.
            Err(e) if warp => {
                eprintln!("{label}: no pipeline ({e}); skipped");
                continue;
            }
            Err(e) => panic!("{label}: compute_pipeline failed: {e}"),
        };
        let ud = vec![0u32; shader.user_data_dwords().max(1) as usize];
        let uavs = host
            .dispatch(
                &root,
                &pso,
                &Dispatch {
                    roots: vec![(0, Root::Constants(ud))],
                    views: &[View::RawUav(32)],
                    samplers: &[],
                    groups: [1, 1, 1],
                },
            )
            .unwrap_or_else(|e| panic!("{label}: dispatch failed: {e}"));
        out.push(uavs[0].clone());
    }
    if out.is_empty() {
        eprintln!("no SM 6.6 device with 32-lane waves; skipped");
    }
    out
}

/// The basic modes: a quad permutation and a 32-lane and/or/xor mapping.
#[test]
fn ds_swizzle_basic_modes() {
    // offset[15]: lanes of each quad read lanes 3, 2, 1, 0 of it.
    let reversed: Vec<u32> = (0..32).map(|i| (i ^ 3) + 1).collect();
    // and 0x1f, or 0, xor 0x10: the halves swap.
    let swapped: Vec<u32> = (0..32).map(|i| (i ^ 16) + 1).collect();
    for (offset, want) in [(0x801B, reversed), (0x401F, swapped)] {
        for got in swizzle_32(offset) {
            assert_eq!(got, want, "offset {offset:#06x}");
        }
    }
}

/// Offsets from 0xE000: the FFT orders, as the ISA lists them for inputs
/// 1..=0x20 (RDNA2 ISA, 12.13.1).
#[test]
fn ds_swizzle_fft_mode() {
    let cases: [(u16, [u32; 32]); 3] = [
        (
            0xE000,
            [
                0x01, 0x11, 0x09, 0x19, 0x05, 0x15, 0x0d, 0x1d, 0x03, 0x13, 0x0b, 0x1b, 0x07, 0x17,
                0x0f, 0x1f, 0x02, 0x12, 0x0a, 0x1a, 0x06, 0x16, 0x0e, 0x1e, 0x04, 0x14, 0x0c, 0x1c,
                0x08, 0x18, 0x10, 0x20,
            ],
        ),
        (
            0xE010,
            [
                0x01, 0x09, 0x05, 0x0d, 0x03, 0x0b, 0x07, 0x0f, 0x02, 0x0a, 0x06, 0x0e, 0x04, 0x0c,
                0x08, 0x10, 0x11, 0x19, 0x15, 0x1d, 0x13, 0x1b, 0x17, 0x1f, 0x12, 0x1a, 0x16, 0x1e,
                0x14, 0x1c, 0x18, 0x20,
            ],
        ),
        // 0x1f: no swizzle.
        (0xE01F, std::array::from_fn(|i| i as u32 + 1)),
    ];
    for (offset, want) in cases {
        for got in swizzle_32(offset) {
            assert_eq!(got, want, "offset {offset:#06x}");
        }
    }
    // The other bits of an FFT offset do not matter.
    for got in swizzle_32(0xFFF0) {
        assert_eq!(got, cases[1].1, "offset 0xfff0");
    }
}

/// Offsets from 0xC000 to 0xDFFF: the wave's 32 lanes rotate left by
/// offset[9:5], or right with offset[10] (RDNA2 ISA, 12.13.1).
#[test]
fn ds_swizzle_rotate_mode() {
    let rotated = |by: u32| -> Vec<u32> { (0..32).map(|i| (i + by) % 32 + 1).collect() };
    for (offset, by) in [
        (0xC000 | 1 << 5, 1),            // the ISA's example: {2, 3, ... 0x20, 1}
        (0xC000 | 1 << 10 | 1 << 5, 31), // and {0x20, 1, ... 0x1f}
        (0xC000, 0),
        (0xC000 | 5 << 5, 5),
        (0xC000 | 1 << 10 | 5 << 5, 27),
        (0xD800 | 31 << 5, 31), // bits 11 and 12 are not read
    ] {
        for got in swizzle_32(offset) {
            assert_eq!(got, rotated(by), "offset {offset:#06x}");
        }
    }
}

/// Rotations with a mask (offset[4:0]): the mask's bits of the lane index
/// stay, the others rotate, as the ISA's pseudocode has it. Its examples for
/// mask 1 list other lanes; the lanes here are the ones measured on RDNA2
/// hardware (gfx1036, by Lyrnc in Marvisak/VIctory#606, who gives the first
/// and last entries of each list and reports the pseudocode matching all 2048
/// offsets 0xC000-0xC7FF, and bits 11 and 12 being ignored).
#[test]
fn ds_swizzle_masked_rotations() {
    // The lane each lane reads.
    let cases: [(u16, [u32; 32]); 4] = [
        // Left by 1, mask 1: 0, 3, 2, 5, 4, 7, 6, 9, 8, ..., 28, 31, 30, 1.
        (
            0xC021,
            [
                0, 3, 2, 5, 4, 7, 6, 9, 8, 11, 10, 13, 12, 15, 14, 17, 16, 19, 18, 21, 20, 23, 22,
                25, 24, 27, 26, 29, 28, 31, 30, 1,
            ],
        ),
        // Right by 1, mask 1: 30, 1, 0, 3, 2, 5, 4, ...
        (
            0xC421,
            [
                30, 1, 0, 3, 2, 5, 4, 7, 6, 9, 8, 11, 10, 13, 12, 15, 14, 17, 16, 19, 18, 21, 20,
                23, 22, 25, 24, 27, 26, 29, 28, 31,
            ],
        ),
        // Left by 1, mask 3: 0, 1, 2, 7, 4, 5, 6, 11, ...
        (
            0xC023,
            [
                0, 1, 2, 7, 4, 5, 6, 11, 8, 9, 10, 15, 12, 13, 14, 19, 16, 17, 18, 23, 20, 21, 22,
                27, 24, 25, 26, 31, 28, 29, 30, 3,
            ],
        ),
        // Bits 11 and 12 set: as 0xC021.
        (
            0xD821,
            [
                0, 3, 2, 5, 4, 7, 6, 9, 8, 11, 10, 13, 12, 15, 14, 17, 16, 19, 18, 21, 20, 23, 22,
                25, 24, 27, 26, 29, 28, 31, 30, 1,
            ],
        ),
    ];
    for (offset, lanes) in cases {
        // Lane i holds i + 1.
        let want: Vec<u32> = lanes.iter().map(|j| j + 1).collect();
        for got in swizzle_32(offset) {
            assert_eq!(got, want, "offset {offset:#06x}");
        }
    }
}
