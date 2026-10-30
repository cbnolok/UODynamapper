//! Stable ties for instance batches whose mesh positions are placeholder corners.

use bevy::core_pipeline::core_3d::Transparent3d;
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_phase::ViewSortedRenderPhases;
use bevy::render::sync_world::MainEntity;
use bevy::math::FloatOrd;
use std::collections::HashMap;
use super::statics_collect::StaticChunkBatchKey;

#[derive(Component, Clone, Copy, ExtractComponent)]
#[extract_app(bevy::render::RenderApp)]
pub(super) struct StaticTransparentBatchOrder {
    pub key: StaticChunkBatchKey,
    // Ground before upright art when their stock transparent distances tie.
    pub kind: u8,
}

pub(super) fn stabilize_equal_depth_batches(
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    batches: Query<(&MainEntity, &StaticTransparentBatchOrder)>,
) {
    let orders: HashMap<_, _> = batches.iter().map(|(entity, order)| (*entity,
        (order.kind, order.key.map_id, order.key.gy, order.key.gx, order.key.scale))).collect();
    for phase in phases.values_mut() {
        let key = |item: &Transparent3d| (FloatOrd(item.distance), orders.get(&item.entity.1).copied());
        // Retained phases are normally already canonical. Only a changed tie
        // group needs another sort after Bevy's distance/material-bias ordering.
        if !phase.items.values().map(key).is_sorted() {
            phase.items.sort_by_key(|_, item| key(item));
        }
    }
}
