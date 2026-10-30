//! `NiPathInterpolator`, as the Gamebryo 2.6 SDK computes it.
//!
//! Sources (`CoreLibs/NiAnimation`):
//! - `NiPathInterpolator::Update`: the percentage curve gives a value; `GetLoc` turns it into a path segment
//!   and a normalized time; the follow flag adds the rotation of `CalcRefFrame`.
//! - `GetLoc`: without constant velocity the value is a time against the path key times (clamped to them);
//!   with it, `GetPercentLoc` finds the point at that fraction of the arc length (Newton's method on the
//!   Gauss-Legendre arc length of `PathLength`, the speed of `PathSpeed`).
//! - `GetTotalPathLength`: the arc length from the first key to each key, computed once.
//! - `CalcRefFrame`: the frame of `NiPosKey::CoordinateFrame` (`NiPosKey.cpp`), the allow-flip smoothing, the
//!   flip and follow-axis swaps, and the bank turn about the tangent.
//! - The maximum curvature of `NiBezPosKey` / `NiTCBPosKey::ComputeMaxCurvature` (sampled per segment; zero
//!   for linear and step keys).

use super::desc::{Curve, KeyKind, PathInterp};
use super::scene::make_rotation;
use crate::math::{Mat3, Vec3};

/// `NiPathInterpolator` flag: the path is open (no wrap from the last key to the first in the smoothing).
const FLAG_OPEN_CURVE: u16 = 0x2;
/// `NiPathInterpolator` flag: the follow frame uses the curve's normal, so it may flip over.
const FLAG_ALLOW_FLIP: u16 = 0x4;
/// `NiPathInterpolator` flag: bank about the tangent by the curvature.
const FLAG_BANK: u16 = 0x8;
/// `NiPathInterpolator` flag: the percentage is a fraction of the arc length.
const FLAG_CONST_VELOCITY: u16 = 0x10;
/// `NiPathInterpolator` flag: the node turns along the path.
const FLAG_FOLLOW: u16 = 0x20;
/// `NiPathInterpolator` flag: negate the frame's direction and up axes.
const FLAG_FLIP: u16 = 0x40;
/// `m_sFollowAxis` that points the node's Y axis along the path (`CalcRefFrame` case 1).
const FOLLOW_AXIS_Y: i16 = 1;
/// `m_sFollowAxis` that points the node's Z axis along the path (`CalcRefFrame` case 2).
const FOLLOW_AXIS_Z: i16 = 2;

/// `PathLength`: Legendre roots mapped to (root + 1) / 2.
const GAUSS_ROOTS: [f32; 5] = [0.046_910_077, 0.230_765_345, 0.5, 0.769_234_655, 0.953_089_922];
/// `PathLength`: Legendre coefficients divided by 2.
const GAUSS_COEFFICIENTS: [f32; 5] = [0.118_463_442, 0.239_314_335, 0.284_444_444, 0.239_314_335, 0.118_463_442];
/// `GetPercentLoc`: Newton iterations at most.
const NEWTON_MAX_ITERATIONS: usize = 32;
/// `GetPercentLoc`: the arc-length error at which Newton's method stops.
const NEWTON_TOLERANCE: f32 = 1e-4;
/// `NiPosKey::CoordinateFrame`: below this the curvature is zero and the normal is built from the tangent.
const CURVATURE_EPSILON: f32 = 1e-6;
/// `NiPoint3::Unitize` / `UnitCross`: shorter vectors become zero.
const UNITIZE_EPSILON: f32 = 1e-6;
/// `NiPosKey::ms_iSampleSize`: curvature samples per segment (the segment ends included, so one more).
const CURVATURE_SAMPLE_SIZE: usize = 20;
/// `CalcRefFrame` bank scale `4 / pi`: at the maximum curvature `atan(1)` makes the factor 1.
const BANK_SCALE: f32 = 4.0 / std::f32::consts::PI;

/// The node channels a path interpolator sets: the translation, and the rotation with the follow flag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathPose {
    pub translation: Vec3,
    pub rotation: Option<Mat3>,
}

/// `NiPosKey::CoordinateFrame` at one point.
struct Frame {
    tangent: Vec3,
    normal: Vec3,
    binormal: Vec3,
    /// The pseudo-curvature `|D1 x D2| / |D1|^2`.
    curvature: f32,
}

/// `NiPoint3::Unitize`.
fn unitize(v: Vec3) -> Vec3 {
    let length = v.length();
    if length > UNITIZE_EPSILON { v * (1.0 / length) } else { Vec3::ZERO }
}

/// First and second derivative of segment `index` at `t`, as vectors.
fn derivatives(path: &Curve<[f32; 3]>, index: usize, t: f32) -> (Vec3, Vec3) {
    path.segment_derivatives(index, t)
        .map_or((Vec3::ZERO, Vec3::ZERO), |(d1, d2)| (Vec3::from_array(d1), Vec3::from_array(d2)))
}

/// `PathSpeed`: `|D1|`.
fn path_speed(path: &Curve<[f32; 3]>, index: usize, t: f32) -> f32 {
    derivatives(path, index, t).0.length()
}

/// `PathLength`: the arc length of segment `index` over [0, t], by 5-point Gauss-Legendre quadrature.
fn path_length(path: &Curve<[f32; 3]>, index: usize, t: f32) -> f32 {
    let sum: f32 = GAUSS_ROOTS
        .iter()
        .zip(GAUSS_COEFFICIENTS)
        .map(|(root, coefficient)| coefficient * path_speed(path, index, t * root))
        .sum();
    sum * t
}

/// `GetTotalPathLength`: the arc length from the first key to each key.
fn partial_lengths(path: &Curve<[f32; 3]>) -> Vec<f32> {
    let mut lengths = Vec::with_capacity(path.keys.len());
    if path.keys.is_empty() {
        return lengths;
    }
    lengths.push(0.0);
    for index in 0..path.keys.len() - 1 {
        let previous = lengths[index];
        lengths.push(previous + path_length(path, index, 1.0));
    }
    lengths
}

/// The key type's `ComputeMaxCurvature`: the largest sampled pseudo-curvature of Bezier and TCB segments,
/// zero for linear and step keys.
fn max_curvature(path: &Curve<[f32; 3]>) -> f32 {
    if !matches!(path.kind, KeyKind::Bezier | KeyKind::Tcb) {
        return 0.0;
    }
    let delta = 1.0 / CURVATURE_SAMPLE_SIZE as f32;
    let mut max = 0.0f32;
    for index in 0..path.keys.len().saturating_sub(1) {
        for sample in 0..=CURVATURE_SAMPLE_SIZE {
            let (d1, d2) = derivatives(path, index, sample as f32 * delta);
            let curvature = d1.cross(d2).length() / d1.dot(d1);
            if curvature > max {
                max = curvature;
            }
        }
    }
    max
}

/// `NiPosKey::CoordinateFrame` of segment `index` at `t`.
fn coordinate_frame(path: &Curve<[f32; 3]>, index: usize, t: f32) -> Frame {
    let (d1, d2) = derivatives(path, index, t);
    let length_sqr = d1.dot(d1);
    let inverse_length = 1.0 / length_sqr.sqrt();
    let tangent = d1 * inverse_length;
    let mut curvature = d1.cross(d2).length() * inverse_length * inverse_length;
    let normal = if curvature.abs() > CURVATURE_EPSILON {
        d2 * length_sqr - d1 * d1.dot(d2)
    } else {
        curvature = 0.0;
        if tangent.x.abs() > CURVATURE_EPSILON || tangent.y.abs() > CURVATURE_EPSILON {
            Vec3::new(tangent.y, -tangent.x, 0.0)
        } else {
            Vec3::new(0.0, -tangent.z, tangent.y)
        }
    };
    let normal = unitize(normal);
    Frame { tangent, normal, binormal: tangent.cross(normal), curvature }
}

impl PathInterp {
    /// The interpolator with the derived arc lengths and maximum curvature (`LinkObject`).
    pub fn new(
        path: Option<Curve<[f32; 3]>>,
        percent: Option<Curve<f32>>,
        flags: u16,
        follow_axis: i16,
        bank_dir: i32,
        max_bank_angle: f32,
        smoothing: f32,
    ) -> Self {
        let partial_lengths = path.as_ref().map(partial_lengths).unwrap_or_default();
        let max_curvature = path.as_ref().map_or(0.0, max_curvature);
        Self { path, percent, flags, follow_axis, bank_dir, max_bank_angle, smoothing, partial_lengths, max_curvature }
    }

    fn flag(&self, flag: u16) -> bool {
        self.flags & flag != 0
    }

    /// `NiPathInterpolator::Update` at `time`; `None` without percentage or path keys (the node keeps its
    /// channels).
    pub fn eval(&self, time: f32) -> Option<PathPose> {
        let value = self.percent.as_ref()?.eval(time)?;
        let path = self.path.as_ref()?;
        let follow = self.flag(FLAG_FOLLOW);
        if path.keys.len() < 2 {
            let key = path.keys.first()?;
            let rotation = follow.then_some(Mat3::IDENTITY);
            return Some(PathPose { translation: Vec3::from_array(key.value), rotation });
        }
        let (index, t) = self.location(path, value);
        let rotation = follow.then(|| self.reference_frame(path, index, t));
        let translation = Vec3::from_array(path.segment_value(index, t)?);
        Some(PathPose { translation, rotation })
    }

    /// `GetLoc`: the segment and normalized time of a percentage-curve value. The path has two keys or more.
    fn location(&self, path: &Curve<[f32; 3]>, value: f32) -> (usize, f32) {
        if self.flag(FLAG_CONST_VELOCITY) {
            return self.percent_location(path, value);
        }
        let keys = &path.keys;
        let last = keys.len() - 1;
        if value <= keys[0].time {
            return (0, 0.0);
        }
        if value >= keys[last].time {
            return (last - 1, 1.0);
        }
        let index = (0..last).rev().find(|&i| keys[i].time <= value).unwrap_or(0);
        let span = keys[index + 1].time - keys[index].time;
        (index, if span > 0.0 { (value - keys[index].time) / span } else { 0.0 })
    }

    /// `GetPercentLoc`: the segment and normalized time at the fraction `percent` of the arc length. Values
    /// outside [0, 1] wrap (`NiFmod`).
    fn percent_location(&self, path: &Curve<[f32; 3]>, percent: f32) -> (usize, f32) {
        let last = path.keys.len() - 1;
        let mut percent = percent;
        if percent == 0.0 {
            return (0, 0.0);
        } else if percent < 0.0 {
            percent = percent % 1.0 + 1.0;
        }
        if percent == 1.0 {
            return (last - 1, 1.0);
        } else if percent > 1.0 {
            percent %= 1.0;
        }
        let lengths = &self.partial_lengths;
        let Some(&total) = lengths.last() else { return (0, 0.0) };
        let length = percent * total;
        // The SDK's search always finds a segment for a fraction in [0, 1); rounding past the last partial
        // length falls back to the last segment here.
        let index = (0..last).find(|&i| length <= lengths[i + 1]).unwrap_or(last - 1);
        let distance = length - lengths[index];
        let segment_length = lengths[index + 1] - lengths[index];
        let mut t = if segment_length > 0.0 { distance / segment_length } else { 0.0 };
        for _ in 0..NEWTON_MAX_ITERATIONS {
            let difference = path_length(path, index, t) - distance;
            if difference.abs() <= NEWTON_TOLERANCE {
                break;
            }
            let speed = path_speed(path, index, t);
            if speed <= 0.0 {
                break;
            }
            t -= difference / speed;
        }
        (index, t)
    }

    /// `CalcRefFrame`: the node rotation at segment `index`, normalized time `t`. The path has two keys or
    /// more.
    fn reference_frame(&self, path: &Curve<[f32; 3]>, index: usize, t: f32) -> Mat3 {
        let count = path.keys.len();
        let Frame { tangent, mut normal, mut binormal, mut curvature } = coordinate_frame(path, index, t);
        let (mut direction, mut up, mut right);
        if self.flag(FLAG_ALLOW_FLIP) {
            let smoothing = self.smoothing;
            let open = self.flag(FLAG_OPEN_CURVE);
            let mut blend = |a0: f32, first: Frame, second: Frame| {
                let a1 = 1.0 - a0;
                normal = unitize(first.normal * a0 + second.normal * a1);
                curvature = a0 * first.curvature + a1 * second.curvature;
                binormal = tangent.cross(normal);
            };
            if smoothing != 0.0 {
                if t >= 1.0 - smoothing {
                    let next = index + 1;
                    let wrapped = if !open && next == count - 1 { 0 } else { next };
                    if wrapped < count - 1 {
                        let a0 = (1.0 - t + smoothing) / (2.0 * smoothing);
                        blend(
                            a0,
                            coordinate_frame(path, index, 1.0 - smoothing),
                            coordinate_frame(path, wrapped, smoothing),
                        );
                    }
                } else if t <= smoothing {
                    let wrapped = if !open && index == 0 { count - 1 } else { index };
                    if wrapped >= 1 {
                        let a1 = (smoothing - t) / (2.0 * smoothing);
                        blend(
                            1.0 - a1,
                            coordinate_frame(path, index, smoothing),
                            coordinate_frame(path, wrapped - 1, 1.0 - smoothing),
                        );
                    }
                }
            }
            (direction, up, right) = (tangent, -normal, -binormal);
        } else {
            let unit_up = unitize(Vec3::new(0.0, 0.0, 1.0).cross(tangent));
            (direction, up, right) = (tangent, unit_up, tangent.cross(unit_up));
        }
        if self.flag(FLAG_FLIP) {
            (direction, up) = (-direction, -up);
        }
        match self.follow_axis {
            FOLLOW_AXIS_Y => (direction, up) = (-up, direction),
            FOLLOW_AXIS_Z => (direction, right) = (-right, direction),
            _ => {}
        }
        let frame = Mat3::from_cols(direction, up, right);
        if !self.flag(FLAG_BANK) {
            return frame;
        }
        let mut bank = self.bank_dir as f32 * self.max_bank_angle;
        if curvature < self.max_curvature {
            bank *= BANK_SCALE * (curvature / self.max_curvature).atan();
        }
        make_rotation(bank, tangent).mul_mat3(&frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nif::desc::{Key, Tcb};

    const TOLERANCE: f32 = 1e-3;

    fn key<T: Copy>(time: f32, value: T, tangent: T) -> Key<T> {
        Key { time, value, in_tangent: tangent, out_tangent: tangent, tcb: Tcb::default() }
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < TOLERANCE
    }

    /// A Bezier path whose first half is slow (short tangents) and whose second half covers most of the
    /// distance, with a linear percentage curve over one second.
    fn uneven_path(flags: u16, follow_axis: i16) -> PathInterp {
        let path = Curve {
            kind: KeyKind::Linear,
            keys: vec![key(0.0, [0.0, 0.0, 0.0], [0.0; 3]), key(0.5, [1.0, 0.0, 0.0], [0.0; 3]), key(1.0, [4.0, 0.0, 0.0], [0.0; 3])],
        };
        let percent = Curve { kind: KeyKind::Linear, keys: vec![key(0.0, 0.0, 0.0), key(1.0, 1.0, 0.0)] };
        PathInterp::new(Some(path), Some(percent), flags, follow_axis, 1, 0.0, 0.0)
    }

    #[test]
    fn arc_length_matches_the_polyline() {
        let path = uneven_path(0, 0);
        assert_eq!(path.partial_lengths.len(), 3);
        assert!((path.partial_lengths[1] - 1.0).abs() < TOLERANCE);
        assert!((path.partial_lengths[2] - 4.0).abs() < TOLERANCE);
    }

    #[test]
    fn constant_velocity_moves_by_arc_length() {
        // Without constant velocity the percentage is a key time: half way is the second key at x = 1.
        let timed = uneven_path(0, 0);
        assert!(close(timed.eval(0.5).unwrap().translation, Vec3::new(1.0, 0.0, 0.0)));
        // With it, half the arc length of 4 is x = 2; a quarter is x = 1.
        let even = uneven_path(FLAG_CONST_VELOCITY, 0);
        assert!(close(even.eval(0.5).unwrap().translation, Vec3::new(2.0, 0.0, 0.0)));
        assert!(close(even.eval(0.25).unwrap().translation, Vec3::new(1.0, 0.0, 0.0)));
        // Values past 1 wrap; exactly 1 is the end.
        assert!(close(even.eval(1.0).unwrap().translation, Vec3::new(4.0, 0.0, 0.0)));
        assert!(even.eval(0.5).unwrap().rotation.is_none());
    }

    #[test]
    fn constant_velocity_on_a_bezier_curve() {
        // A smooth Bezier quarter-turn whose speed varies: equal steps of the percentage cover equal arc
        // lengths, and each point solves the quadrature arc length to Newton's tolerance.
        let path = Curve {
            kind: KeyKind::Bezier,
            keys: vec![
                Key { time: 0.0, value: [0.0, 0.0, 0.0], in_tangent: [0.0; 3], out_tangent: [2.0, 0.0, 0.0], tcb: Tcb::default() },
                Key { time: 1.0, value: [1.0, 1.0, 0.0], in_tangent: [0.0, 2.0, 0.0], out_tangent: [0.0; 3], tcb: Tcb::default() },
            ],
        };
        let percent = Curve { kind: KeyKind::Linear, keys: vec![key(0.0, 0.0, 0.0), key(1.0, 1.0, 0.0)] };
        let interp = PathInterp::new(Some(path.clone()), Some(percent), FLAG_CONST_VELOCITY, 0, 1, 0.0, 0.0);
        let total = interp.partial_lengths[1];
        let steps = 8;
        for i in 1..steps {
            let fraction = i as f32 / steps as f32;
            let (index, t) = interp.percent_location(&path, fraction);
            assert!((path_length(&path, index, t) - fraction * total).abs() <= NEWTON_TOLERANCE);
        }
        let points: Vec<Vec3> = (0..=steps).map(|i| interp.eval(i as f32 / steps as f32).unwrap().translation).collect();
        let chord = |a: Vec3, b: Vec3| (a - b).length();
        let chords: Vec<f32> = points.windows(2).map(|w| chord(w[0], w[1])).collect();
        let expected = total / steps as f32;
        for length in chords {
            // Chords are a little shorter than their arcs; the steps are even within a few percent.
            assert!((length - expected).abs() < expected * 0.05, "chord {length} vs arc step {expected}");
        }
    }

    #[test]
    fn follow_turns_the_node_along_the_path() {
        // Along +X without allow-flip: direction X, up = Z x X = Y, right = X x Y = Z.
        let follow = uneven_path(FLAG_FOLLOW, 0);
        let rotation = follow.eval(0.25).unwrap().rotation.unwrap();
        assert!(close(rotation.col(0), Vec3::new(1.0, 0.0, 0.0)));
        assert!(close(rotation.col(1), Vec3::new(0.0, 1.0, 0.0)));
        assert!(close(rotation.col(2), Vec3::new(0.0, 0.0, 1.0)));
        // Flip negates direction and up.
        let flipped = uneven_path(FLAG_FOLLOW | FLAG_FLIP, 0).eval(0.25).unwrap().rotation.unwrap();
        assert!(close(flipped.col(0), Vec3::new(-1.0, 0.0, 0.0)));
        assert!(close(flipped.col(1), Vec3::new(0.0, -1.0, 0.0)));
        // Follow axis Y: the node's Y axis points along the path.
        let axis_y = uneven_path(FLAG_FOLLOW, FOLLOW_AXIS_Y).eval(0.25).unwrap().rotation.unwrap();
        assert!(close(axis_y.col(1), Vec3::new(1.0, 0.0, 0.0)));
        assert!(close(axis_y.col(0), Vec3::new(0.0, -1.0, 0.0)));
        // Follow axis Z: the node's Z axis points along the path.
        let axis_z = uneven_path(FLAG_FOLLOW, FOLLOW_AXIS_Z).eval(0.25).unwrap().rotation.unwrap();
        assert!(close(axis_z.col(2), Vec3::new(1.0, 0.0, 0.0)));
        assert!(close(axis_z.col(0), Vec3::new(0.0, 0.0, -1.0)));
    }

    #[test]
    fn bank_turns_about_the_tangent() {
        // A straight path has zero curvature, so the bank factor is atan(0) = 0 when the maximum is above it;
        // with a zero maximum curvature the full angle applies.
        let mut path = uneven_path(FLAG_FOLLOW | FLAG_BANK, 0);
        path.max_bank_angle = std::f32::consts::FRAC_PI_2;
        let rotation = path.eval(0.25).unwrap().rotation.unwrap();
        assert!(close(rotation.col(0), Vec3::new(1.0, 0.0, 0.0)));
        // MakeRotation(+90 degrees, X) turns Y to -Z (the SDK's rotation is clockwise).
        assert!(close(rotation.col(1), Vec3::new(0.0, 0.0, -1.0)));
    }

    #[test]
    fn no_percentage_keys_leave_the_node() {
        let mut path = uneven_path(0, 0);
        path.percent = None;
        assert!(path.eval(0.5).is_none());
    }
}
