//! Static web map export: the renderer's export mode writes frames to a spool, this crate cuts them into a
//! WebP tile pyramid with a Leaflet viewer.
//!
//! The design and the renderer contract: `docs/agent_notes/proposals/2026-11-01-web-map-export.md`.

pub mod export;
pub mod formats;
pub mod geometry;
pub mod packages;
#[cfg(unix)]
pub mod pilot;
pub mod raster;
pub mod storage;
pub mod viewer;
pub mod webp;
