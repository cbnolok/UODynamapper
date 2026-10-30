//! CPU particle simulation of the Ultima Online Enhanced Client effects, engine- and GUI-independent.
//!
//! - [`chaos`]: the Chaos engine (`.ems` scripts): compiled description, runtime, render output.
//! - [`nif`]: Gamebryo NIF effects (`.nif` scenes): compiled description, runtime, render output.
//! - [`effect`]: the effect layer (a `ParticleEffect` definition around any [`ParticleSystemInstance`]).
//! - [`render`]: draw batches, sprites and mesh instances, and CPU quad expansion.
//! - [`raster`]: a small software rasterizer of render frames, for previews.
//! - [`hued`]: hued NIF effects (`UOHuedParticleEffectNif`): half-speed clock and the hue atlas render pass.
//! - [`rng`]: the client's MT19937, behind the [`rng::Rng`] trait, and the C runtime `rand()` NIF effects draw
//!   from ([`rng::MsvcRand`]).
//!
//! Simulation is deterministic for a given seed and update sequence. Time steps are whole milliseconds, as in
//! the client.
//!
//! Feature `uocf` (default) adds the compiler from uocf's Chaos syntax tree and the conversion of uocf effect
//! definitions. Feature `nif` (default) adds the compiler from gamebryo-nif's parsed NIF files. Without them the
//! crate only runs descriptions it is given.

pub mod chaos;
pub mod codec;
pub mod effect;
pub mod hued;
pub mod math;
pub mod nif;
pub mod raster;
pub mod render;
pub mod rng;
pub mod system;

pub use chaos::{ChaosEffectInstance, ChaosSystem, ChaosSystemDesc};
pub use effect::{EffectInstance, EffectPlacement, ParticleEffectParams};
pub use math::{Mat3, Vec3};
pub use nif::{NifEffect, NifEffectDesc, NifEffectInstance};
pub use render::{CameraFrame, DrawBatch, MeshInstance, RenderFrame, SpriteInstance, SpriteKind};
pub use rng::{CrtRng, MsvcRand, Mt19937, Rng};
pub use system::{NodeTransform, ParticleSystemInstance};
