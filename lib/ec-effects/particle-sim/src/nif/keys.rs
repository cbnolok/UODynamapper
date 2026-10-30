//! Key interpolation and controller time, as the Gamebryo SDK computes them.
//!
//! Sources (Gamebryo 2.6 SDK, `CoreLibs/NiAnimation` and `CoreLibs/NiMain`):
//! - `NiFloatKey::GenInterp` and its linear, Bezier (Hermite), TCB and step interpolations; the same per
//!   component for `NiPosKey` and `NiColorKey`. Times outside the keys clamp to the first / last value.
//! - TCB tangents: `NiTCBFloatKey::CalculateDVals` (incoming `DS`, outgoing `DD`, the end keys mirrored).
//! - Rotation keys: `NiLinRotKey` (slerp), `NiTCBRotKey` (squad with `CalculateDVals` on quaternion logs),
//!   `NiEulerRotKey` (three float curves, `NiQuaternion::FromAngleAxesXYZ`).
//! - `NiQuaternion::Slerp` of the Win32 build: Jonathan Blow's counter-warped lerp with `FastNormalize`, the
//!   implementation of the client's platform.
//! - Controller time: `NiTimeController::ComputeScaledTime` as UOSA.exe has it (0x006a4900) and
//!   `DontDoUpdate` (0x007a0d30).

use super::desc::{
    BoolInterp, BoolKey, ControllerTiming, Curve, CycleType, FloatInterp, Key, KeyKind, Point3Interp, RotKey,
    RotationCurve, Tcb,
};
use crate::math::{Mat3, Vec3};

/// `NiQuaternion::ms_fEpsilon`: below this |sin|, log and exp use the small-angle forms.
const QUAT_EPSILON: f32 = 1e-3;
/// Blow slerp `CounterWarp` attenuation (`NiQuaternion_Win32.cpp`).
const SLERP_ATTENUATION: f32 = 0.822_796_87;
/// Blow slerp `CounterWarp` worst-case slope.
const SLERP_WORST_CASE_SLOPE: f32 = 0.585_492_19;
/// `ISqrt_approx_in_neighborhood` expansion point.
const ISQRT_NEIGHBORHOOD: f32 = 0.959_066;
/// `ISqrt_approx_in_neighborhood` scale.
const ISQRT_SCALE: f32 = 1.000_311;
/// `FastNormalize`: below this squared length, one more Newton step.
const FAST_NORMALIZE_SECOND_STEP: f32 = 0.915_211_98;
/// `FastNormalize`: below this squared length, a third step.
const FAST_NORMALIZE_THIRD_STEP: f32 = 0.652_119_70;
/// The slerp parameter that splits the counter-warp halves.
const SLERP_HALF: f32 = 0.5;
/// The TCB tangent weights are halved (`0.5 * (1 - T)`).
const TCB_HALF: f32 = 0.5;

/// A quaternion (w, x, y, z), Gamebryo's `NiQuaternion`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quat {
    pub w: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Quat {
    pub const IDENTITY: Self = Self { w: 1.0, x: 0.0, y: 0.0, z: 0.0 };

    pub const fn from_array([w, x, y, z]: [f32; 4]) -> Self {
        Self { w, x, y, z }
    }

    pub fn dot(self, other: Self) -> f32 {
        self.w * other.w + self.x * other.x + self.y * other.y + self.z * other.z
    }

    fn scale(self, k: f32) -> Self {
        Self { w: self.w * k, x: self.x * k, y: self.y * k, z: self.z * k }
    }

    fn add(self, other: Self) -> Self {
        Self { w: self.w + other.w, x: self.x + other.x, y: self.y + other.y, z: self.z + other.z }
    }

    fn neg(self) -> Self {
        self.scale(-1.0)
    }

    /// The Hamilton product `self * other`.
    pub fn mul(self, q: Self) -> Self {
        let p = self;
        Self {
            w: p.w * q.w - p.x * q.x - p.y * q.y - p.z * q.z,
            x: p.w * q.x + p.x * q.w + p.y * q.z - p.z * q.y,
            y: p.w * q.y - p.x * q.z + p.y * q.w + p.z * q.x,
            z: p.w * q.z + p.x * q.y - p.y * q.x + p.z * q.w,
        }
    }

    /// `NiQuaternion::FromAngleAxesXYZ`: x, y, z angles in radians.
    pub fn from_euler_xyz(x: f32, y: f32, z: f32) -> Self {
        let (s0, c0) = (x * 0.5).sin_cos();
        let (s1, c1) = (y * 0.5).sin_cos();
        let (s2, c2) = (z * 0.5).sin_cos();
        Self {
            w: c2 * c1 * c0 + s2 * s1 * s0,
            x: c2 * c1 * s0 - s2 * s1 * c0,
            y: c2 * s1 * c0 + s2 * c1 * s0,
            z: -c2 * s1 * s0 + s2 * c1 * c0,
        }
    }

    /// `NiQuaternion::ToRotation`.
    pub fn to_mat3(self) -> Mat3 {
        let Self { w, x, y, z } = self;
        let (tx, ty, tz) = (2.0 * x, 2.0 * y, 2.0 * z);
        let (twx, twy, twz) = (tx * w, ty * w, tz * w);
        let (txx, txy, txz) = (tx * x, ty * x, tz * x);
        let (tyy, tyz, tzz) = (ty * y, tz * y, tz * z);
        Mat3::from_cols(
            Vec3::new(1.0 - (tyy + tzz), txy + twz, txz - twy),
            Vec3::new(txy - twz, 1.0 - (txx + tzz), tyz + twx),
            Vec3::new(txz + twy, tyz - twx, 1.0 - (txx + tyy)),
        )
    }

    /// `NiQuaternion::FastNormalize` (Win32).
    pub fn fast_normalize(self) -> Self {
        let isqrt = |s: f32| {
            let additive = ISQRT_SCALE / ISQRT_NEIGHBORHOOD.sqrt();
            let factor = ISQRT_SCALE * (-0.5 / (ISQRT_NEIGHBORHOOD * ISQRT_NEIGHBORHOOD.sqrt()));
            additive + (s - ISQRT_NEIGHBORHOOD) * factor
        };
        let s = self.dot(self);
        let mut k = isqrt(s);
        if s <= FAST_NORMALIZE_SECOND_STEP {
            k *= isqrt(k * k * s);
            if s <= FAST_NORMALIZE_THIRD_STEP {
                k *= isqrt(k * k * s);
            }
        }
        self.scale(k)
    }

    /// `NiQuaternion::Slerp` (Win32: counter-warped lerp, then `FastNormalize`).
    pub fn slerp(t: f32, p: Self, q: Self) -> Self {
        let cos = p.dot(q);
        let counter_warp = |t: f32| {
            let factor = (1.0 - SLERP_ATTENUATION * cos).powi(2);
            let k = SLERP_WORST_CASE_SLOPE * factor;
            t * (k * t * (2.0 * t - 3.0) + 1.0 + k)
        };
        let t_prime = if t <= SLERP_HALF { counter_warp(t) } else { 1.0 - counter_warp(1.0 - t) };
        let lerp = |a: f32, b: f32| a + t_prime * (b - a);
        Self { w: lerp(p.w, q.w), x: lerp(p.x, q.x), y: lerp(p.y, q.y), z: lerp(p.z, q.z) }.fast_normalize()
    }

    /// `NiQuaternion::Squad`.
    pub fn squad(t: f32, p: Self, a: Self, b: Self, q: Self) -> Self {
        Self::slerp(2.0 * t * (1.0 - t), Self::slerp(t, p, q), Self::slerp(t, a, b))
    }

    /// `NiQuaternion::Log` of a unit quaternion.
    pub fn log(self) -> Self {
        let angle = self.w.clamp(-1.0, 1.0).acos();
        let sin = angle.sin();
        let coeff = if sin.abs() < QUAT_EPSILON { 1.0 } else { angle / sin };
        Self { w: 0.0, x: coeff * self.x, y: coeff * self.y, z: coeff * self.z }
    }

    /// `NiQuaternion::Exp` of a pure quaternion.
    pub fn exp(self) -> Self {
        let angle = (self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        let (sin, cos) = angle.sin_cos();
        let coeff = if sin.abs() < QUAT_EPSILON { 1.0 } else { sin / angle };
        Self { w: cos, x: coeff * self.x, y: coeff * self.y, z: coeff * self.z }
    }
}

/// A value that keys interpolate: float, point, colour.
pub trait KeyValue: Copy {
    fn add(self, other: Self) -> Self;
    fn sub(self, other: Self) -> Self;
    fn scale(self, k: f32) -> Self;
}

impl KeyValue for f32 {
    fn add(self, other: Self) -> Self {
        self + other
    }
    fn sub(self, other: Self) -> Self {
        self - other
    }
    fn scale(self, k: f32) -> Self {
        self * k
    }
}

impl<const N: usize> KeyValue for [f32; N] {
    fn add(self, other: Self) -> Self {
        std::array::from_fn(|k| self[k] + other[k])
    }
    fn sub(self, other: Self) -> Self {
        std::array::from_fn(|k| self[k] - other[k])
    }
    fn scale(self, k: f32) -> Self {
        self.map(|value| value * k)
    }
}

/// The key segment of `time`: (index of the first key, normalized time in [0, 1]), or the single key to hold.
enum Segment {
    Hold(usize),
    Between(usize, f32),
}

/// `GenInterp`'s key search, clamped to the key range.
fn segment(times: impl Fn(usize) -> f32, count: usize, time: f32) -> Option<Segment> {
    if count == 0 {
        return None;
    }
    let last = count - 1;
    if count == 1 || time <= times(0) {
        return Some(Segment::Hold(0));
    }
    if time >= times(last) {
        return Some(Segment::Hold(last));
    }
    let index = (0..last).rev().find(|&i| times(i) <= time).unwrap_or(0);
    let (t0, t1) = (times(index), times(index + 1));
    let span = t1 - t0;
    let normalized = if span > 0.0 { (time - t0) / span } else { 0.0 };
    Some(Segment::Between(index, normalized))
}

/// Hermite cubic of `NiInterpScalar`: `p + (dp + (a + b t) t) t`, `a = 3(q - p) - (dq + 2 dp)`,
/// `b = dp + dq - 2(q - p)`.
fn hermite<T: KeyValue>(t: f32, p: T, q: T, dp: T, dq: T) -> T {
    let diff = q.sub(p);
    let a = diff.scale(3.0).sub(dq.add(dp.scale(2.0)));
    let b = dp.add(dq).sub(diff.scale(2.0));
    p.add(dp.add(a.add(b.scale(t)).scale(t)).scale(t))
}

/// `CalculateDVals` of key `i`: (incoming DS, outgoing DD).
fn tcb_tangents<T: KeyValue>(keys: &[Key<T>], i: usize) -> (T, T) {
    let key = &keys[i];
    let last = keys.len() - 1;
    // End keys mirror their neighbour; the time lengths are then 1.
    let (previous, next, previous_length, next_length) = if keys.len() < 2 {
        (key.value, key.value, 1.0, 1.0)
    } else if i == 0 {
        (key.value.scale(2.0).sub(keys[1].value), keys[1].value, 1.0, 1.0)
    } else if i == last {
        (keys[i - 1].value, key.value.scale(2.0).sub(keys[i - 1].value), 1.0, 1.0)
    } else {
        (keys[i - 1].value, keys[i + 1].value, key.time - keys[i - 1].time, keys[i + 1].time - key.time)
    };
    let Tcb { tension, continuity, bias } = key.tcb;
    let diff1 = key.value.sub(previous);
    let diff2 = next.sub(key.value);
    let (one_plus_c, one_minus_c) = (1.0 + continuity, 1.0 - continuity);
    let (one_plus_b, one_minus_b) = (1.0 + bias, 1.0 - bias);
    let half_one_minus_t = TCB_HALF * (1.0 - tension);
    let incoming = diff1
        .scale(half_one_minus_t * one_minus_c * one_plus_b)
        .add(diff2.scale(half_one_minus_t * one_plus_c * one_minus_b));
    let outgoing = diff1
        .scale(half_one_minus_t * one_plus_c * one_plus_b)
        .add(diff2.scale(half_one_minus_t * one_minus_c * one_minus_b));
    let inverse = 2.0 / (previous_length + next_length);
    (incoming.scale(previous_length * inverse), outgoing.scale(next_length * inverse))
}

/// First and second derivative of the Hermite cubic of [`hermite`] at `t`: `dp + (2a + 3b t) t` and
/// `2a + 6b t` (`NiBezPosKey` / `NiTCBPosKey::InterpolateD1` and `InterpolateD2`).
fn hermite_derivatives<T: KeyValue>(t: f32, p: T, q: T, dp: T, dq: T) -> (T, T) {
    let diff = q.sub(p);
    let a = diff.scale(3.0).sub(dq.add(dp.scale(2.0)));
    let b = dp.add(dq).sub(diff.scale(2.0));
    let d1 = dp.add(a.scale(2.0).add(b.scale(3.0 * t)).scale(t));
    let d2 = a.scale(2.0).add(b.scale(6.0 * t));
    (d1, d2)
}

impl<T: KeyValue> Curve<T> {
    /// `GenInterp` at `time`; `None` without keys.
    pub fn eval(&self, time: f32) -> Option<T> {
        let keys = &self.keys;
        match segment(|i| keys[i].time, keys.len(), time)? {
            Segment::Hold(index) => Some(keys[index].value),
            Segment::Between(index, t) => self.segment_value(index, t),
        }
    }

    /// The interpolation function of the key type between keys `index` and `index + 1` at the normalized
    /// time `t`; `None` when `index + 1` is not a key.
    pub(crate) fn segment_value(&self, index: usize, t: f32) -> Option<T> {
        let keys = &self.keys;
        let (k0, k1) = (keys.get(index)?, keys.get(index + 1)?);
        Some(match self.kind {
            KeyKind::Linear => k0.value.add(k1.value.sub(k0.value).scale(t)),
            KeyKind::Step => {
                if t < 1.0 {
                    k0.value
                } else {
                    k1.value
                }
            }
            KeyKind::Bezier => hermite(t, k0.value, k1.value, k0.out_tangent, k1.in_tangent),
            KeyKind::Tcb => {
                let (_, outgoing) = tcb_tangents(keys, index);
                let (incoming, _) = tcb_tangents(keys, index + 1);
                hermite(t, k0.value, k1.value, outgoing, incoming)
            }
        })
    }

    /// The first and second derivative of [`Self::segment_value`] by the normalized time, as the key type's
    /// `InterpolateD1` / `InterpolateD2` give them. Linear: `p1 - p0` and zero (`NiLinPosKey`); step: `p0 - p1`
    /// and zero, the sign as `NiStepPosKey::InterpolateD1` has it.
    pub(crate) fn segment_derivatives(&self, index: usize, t: f32) -> Option<(T, T)> {
        let keys = &self.keys;
        let (k0, k1) = (keys.get(index)?, keys.get(index + 1)?);
        let zero = k0.value.scale(0.0);
        Some(match self.kind {
            KeyKind::Linear => (k1.value.sub(k0.value), zero),
            KeyKind::Step => (k0.value.sub(k1.value), zero),
            KeyKind::Bezier => hermite_derivatives(t, k0.value, k1.value, k0.out_tangent, k1.in_tangent),
            KeyKind::Tcb => {
                let (_, outgoing) = tcb_tangents(keys, index);
                let (incoming, _) = tcb_tangents(keys, index + 1);
                hermite_derivatives(t, k0.value, k1.value, outgoing, incoming)
            }
        })
    }

    /// First and last key times.
    pub fn time_range(&self) -> Option<(f32, f32)> {
        Some((self.keys.first()?.time, self.keys.last()?.time))
    }
}

/// `NiBoolKey::GenInterp` (bool keys always step): the value of the last key at or before `time`, the first
/// key's before it.
pub fn eval_bool_keys(keys: &[BoolKey], time: f32) -> Option<bool> {
    let first = keys.first()?;
    Some(keys.iter().rev().find(|key| key.time <= time).unwrap_or(first).value)
}

/// `NiRotKey::FillDerivedVals`: consecutive quaternions on the same hemisphere.
fn aligned_quats(keys: &[RotKey]) -> Vec<Quat> {
    let mut quats: Vec<Quat> = Vec::with_capacity(keys.len());
    for key in keys {
        let mut q = Quat::from_array(key.value);
        if let Some(previous) = quats.last() {
            if previous.dot(q) < 0.0 {
                q = q.neg();
            }
        }
        quats.push(q);
    }
    quats
}

/// `NiTCBRotKey::CalculateDVals`: the inner quadrangle points (A, B) of key `i`.
fn tcb_rot_points(keys: &[RotKey], quats: &[Quat], i: usize) -> (Quat, Quat) {
    let last = quats.len() - 1;
    let q = quats[i];
    let (previous, next, t_previous, t_next) = if quats.len() < 2 {
        return (q, q);
    } else if i == 0 {
        (quats[0], quats[1], keys[0].time, keys[1].time)
    } else if i == last {
        (quats[last - 1], quats[last], keys[last - 1].time, keys[last].time)
    } else {
        (quats[i - 1], quats[i + 1], keys[i - 1].time, keys[i + 1].time)
    };
    let inverse = Quat { w: q.w, x: -q.x, y: -q.y, z: -q.z };
    let log_previous = inverse.mul(previous).log();
    let log_next = inverse.mul(next).log();
    let Tcb { tension, continuity, bias } = keys[i].tcb;
    let (mt, mc, pc, mb, pb) = (1.0 - tension, 1.0 - continuity, 1.0 + continuity, 1.0 - bias, 1.0 + bias);
    let span = t_next - t_previous;
    let time = keys[i].time;
    let adjust_out = if span != 0.0 { (time - t_previous) / span } else { TCB_HALF };
    let adjust_in = if span != 0.0 { (t_next - time) / span } else { TCB_HALF };
    let tangent_out =
        log_next.scale(adjust_out * mt * mc * mb).add(log_previous.scale(adjust_out * mt * pc * pb));
    let a = q.mul(log_next.sub_pure(tangent_out).scale(-TCB_HALF).exp());
    let tangent_in = log_next.scale(adjust_in * mt * pc * mb).add(log_previous.scale(adjust_in * mt * mc * pb));
    let b = q.mul(log_previous.sub_pure(tangent_in).scale(TCB_HALF).exp());
    (a, b)
}

impl Quat {
    fn sub_pure(self, other: Self) -> Self {
        self.add(other.neg())
    }
}

impl RotationCurve {
    /// The rotation at `time`; `None` without keys.
    pub fn eval(&self, time: f32) -> Option<Quat> {
        match self {
            Self::Euler { axes } => {
                if axes.iter().all(|axis| axis.as_ref().is_none_or(|curve| curve.keys.is_empty())) {
                    return None;
                }
                let angle = |axis: &Option<Curve<f32>>| axis.as_ref().and_then(|curve| curve.eval(time)).unwrap_or(0.0);
                Some(Quat::from_euler_xyz(angle(&axes[0]), angle(&axes[1]), angle(&axes[2])))
            }
            Self::Quaternion { kind, keys } => {
                let (index, t) = match segment(|i| keys[i].time, keys.len(), time)? {
                    Segment::Hold(index) => return Some(Quat::from_array(keys[index].value)),
                    Segment::Between(index, t) => (index, t),
                };
                let quats = aligned_quats(keys);
                let (p, q) = (quats[index], quats[index + 1]);
                Some(match kind {
                    KeyKind::Step => {
                        if t < 1.0 {
                            p
                        } else {
                            q
                        }
                    }
                    KeyKind::Tcb => {
                        let (a, _) = tcb_rot_points(keys, &quats, index);
                        let (_, b) = tcb_rot_points(keys, &quats, index + 1);
                        Quat::squad(t, p, a, b, q)
                    }
                    // Bezier rotation keys do not occur in the client's files; they interpolate linearly here.
                    KeyKind::Linear | KeyKind::Bezier => Quat::slerp(t, p, q),
                })
            }
        }
    }
}

impl FloatInterp {
    /// `NiFloatInterpolator::Update`: keys, else the pose; `None` leaves the target unchanged.
    pub fn eval(&self, time: f32) -> Option<f32> {
        self.keys.as_ref().and_then(|curve| curve.eval(time)).or(self.pose)
    }
}

impl Point3Interp {
    pub fn eval(&self, time: f32) -> Option<[f32; 3]> {
        self.keys.as_ref().and_then(|curve| curve.eval(time)).or(self.pose)
    }
}

impl BoolInterp {
    pub fn eval(&self, time: f32) -> Option<bool> {
        eval_bool_keys(&self.keys, time).or(self.pose)
    }

    /// Posed: no keys and a valid pose (`GetChannelPosed(0)`).
    pub fn is_posed(&self) -> bool {
        self.keys.is_empty() && self.pose.is_some()
    }
}

impl FloatInterp {
    pub fn is_posed(&self) -> bool {
        self.keys.as_ref().is_none_or(|curve| curve.keys.is_empty()) && self.pose.is_some()
    }
}

/// The time state of one controller (`NiTimeController` +0x1c start, +0x20 last time, +0x24 accumulated
/// scaled time, `DontDoUpdate`'s last scaled time +0x28).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ControllerClock {
    last_time: Option<f32>,
    accumulated: f32,
    last_scaled: Option<f32>,
}

impl ControllerClock {
    /// `ComputeScaledTime` (0x006a4900). `range` is (lo, hi), `None` for the unbounded range
    /// `NiPSysUpdateCtlr` uses.
    pub fn scaled_time(&mut self, timing: &ControllerTiming, cycle: CycleType, range: Option<(f32, f32)>, time: f32) -> f32 {
        let delta = match self.last_time {
            None => {
                self.accumulated = 0.0;
                if timing.app_init { 0.0 } else { time }
            }
            Some(last) => time - last,
        };
        self.accumulated += timing.frequency * delta;
        self.last_time = Some(time);
        let mut scaled = timing.phase + self.accumulated;
        let Some((lo, hi)) = range else {
            return scaled;
        };
        let span = hi - lo;
        match cycle {
            CycleType::Loop => {
                scaled = if span != 0.0 {
                    let wrapped = (scaled - lo) % span + lo;
                    if wrapped < lo { wrapped + span } else { wrapped }
                } else {
                    lo
                };
            }
            CycleType::Reverse => {
                scaled = if span != 0.0 {
                    let mut x = (scaled - lo) % (span + span);
                    if x < 0.0 {
                        x += span + span;
                    }
                    if x <= span { lo + x } else { (span + span) - x + lo }
                } else {
                    lo
                };
            }
            CycleType::Clamp => {}
        }
        if hi < scaled {
            scaled = hi;
        } else if scaled < lo {
            scaled = lo;
        }
        if timing.reverse_direction {
            scaled = hi - (scaled - lo);
        }
        scaled
    }

    /// `DontDoUpdate` (0x007a0d30) with `ComputeScaledTime`: the scaled time when the controller updates at
    /// `time` (active, the time moved and the scaled time changed), `None` otherwise.
    pub fn tick(&mut self, timing: &ControllerTiming, cycle: CycleType, range: Option<(f32, f32)>, time: f32) -> Option<f32> {
        if !timing.active || self.last_time == Some(time) {
            return None;
        }
        let scaled = self.scaled_time(timing, cycle, range, time);
        if self.last_scaled == Some(scaled) {
            return None;
        }
        self.last_scaled = Some(scaled);
        Some(scaled)
    }

    /// The controller's last update time (`+0x20`).
    pub fn last_time(&self) -> Option<f32> {
        self.last_time
    }

    /// `NiTimeController::Start` (0x006a48a0): the next update starts the accumulation again.
    pub fn restart(&mut self) {
        *self = Self::default();
    }
}

impl ControllerTiming {
    /// The (lo, hi) key range of the controller.
    pub fn range(&self) -> Option<(f32, f32)> {
        Some((self.start_time, self.stop_time))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1e-5;

    fn key<T: Copy>(time: f32, value: T, in_tangent: T, out_tangent: T) -> Key<T> {
        Key { time, value, in_tangent, out_tangent, tcb: Tcb::default() }
    }

    fn curve(kind: KeyKind, keys: Vec<Key<f32>>) -> Curve<f32> {
        Curve { kind, keys }
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < EPSILON, "{actual} != {expected}");
    }

    #[test]
    fn linear_keys_interpolate_and_clamp() {
        let curve = curve(KeyKind::Linear, vec![key(1.0, 2.0, 0.0, 0.0), key(3.0, 6.0, 0.0, 0.0)]);
        assert_close(curve.eval(2.0).unwrap(), 4.0);
        assert_close(curve.eval(0.0).unwrap(), 2.0);
        assert_close(curve.eval(9.0).unwrap(), 6.0);
    }

    #[test]
    fn bezier_keys_follow_the_hermite_formula() {
        // p = 1, q = 3, dp = 0.5, dq = -1: a = 3*2 - (-1 + 1) = 6, b = 0.5 - 1 - 4 = -4.5;
        // at t = 0.25: 1 + (0.5 + (6 - 4.5*0.25)*0.25)*0.25 = 1.429_687_5.
        let curve = curve(KeyKind::Bezier, vec![key(0.0, 1.0, 0.0, 0.5), key(2.0, 3.0, -1.0, 0.0)]);
        assert_close(curve.eval(0.5).unwrap(), 1.429_687_5);
    }

    #[test]
    fn tcb_keys_use_catmull_rom_tangents_at_zero_parameters() {
        // Keys 0, 1, 4 at times 0, 1, 2 with T = C = B = 0. Key 1: diff1 = 1, diff2 = 3, DS = DD = 0.5*1 + 0.5*3
        // = 2 (equal lengths). Key 0 mirrors: previous = -1, diff1 = diff2 = 1, DD = 1.
        // Segment 0 at t = 0.5: hermite(0.5, 0, 1, 1, 2) = 0 + (1 + (a + b/2)/2)/2 with a = 3 - 4 = -1,
        // b = 3 - 2 = 1 -> (1 + (-0.5)/2)/2 = 0.375.
        let curve = curve(
            KeyKind::Tcb,
            vec![key(0.0, 0.0, 0.0, 0.0), key(1.0, 1.0, 0.0, 0.0), key(2.0, 4.0, 0.0, 0.0)],
        );
        assert_close(curve.eval(0.5).unwrap(), 0.375);
        assert_close(curve.eval(1.0).unwrap(), 1.0);
    }

    #[test]
    fn tcb_tension_one_flattens_the_tangents() {
        let mut keys = vec![key(0.0, 0.0, 0.0, 0.0), key(1.0, 1.0, 0.0, 0.0), key(2.0, 4.0, 0.0, 0.0)];
        for key in &mut keys {
            key.tcb.tension = 1.0;
        }
        let curve = curve(KeyKind::Tcb, keys);
        // Zero tangents: smoothstep, 0.5 at the middle of segment 0.
        assert_close(curve.eval(0.5).unwrap(), 0.5);
    }

    #[test]
    fn step_keys_hold_until_the_next_key() {
        let curve = curve(KeyKind::Step, vec![key(0.0, 1.0, 0.0, 0.0), key(1.0, 5.0, 0.0, 0.0)]);
        assert_close(curve.eval(0.999).unwrap(), 1.0);
        assert_close(curve.eval(1.0).unwrap(), 5.0);
    }

    #[test]
    fn bool_keys_step() {
        let keys = [BoolKey { time: 0.0, value: false }, BoolKey { time: 1.0, value: true }];
        assert_eq!(eval_bool_keys(&keys, 0.5), Some(false));
        assert_eq!(eval_bool_keys(&keys, 1.0), Some(true));
        assert_eq!(eval_bool_keys(&keys, -1.0), Some(false));
    }

    #[test]
    fn euler_keys_compose_xyz() {
        let half_pi = std::f32::consts::FRAC_PI_2;
        let z = curve(KeyKind::Linear, vec![key(0.0, 0.0, 0.0, 0.0), key(1.0, 2.0 * half_pi, 0.0, 0.0)]);
        let rotation = RotationCurve::Euler { axes: [None, None, Some(z)] };
        let matrix = rotation.eval(0.5).unwrap().to_mat3();
        // A quarter turn about z maps x to y.
        let mapped = matrix.mul_vec3(Vec3::X);
        assert!((mapped - Vec3::Y).length() < EPSILON, "{mapped:?}");
        // FromAngleAxesXYZ applies x first: x then z quarter turns map y to z then... check against a product.
        let combined = Quat::from_euler_xyz(half_pi, 0.0, half_pi).to_mat3();
        let expected = Quat::from_euler_xyz(0.0, 0.0, half_pi).to_mat3().mul_mat3(&Quat::from_euler_xyz(half_pi, 0.0, 0.0).to_mat3());
        for row in 0..3 {
            for column in 0..3 {
                assert!((combined.rows[row][column] - expected.rows[row][column]).abs() < EPSILON);
            }
        }
    }

    #[test]
    fn slerp_is_close_to_the_exact_rotation() {
        let p = Quat::IDENTITY;
        let angle = 1.2_f32;
        let q = Quat { w: (angle * 0.5).cos(), x: 0.0, y: 0.0, z: (angle * 0.5).sin() };
        /// Blow's approximation error bound on this arc.
        const SLERP_TOLERANCE: f32 = 2e-3;
        for step in 0..=4 {
            let t = step as f32 / 4.0;
            let result = Quat::slerp(t, p, q);
            let exact = (angle * t * 0.5).cos();
            assert!((result.w - exact).abs() < SLERP_TOLERANCE, "t = {t}: {} vs {exact}", result.w);
        }
        // FastNormalize's single isqrt step leaves unit quaternions a few 1e-4 short.
        assert!((Quat::slerp(0.0, p, q).w - 1.0).abs() < SLERP_TOLERANCE);
    }

    fn timing(cycle: CycleType) -> ControllerTiming {
        ControllerTiming {
            cycle,
            active: true,
            app_init: false,
            reverse_direction: false,
            frequency: 1.0,
            phase: 0.0,
            start_time: 1.0,
            stop_time: 3.0,
        }
    }

    fn scaled_at(cycle: CycleType, time: f32) -> f32 {
        let timing = timing(cycle);
        ControllerClock::default().scaled_time(&timing, cycle, timing.range(), time)
    }

    #[test]
    fn controller_cycles_loop_reverse_and_clamp() {
        // Span 2 from 1: time 4.5 -> loop 1 + (3.5 % 2) = 2.5; reverse: x = 3.5 % 4 = 3.5 > 2 -> 4 - 3.5 + 1 = 1.5.
        assert_close(scaled_at(CycleType::Loop, 4.5), 2.5);
        assert_close(scaled_at(CycleType::Reverse, 4.5), 1.5);
        assert_close(scaled_at(CycleType::Clamp, 4.5), 3.0);
        assert_close(scaled_at(CycleType::Clamp, 0.2), 1.0);
        assert_close(scaled_at(CycleType::Loop, 0.5), 2.5);
    }

    #[test]
    fn controller_time_accumulates_frequency_and_direction() {
        let mut timing = timing(CycleType::Clamp);
        timing.frequency = 2.0;
        timing.reverse_direction = true;
        let mut clock = ControllerClock::default();
        // First update: delta = time (APP_TIME) -> 2 * 0.75 = 1.5 -> reversed 3 - 0.5 = 2.5.
        assert_close(clock.scaled_time(&timing, timing.cycle, timing.range(), 0.75), 2.5);
        // Then deltas accumulate: + 2 * 0.25 = 2.0 -> 3 - 1 = 2.0.
        assert_close(clock.scaled_time(&timing, timing.cycle, timing.range(), 1.0), 2.0);
    }

    #[test]
    fn controllers_skip_repeated_times() {
        let timing = timing(CycleType::Clamp);
        let mut clock = ControllerClock::default();
        assert!(clock.tick(&timing, timing.cycle, timing.range(), 1.5).is_some());
        assert!(clock.tick(&timing, timing.cycle, timing.range(), 1.5).is_none());
        // Both clamp to hi: the scaled time did not change.
        assert!(clock.tick(&timing, timing.cycle, timing.range(), 5.0).is_some());
        assert!(clock.tick(&timing, timing.cycle, timing.range(), 6.0).is_none());
    }
}
