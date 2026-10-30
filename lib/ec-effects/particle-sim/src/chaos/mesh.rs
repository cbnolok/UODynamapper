//! MESH particles (`MeshParticle`, ctor 0x008c6400): a clone of a `.nif` effect per particle, placed and
//! animated by `ParticleSet::Update` (0x008b2de0, case 2).

use std::fmt;
use std::sync::Arc;

use crate::math::{Mat3, Vec3};
use crate::nif::scene::make_rotation;
use crate::nif::{NifEffect, NifEffectInstance};
use crate::render::RenderFrame;
use crate::system::NodeTransform;

/// Step of the controller sub-steps of a clone over its particle's last update (`ParticleSet::Update`,
/// 0x008b2de0, the literal 0.0333).
pub const MESH_SUBSTEP_S: f32 = 0.0333;
/// Seed of the C runtime `rand()` each clone draws from. The client shares one generator with the whole
/// process; here each clone owns one, seeded alike.
pub const MESH_CLONE_SEED: u32 = 1;

/// Resolves the `.nif` a MESH sequence names (`meshname`, as in the script) to its prepared effect, or `None`
/// when it is missing. The client's `ParticleSystemManager::LoadMesh` (0x008a8440) loads each name once and
/// hands out clones; cache by name behind the resolver to do the same.
#[derive(Clone)]
pub struct MeshSource(Arc<dyn Fn(&str) -> Option<Arc<NifEffect>> + Send + Sync>);

impl MeshSource {
    pub fn new(resolve: impl Fn(&str) -> Option<Arc<NifEffect>> + Send + Sync + 'static) -> Self {
        Self(Arc::new(resolve))
    }

    pub fn resolve(&self, name: &str) -> Option<Arc<NifEffect>> {
        (self.0)(name)
    }
}

impl fmt::Debug for MeshSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MeshSource")
    }
}

/// `NiMatrix3::FromEulerAnglesXYZ` (UOSA.exe 0x0066b640, `NiMatrix3.cpp:1076-1084`): `X · (Y · Z)` with the
/// SDK's rotation matrices.
pub fn euler_xyz(angles: [f32; 3]) -> Mat3 {
    let [x, y, z] = angles;
    make_rotation(x, Vec3::X).mul_mat3(&make_rotation(y, Vec3::Y).mul_mat3(&make_rotation(z, Vec3::Z)))
}

/// Where a clone goes: the particle's state as `ParticleSet::Update` reads it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MeshPlacement {
    pub position: Vec3,
    /// Effect node rotation, times the particle's Euler rotation when the sequence rotates its particles.
    pub rotation: Mat3,
    pub size: f32,
    /// Particle age, seconds.
    pub age: f32,
    /// Seconds of the particle's last update step.
    pub step: f32,
}

/// The clones of one MESH sequence. The client keeps `numparticles` clones and gives the k-th live particle
/// (deque order) the k-th clone; `MeshParticle::SetActiveCount` (0x008c6520) culls the rest.
#[derive(Debug, Clone)]
pub(crate) struct MeshClones {
    effect: Arc<NifEffect>,
    clones: Vec<NifEffectInstance>,
    active: usize,
}

impl MeshClones {
    pub fn new(effect: Arc<NifEffect>) -> Self {
        Self { effect, clones: Vec::new(), active: 0 }
    }

    /// Places and animates the clone of every live particle (`ParticleSet::Update` case 2): local translate =
    /// position, rotation, scale = |size|; when the last step was longer than [`MESH_SUBSTEP_S`] the clone is
    /// first updated at `age - step + MESH_SUBSTEP_S`, `+ 2 * MESH_SUBSTEP_S`, ... while more than a sub-step
    /// remains, then at `age`.
    pub fn update(&mut self, placements: impl ExactSizeIterator<Item = MeshPlacement>) {
        self.active = placements.len();
        while self.clones.len() < self.active {
            self.clones.push(NifEffectInstance::with_seed(Arc::clone(&self.effect), MESH_CLONE_SEED));
        }
        for (clone, placement) in self.clones.iter_mut().zip(placements) {
            let node = NodeTransform {
                position: placement.position,
                rotation: placement.rotation,
                scale: placement.size.abs(),
                velocity: None,
            };
            let mut remaining = placement.step;
            let mut time = placement.age - placement.step;
            while remaining > MESH_SUBSTEP_S {
                time += MESH_SUBSTEP_S;
                clone.pose_at(time, &node);
                remaining -= MESH_SUBSTEP_S;
            }
            clone.pose_at(placement.age, &node);
        }
    }

    /// The draws of the active clones, appended in deque order; each geometry keeps its own sort item.
    /// `order` is the next free scene order number; returns the one after the clones'.
    pub fn render(&self, out: &mut RenderFrame, mut order: u32) -> u32 {
        let mut frame = RenderFrame::default();
        for clone in &self.clones[..self.active] {
            frame.clear();
            clone.render_frame(&mut frame);
            order = out.append(&frame, order);
        }
        order
    }
}
