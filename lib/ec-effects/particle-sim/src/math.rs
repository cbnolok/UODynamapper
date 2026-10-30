//! Minimal f32 vector math. The client simulates in single precision (SSE `SUBSS`/`MULSS`/`ADDSS`), so every
//! operation here is plain f32 arithmetic in the same order as the client's code, with no fused multiply-add.

use std::ops::{Add, AddAssign, Mul, MulAssign, Neg, Sub, SubAssign};

/// A 3-component vector.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    pub const X: Self = Self::new(1.0, 0.0, 0.0);
    pub const Y: Self = Self::new(0.0, 1.0, 0.0);
    pub const Z: Self = Self::new(0.0, 0.0, 1.0);

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub const fn from_array(values: [f32; 3]) -> Self {
        Self::new(values[0], values[1], values[2])
    }

    pub const fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }

    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    /// Component-wise product.
    pub fn mul_elements(self, other: Self) -> Self {
        Self::new(self.x * other.x, self.y * other.y, self.z * other.z)
    }

    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// The unit vector, or zero for a zero vector.
    pub fn normalize_or_zero(self) -> Self {
        let length = self.length();
        if length > 0.0 { self * (1.0 / length) } else { Self::ZERO }
    }

    pub fn is_zero(self) -> bool {
        self == Self::ZERO
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// `self` rotated by `angle` radians about the unit `axis`, counter-clockwise when looking down the axis
    /// (Rodrigues' formula).
    pub fn rotated_about(self, axis: Self, angle: f32) -> Self {
        let (sin, cos) = angle.sin_cos();
        self * cos + axis.cross(self) * sin + axis * (axis.dot(self) * (1.0 - cos))
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }
}

impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }
}

impl Mul<f32> for Vec3 {
    type Output = Self;
    fn mul(self, factor: f32) -> Self {
        Self::new(self.x * factor, self.y * factor, self.z * factor)
    }
}

impl Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}

impl AddAssign for Vec3 {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

impl SubAssign for Vec3 {
    fn sub_assign(&mut self, other: Self) {
        *self = *self - other;
    }
}

impl MulAssign<f32> for Vec3 {
    fn mul_assign(&mut self, factor: f32) {
        *self = *self * factor;
    }
}

/// A 3x3 matrix stored by rows, applied to column vectors: `x' = row0 · v` (Gamebryo `NiMatrix3`, as the
/// client's `node_rot * v`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat3 {
    pub rows: [[f32; 3]; 3],
}

impl Default for Mat3 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mat3 {
    pub const IDENTITY: Self = Self { rows: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] };

    pub const fn from_rows(rows: [[f32; 3]; 3]) -> Self {
        Self { rows }
    }

    pub fn from_cols(c0: Vec3, c1: Vec3, c2: Vec3) -> Self {
        Self { rows: [[c0.x, c1.x, c2.x], [c0.y, c1.y, c2.y], [c0.z, c1.z, c2.z]] }
    }

    pub fn col(&self, index: usize) -> Vec3 {
        Vec3::new(self.rows[0][index], self.rows[1][index], self.rows[2][index])
    }

    pub fn mul_vec3(&self, v: Vec3) -> Vec3 {
        let [r0, r1, r2] = self.rows;
        Vec3::new(
            r0[0] * v.x + r0[1] * v.y + r0[2] * v.z,
            r1[0] * v.x + r1[1] * v.y + r1[2] * v.z,
            r2[0] * v.x + r2[1] * v.y + r2[2] * v.z,
        )
    }

    pub fn mul_mat3(&self, other: &Self) -> Self {
        Self::from_cols(self.mul_vec3(other.col(0)), self.mul_vec3(other.col(1)), self.mul_vec3(other.col(2)))
    }

    pub fn is_finite(&self) -> bool {
        self.rows.iter().flatten().all(|value| value.is_finite())
    }
}
