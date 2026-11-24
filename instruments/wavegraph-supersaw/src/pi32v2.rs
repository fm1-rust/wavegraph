//! Block mixing kernel for the seven full-precision ramps.
use super::Saw;

#[inline(never)]
#[allow(unsafe_code)]
pub(super) fn mix(saws: &mut [Saw; 7], side_gain: u16, output: &mut [[i32; 2]]) {
    let [center, a, b, c, d, e, f] = saws;
    let out = output.as_flattened_mut();
    let len = out.len();
    if len == 0 {
        return;
    }
    let dst = out.as_mut_ptr();
    let gains = [side_gain, 25];
    // SAFETY: Each pass visits exactly len aligned i32s. The first writes
    // four side ramps into dst. The second reads each partial sum before
    // overwriting it, and loads exactly two aligned u16 gains. rep can
    // leave a residual count, so each loop continues until r0 reaches zero.
    // Every pointer, phase, and scratch register modified by asm is an output.
    unsafe {
        // The two 32-bit lanes hold independent sums of signed 24-bit ramps.
        core::arch::asm!(
            "2:", "rep r0 {{",
            "r12 += r13", "r4 += r5", "r6 += r9", "r10 += r11",
            "r2 = r12 >>> 8", "r3 = r4 >>> 8", "r14 = r6 >>> 8", "r15 = r10 >>> 8",
            "r3_r2 = r3,r2 +|+ r15,r14 (ssat)",
            "r2 += r3", "[r1++=4] = r2",
            "}}", "if (r0 != 0) goto 2b",
            inout("r0") len => _, inout("r1") dst => _,
            inout("r12") a.phase, in("r13") a.step, inout("r4") b.phase, in("r5") b.step,
            inout("r6") c.phase, in("r9") c.step, inout("r10") d.phase, in("r11") d.step,
            out("r2") _, out("r3") _, out("r14") _, out("r15") _, options(nostack),
        );
        // Add the last two sides, weight the side sum once and add the center.
        core::arch::asm!(
            "r14 = h[r6++=2] (u)", "r15 = h[r6++=2] (u)",
            "2:", "rep r0 {{",
            "r8 += r9", "r10 += r11", "r12 += r13",
            "r3 = r8 >>> 8", "r4 = r10 >>> 8", "r5 = r12 >>> 8",
            "r6 = [r1++=4]", "r3 += r4", "r3 += r6", "r3 *= r14",
            "r5 *= r15", "r3 += r5", "[r2++=4] = r3",
            "}}", "if (r0 != 0) goto 2b",
            inout("r0") len => _, inout("r1") dst => _, inout("r2") dst => _,
            inout("r8") e.phase, in("r9") e.step, inout("r10") f.phase, in("r11") f.step,
            inout("r12") center.phase, in("r13") center.step,
            out("r3") _, out("r4") _, out("r5") _, inout("r6") gains.as_ptr() => _,
            out("r14") _, out("r15") _, options(nostack),
        );
    }
}
