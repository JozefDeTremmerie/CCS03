//! Lowering added in CCS03: its own pull requests and those it took from
//! other forks, kept out of `lower.rs` so that file stays under the asset
//! policy's size limit. A child module of `lower`, so it shares its state.

use super::*;

impl Lower<'_> {
    /// Whether a read of another lane of this lane's quad (a quad swizzle
    /// or a quad permutation) must reach helper lanes: in a pixel shader,
    /// always. At a triangle's edge the quad's other pixels are helpers,
    /// which only quad reads see; a wave read of a helper lane gives
    /// nothing defined. A derivative the title takes by hand then was the
    /// whole value, the texture read that used it took its smallest level,
    /// and triangles' edges showed as lines on car paint, glass and skin.
    /// After Belobug/VIctory-Belobug-AMD 03629f3, whose lowering this
    /// tree's does not share (the two quad sites only).
    pub(super) fn helpers_read(&self, quad: bool) -> bool {
        quad && self.s.pixel
    }

    /// `x * 2^e` for any i32 `e`, rounded once unless the result is a
    /// denormal. DXIL has no ldexp: `x` is multiplied by three powers of
    /// two that are each a normal float. Past ±375 any finite nonzero `x`
    /// is already at zero or infinity, so `e` is clamped to [-375, 378],
    /// which keeps each of q, q and e - 2q (q = e / 3) inside [-126, 127].
    pub(super) fn ldexp(&mut self, x: V, e: V) -> V {
        let c = self.cur;
        let (lo, hi) = (self.s.m.i32(-375i32 as u32), self.s.m.i32(378));
        let below = self.s.m.cmp(c, pred::SLT, e, lo);
        let e = self.s.m.select(c, below, lo, e);
        let above = self.s.m.cmp(c, pred::SGT, e, hi);
        let e = self.s.m.select(c, above, hi, e);
        let three = self.s.m.i32(3);
        let q = self.s.m.bin(c, Bin::SDiv, e, three);
        let two_q = self.s.m.bin(c, Bin::Add, q, q);
        let rest = self.s.m.bin(c, Bin::Sub, e, two_q);
        let mut r = x;
        for k in [q, q, rest] {
            // 2^k for k in [-126, 127]: the biased exponent alone.
            let (bias, at) = (self.s.m.i32(127), self.s.m.i32(23));
            let biased = self.s.m.bin(c, Bin::Add, k, bias);
            let bits = self.s.m.bin(c, Bin::Shl, biased, at);
            let factor = self.as_float(bits);
            r = self.s.m.bin(c, Bin::Mul, r, factor);
        }
        r
    }

    /// The significand (bits of a float with magnitude in [0.5, 1), the
    /// sign kept) and the exponent of the f32 with bits `x`, so that
    /// x = significand * 2^exponent, worked on the bits so that denormals
    /// are normalized whatever the host's float mode. Zero gives itself and
    /// 0; infinity and NaN give themselves and 0.
    pub(super) fn frexp(&mut self, x: V) -> (V, V) {
        let c = self.cur;
        let k = |l: &mut Self, v: u32| l.s.m.i32(v);
        let (at, ff) = (k(self, 23), k(self, 0xFF));
        let field = self.s.m.bin(c, Bin::LShr, x, at);
        let field = self.s.m.bin(c, Bin::And, field, ff);
        let frac_mask = k(self, 0x7F_FFFF);
        let frac = self.s.m.bin(c, Bin::And, x, frac_mask);
        let sign_mask = k(self, 0x8000_0000);
        let sign = self.s.m.bin(c, Bin::And, x, sign_mask);
        // A denormal's leading one at bit 31 - lz moves to bit 23.
        let lz = self.s.op(
            c,
            "unaryBits.i32",
            Attr::ReadNone,
            self.s.t.i32,
            33,
            &[frac],
        );
        let (eight, k31) = (k(self, 8), k(self, 31));
        let sh = self.s.m.bin(c, Bin::Sub, lz, eight);
        let sh = self.s.m.bin(c, Bin::And, sh, k31);
        let normalized = self.s.m.bin(c, Bin::Shl, frac, sh);
        let normalized = self.s.m.bin(c, Bin::And, normalized, frac_mask);
        let zero = k(self, 0);
        let denormal = self.s.m.cmp(c, pred::EQ, field, zero);
        let frac = self.s.m.select(c, denormal, normalized, frac);
        let half = k(self, 0x3F00_0000);
        let m = self.s.m.bin(c, Bin::Or, sign, half);
        let m = self.s.m.bin(c, Bin::Or, m, frac);
        // Exponents: field - 126, or -117 - lz for a denormal.
        let (k126, k117) = (k(self, 126), k(self, -117i32 as u32));
        let e_normal = self.s.m.bin(c, Bin::Sub, field, k126);
        let e_denormal = self.s.m.bin(c, Bin::Sub, k117, lz);
        let e = self.s.m.select(c, denormal, e_denormal, e_normal);
        // Zero, infinity and NaN.
        let magnitude = k(self, 0x7FFF_FFFF);
        let magnitude = self.s.m.bin(c, Bin::And, x, magnitude);
        let is_zero = self.s.m.cmp(c, pred::EQ, magnitude, zero);
        let special = self.s.m.cmp(c, pred::EQ, field, ff);
        let keep = self.s.m.bin(c, Bin::Or, is_zero, special);
        let m = self.s.m.select(c, keep, x, m);
        let e = self.s.m.select(c, keep, zero, e);
        (m, e)
    }

    /// Registers an immediate sampler constant and returns its slot index.
    pub(super) fn register_sampler(&mut self, words: &[u32], cmp: bool) -> Result<u32, Why> {
        let words: [u32; 4] = words
            .get(..4)
            .and_then(|w| w.try_into().ok())
            .ok_or("a sampler constant")?;
        let n = match self.samplers.iter().position(|w| *w == words) {
            Some(n) => n,
            None => {
                self.samplers.push(words);
                self.samplers.len() - 1
            }
        };
        if cmp && n < 64 {
            self.s.cmp_samplers |= 1 << n;
        }
        Ok(n as u32)
    }

    /// Resolves a control-flow phi of immediate sampler constants into either a
    /// single static sampler register (`One`) or a uniform `i32` phi index over
    /// `regs` (`Many`) for branch dispatch at the sample site.
    pub(super) fn sampler_phi_choice(
        &mut self,
        block: usize,
        args: &[Source],
        cmp: bool,
        depth: u32,
    ) -> Result<(Option<V>, Vec<u32>), Why> {
        const WHY: Why = "a descriptor chosen by control flow";
        let mut distinct: Vec<[u32; 4]> = Vec::new();
        self.collect_constant_samplers(block, args, depth, &mut distinct)?;
        if distinct.is_empty() {
            return Err(WHY);
        }
        let mut regs = Vec::with_capacity(distinct.len());
        for w in &distinct {
            regs.push(self.register_sampler(w, cmp)?);
        }
        if regs.len() == 1 {
            return Ok((None, regs));
        }
        let idx = self.sampler_index_phi(block, args, 0, &distinct, depth)?;
        Ok((Some(idx), regs))
    }

    pub(super) fn collect_constant_samplers(
        &self,
        block: usize,
        args: &[Source],
        depth: u32,
        distinct: &mut Vec<[u32; 4]>,
    ) -> Result<(), Why> {
        const WHY: Why = "a descriptor chosen by control flow";
        let p = self.x.p;
        for (&q, arg) in p.blocks[block].preds.iter().zip(args) {
            if self.block[q].is_none() {
                continue;
            }
            match arg {
                Source::Constant(w) if w.len() >= 4 => {
                    let k: [u32; 4] = w[..4].try_into().unwrap();
                    if !distinct.contains(&k) {
                        distinct.push(k);
                    }
                }
                Source::Phi {
                    block: inner,
                    args: inner_args,
                } if depth > 0 => {
                    self.collect_constant_samplers(*inner, inner_args, depth - 1, distinct)?;
                }
                _ => return Err(WHY),
            }
        }
        Ok(())
    }

    pub(super) fn sampler_index_phi(
        &mut self,
        block: usize,
        args: &[Source],
        lower: u32,
        distinct: &[[u32; 4]],
        depth: u32,
    ) -> Result<V, Why> {
        const WHY: Why = "a descriptor chosen by control flow";
        let p = self.x.p;
        let mut incoming = Vec::new();
        for (&q, arg) in p.blocks[block].preds.iter().zip(args) {
            if self.block[q].is_none() {
                continue;
            }
            let tail = self.tail[q].ok_or(WHY)?;
            let at = self.edges.get(&(q, block)).copied().unwrap_or(tail);
            let idx = match arg {
                Source::Constant(w) if w.len() >= 4 => {
                    let k: [u32; 4] = w[..4].try_into().unwrap();
                    let pos = distinct.iter().position(|d| *d == k).ok_or(WHY)? as u32;
                    self.s.m.i32(lower + pos)
                }
                Source::Phi {
                    block: inner,
                    args: inner_args,
                } if depth > 0 => {
                    self.sampler_index_phi(*inner, inner_args, lower, distinct, depth - 1)?
                }
                _ => return Err(WHY),
            };
            incoming.push((idx, at));
        }
        let join = self.block[block].ok_or(WHY)?;
        let phi = self.s.m.phi(join, self.s.t.i32);
        self.s.m.set_phi(phi, incoming);
        Ok(phi)
    }

    /// A 64-bit float source as its bits in an i64: a register pair, an
    /// inline constant (the float ones as f64 values, the integer ones
    /// sign-extended) or a literal (its high dword; the low one is 0), with
    /// `abs` and `neg` applied to the sign bit.
    pub(super) fn f64_src(&mut self, inst: &Inst, k: usize) -> Result<V, Why> {
        let Fields::Valu(v) = &inst.fields else {
            unreachable!()
        };
        if v.sdwa.is_some() || v.dpp.is_some() {
            return Err("64-bit operand modifiers");
        }
        let c = self.cur;
        let i64t = self.s.m.int(64);
        self.s.flags |= flags::INT64;
        let op = *v.src.get(k).ok_or("a missing operand")?;
        let mut x = match op {
            Operand::Const(code @ 240..=248) => {
                let bits = f64_inline(code).ok_or("a special operand")?;
                self.s.m.int_const(i64t, bits as i64)
            }
            Operand::Literal(hi) => self.s.m.int_const(i64t, (u64::from(hi) << 32) as i64),
            Operand::Vector { reg, .. } => {
                let lo = self.i32_of(inst.read(Reg::V(reg)).ok_or("an unread operand")?)?;
                let hi = self.i32_of(inst.read(Reg::V(reg + 1)).ok_or("an unread operand")?)?;
                self.join64(lo, hi)
            }
            Operand::Scalar { code, .. } if code != NULL => {
                let lo = self.i32_of(inst.read(Reg::S(code)).ok_or("an unread operand")?)?;
                let hi = self.i32_of(inst.read(Reg::S(code + 1)).ok_or("an unread operand")?)?;
                self.join64(lo, hi)
            }
            Operand::Const(code @ 128..=208) => {
                let n = crate::resource::inline(code).ok_or("a special operand")?;
                self.s.m.int_const(i64t, i64::from(n as i32))
            }
            _ => return Err("a 64-bit float operand"),
        };
        let sign = self.s.m.int_const(i64t, i64::MIN);
        if v.abs >> k & 1 != 0 {
            let keep = self.s.m.int_const(i64t, i64::MAX);
            x = self.s.m.bin(c, Bin::And, x, keep);
        }
        if v.neg >> k & 1 != 0 {
            x = self.s.m.bin(c, Bin::Xor, x, sign);
        }
        Ok(x)
    }

    /// The bits of the f32 nearest an f64 (`x`, its bits in an i64), ties
    /// to even, as `v_cvt_f32_f64` rounds: what the free `f32_bits_of_f64`
    /// (a test's reference) does on the CPU, step for step. Denormal
    /// results are kept; f64 denormals come out as zeros of their sign; a
    /// NaN stays a NaN (quiet, its sign and its payload's high bits kept);
    /// past the largest f32 is infinity.
    pub(super) fn f32_bits_of_f64(&mut self, x: V) -> V {
        let c = self.cur;
        let m = &mut self.s.m;
        let i64t = m.int(64);
        let k = |m: &mut crate::emit::bitcode::Module, v: i64| m.int_const(i64t, v);
        let (k0, k1, k23, k29, k34, k52, k63) =
            (k(m, 0), k(m, 1), k(m, 23), k(m, 29), k(m, 34), k(m, 52), k(m, 63));
        let sign = m.bin(c, Bin::LShr, x, k63);
        let field = m.bin(c, Bin::LShr, x, k52);
        let k7ff = k(m, 0x7ff);
        let exp = m.bin(c, Bin::And, field, k7ff);
        let low52 = k(m, (1 << 52) - 1);
        let mant = m.bin(c, Bin::And, x, low52);
        let one52 = k(m, 1 << 52);
        let sig = m.bin(c, Bin::Or, mant, one52);
        // The f32 exponent the value would have, unbiased by 1023, biased by 127.
        let k896 = k(m, 896);
        let e = m.bin(c, Bin::Sub, exp, k896);
        // Below the normal range the significand shifts out further (a
        // denormal result); by 63 at most, which leaves nothing of it.
        let under = m.bin(c, Bin::Sub, k1, e);
        let pos = m.cmp(c, pred::SGT, under, k0);
        let under = m.select(c, pos, under, k0);
        let far = m.cmp(c, pred::SGT, under, k34);
        let under = m.select(c, far, k34, under);
        let shift = m.bin(c, Bin::Add, k29, under);
        let q = m.bin(c, Bin::LShr, sig, shift);
        let unit = m.bin(c, Bin::Shl, k1, shift);
        let below = m.bin(c, Bin::Sub, unit, k1);
        let rest = m.bin(c, Bin::And, sig, below);
        let less = m.bin(c, Bin::Sub, shift, k1);
        let half = m.bin(c, Bin::Shl, k1, less);
        // Up past half, or at half when the kept bits are odd.
        let odd = m.bin(c, Bin::And, q, k1);
        let rest = m.bin(c, Bin::Add, rest, odd);
        let up = m.cmp(c, pred::IUGT, rest, half);
        let up = m.cast(c, Cast::ZExt, up, i64t);
        // The exponent field less the significand's leading bit, which q
        // carries for a normal result; a carry out of rounding moves it up.
        let normal = m.cmp(c, pred::SGT, e, k1);
        let lower = m.bin(c, Bin::Sub, e, k1);
        let field = m.select(c, normal, lower, k0);
        let field = m.bin(c, Bin::Shl, field, k23);
        let bits = m.bin(c, Bin::Add, field, q);
        let bits = m.bin(c, Bin::Add, bits, up);
        let k255 = k(m, 255);
        let huge = m.cmp(c, pred::SGE, e, k255);
        let inf = k(m, 0x7f80_0000);
        let bits = m.select(c, huge, inf, bits);
        let all = m.cmp(c, pred::EQ, exp, k7ff);
        let payload = m.cmp(c, pred::NE, mant, k0);
        let nan = m.bin(c, Bin::And, all, payload);
        let top = m.bin(c, Bin::LShr, mant, k29);
        let quiet = k(m, 0x7fc0_0000);
        let nan_bits = m.bin(c, Bin::Or, top, quiet);
        let bits = m.select(c, nan, nan_bits, bits);
        let k31 = k(m, 31);
        let sign = m.bin(c, Bin::Shl, sign, k31);
        let bits = m.bin(c, Bin::Or, bits, sign);
        let i32t = self.s.t.i32;
        self.s.m.cast(c, Cast::Trunc, bits, i32t)
    }
}

/// Whether a pixel program runs part of itself in primitive order (POPS):
/// it sets the ordering unit's packer register, or reads the exiting wave id.
///
/// The console's ordered section: s18 is the collision wave id (bits 9:0 this
/// wave's id, 25:16 the id the wave must wait behind, 29:28 the packer mode,
/// bit 31 set when an earlier wave may overlap). The program sets the packer
/// register, spins until `src_pops_exiting_wave_id` passes the earlier wave's
/// id, then does its read-modify-write on UAV images. The host's equivalent
/// is a rasterizer-ordered view, so the program is lowered as the first wave
/// (s18 is 0: bit 31 clear skips the wait), the packer write is dropped, and
/// every UAV it opens is marked rasterizer ordered.
/// shortcut: all UAVs of the program, not only those the section touches.
pub(super) fn enters_pops(p: &Program) -> bool {
    p.blocks.iter().flat_map(|b| &b.insts).any(|i| {
        let Fields::Sop { src, imm, .. } = &i.fields else {
            return false;
        };
        (matches!(i.name(), "s_setreg_imm32_b32" | "s_setreg_b32")
            && imm.is_some_and(|m| m & 0x3F == HW_REG_POPS_PACKER))
            || src.iter().any(|o| matches!(o, Operand::Const(POPS_EXITING_WAVE_ID)))
    })
}

/// The f64 bits of a float inline constant (codes 240-248), as a 64-bit
/// float operand reads it.
fn f64_inline(code: u8) -> Option<u64> {
    Some(match code {
        240 => 0.5f64.to_bits(),
        241 => (-0.5f64).to_bits(),
        242 => 1.0f64.to_bits(),
        243 => (-1.0f64).to_bits(),
        244 => 2.0f64.to_bits(),
        245 => (-2.0f64).to_bits(),
        246 => 4.0f64.to_bits(),
        247 => (-4.0f64).to_bits(),
        // 1 / (2 pi)
        248 => 0x3FC4_5F30_6DC9_C882,
        _ => return None,
    })
}

/// What `Lower::f32_bits_of_f64` emits, on the CPU and in the same steps:
/// the f32 nearest an f64, ties to even, NaNs quiet with their sign and
/// the high bits of their payload.
#[cfg(test)]
pub(crate) fn f32_bits_of_f64(x: u64) -> u32 {
    let sign = x >> 63;
    let exp = (x >> 52 & 0x7ff) as i64;
    let mant = x & ((1 << 52) - 1);
    let sig = mant | 1 << 52;
    let e = exp - 896;
    let under = (1 - e).clamp(0, 34);
    let shift = 29 + under as u32;
    let q = sig >> shift;
    let rest = sig & ((1 << shift) - 1);
    let half = 1u64 << (shift - 1);
    let up = u64::from(rest + (q & 1) > half);
    let field = if e > 1 { (e - 1) as u64 } else { 0 };
    let mut bits = (field << 23) + q + up;
    if e >= 255 {
        bits = 0x7f80_0000;
    }
    if exp == 0x7ff && mant != 0 {
        bits = mant >> 29 | 0x7fc0_0000;
    }
    (bits | sign << 31) as u32
}

#[cfg(test)]
mod tests {
    /// The CPU form of `v_cvt_f32_f64`'s lowering agrees with Rust's own
    /// f64 to f32 conversion (ties to even) on edge values and on a few
    /// million others; NaNs stay NaNs of their sign.
    #[test]
    fn f64_to_f32_rounds_to_nearest_even() {
        let check = |x: u64| {
            let got = super::f32_bits_of_f64(x);
            let want = (f64::from_bits(x) as f32).to_bits();
            if f64::from_bits(x).is_nan() {
                assert!(f32::from_bits(got).is_nan(), "{x:#018x} -> {got:#010x}");
                assert_eq!(got >> 31, (x >> 63) as u32, "{x:#018x} sign");
                assert_eq!(got & 0x0040_0000, 0x0040_0000, "{x:#018x} quiet");
            } else {
                assert_eq!(got, want, "{x:#018x} ({})", f64::from_bits(x));
            }
        };
        let mut edges = vec![
            0,
            1,
            (1 << 52) - 1,
            1 << 52,
            0x7FF0_0000_0000_0000,
            0x7FF0_0000_0000_0001,
            0x7FF8_0000_0000_0000,
            0x7FFF_FFFF_FFFF_FFFF,
            f64::MAX.to_bits(),
            f64::from(f32::MAX).to_bits(),
            f64::from(f32::MIN_POSITIVE).to_bits(),
            f64::from(f32::from_bits(1)).to_bits(),
            f64::from(f32::from_bits(0x007F_FFFF)).to_bits(),
            1.0f64.to_bits(),
            0.1f64.to_bits(),
            (1.0f64 / 3.0).to_bits(),
        ];
        // Around every f32 exponent: exact values, halfway points (to even
        // either way), just above and below them, and the largest f32 with
        // what rounds past it to infinity.
        for e in 0..2047u64 {
            for m in [
                0u64,
                1,
                1 << 28,
                (1 << 28) - 1,
                (1 << 28) + 1,
                3 << 28,
                (1 << 29) - 1,
                (1 << 52) - 1,
                0xF_FFFF_E000_0000,
                0xF_FFFF_F000_0000,
            ] {
                edges.push(e << 52 | m);
            }
        }
        for x in edges {
            check(x);
            check(x | 1 << 63);
        }
        let mut s = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..4_000_000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            check(s);
            // The f32 range and below it, where results round or are denormal.
            let near = (s & 0x800F_FFFF_FFFF_FFFF) | ((s >> 20) % 330 + 700) << 52;
            check(near);
        }
    }
}
