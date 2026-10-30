//! Gamebryo NIF effects (`UOParticleEffectNif`): compiled description, runtime, render output.
//!
//! - [`desc`]: [`NifEffectDesc`], the plain-data description of a NIF scene (nodes, meshes, particle systems,
//!   materials, textures, controllers with their keys).
//! - [`keys`]: key interpolation and controller time (`ComputeScaledTime`), as the Gamebryo SDK computes them.
//! - [`NifEffect`] / [`NifEffectInstance`]: the scene, particle systems and controllers run as UOSA.exe runs
//!   them, behind [`crate::ParticleSystemInstance`]; meshes render as [`crate::render::MeshDraw`]s, particles
//!   as sprite batches.
//! - `compile` (feature `nif`): [`NifEffectDesc`] from a parsed [`gamebryo_nif::NifFile`].
//!
//! Semantics follow the Gamebryo 2.6 SDK sources and the client's own code where it differs (addresses in
//! the doc comments refer to UOSA.exe).

#[cfg(feature = "nif")]
pub mod compile;
pub mod desc;
pub mod instance;
pub mod keys;
pub mod particles;
mod path;
pub(crate) mod scene;

#[cfg(feature = "nif")]
pub use compile::{NifCompileOutput, compile};
pub use desc::NifEffectDesc;
pub use instance::{NifEffect, NifEffectInstance, default_camera, texture_transform_matrix};
pub use particles::NifParticle;
pub use path::PathPose;
