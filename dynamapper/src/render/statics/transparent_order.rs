//! Stable ties for instance batches whose mesh positions are placeholder corners, and for the client light sprites,
//! which blend over one another.
//!
//! Also the walk of a transparent phase's batches (`drawn_batches`) for the passes that redraw its items outside
//! Bevy's own phase rendering: the light scene, the deferred light sprites and the static pixel query.

use bevy::core_pipeline::core_3d::Transparent3d;
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_phase::{PhaseItem, ViewSortedRenderPhases};
use bevy::render::sync_world::MainEntity;
use bevy::math::FloatOrd;
use std::collections::HashMap;
use super::static_batches::StaticChunkBatchKey;

#[derive(Component, Clone, Copy, ExtractComponent)]
#[extract_app(bevy::render::RenderApp)]
pub(crate) struct StaticTransparentBatchOrder {
    pub key: StaticChunkBatchKey,
    // Ground before upright art when their stock transparent distances tie.
    pub kind: u8,
}

/// A client light sprite's place in its scan (`client_light_sprites`): its map block, (gy, gx), then its place among
/// the block's sprites. Where their depths tie the sprites draw in this order, whatever views the launch visited
/// before; it stays as long as the sprite's block holds the same sprites, so a kept sprite keeps its own.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, ExtractComponent)]
#[extract_app(bevy::render::RenderApp)]
pub(crate) struct ClientLightSpriteOrder {
    pub block: (u32, u32),
    pub index: u32,
}

/// The order of tied items: the others first, as they came, then the light sprites in their scan order, then the
/// static batches by their key.
pub(super) fn stabilize_equal_depth_batches(
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    batches: Query<(&MainEntity, &StaticTransparentBatchOrder)>,
    light_sprites: Query<&ClientLightSpriteOrder>,
) {
    let orders: HashMap<_, _> = batches.iter().map(|(entity, order)| (*entity,
        (order.kind, order.key.map_id, order.key.gy, order.key.gx, order.key.scale))).collect();
    for phase in phases.values_mut() {
        let key = |item: &Transparent3d| (FloatOrd(item.distance), orders.get(&item.entity.1).copied(),
            light_sprites.get(item.entity.0).ok().copied());
        // Retained phases are normally already canonical. Only a changed tie
        // group needs another sort after Bevy's distance/material-bias ordering.
        if !phase.items.values().map(key).is_sorted() {
            phase.items.sort_by_key(|_, item| key(item));
        }
    }
}

/// The items of a sorted phase that draw, in order, walked as Bevy's `SortedRenderPhase::render_range` walks them: an
/// item with an empty batch range draws nothing, and an item that draws covers the `batch_range.len()` items from
/// it, the instances batched into it.
///
/// Only those items carry this frame's batch. Bevy keeps sorted phase items from frame to frame, and its batching
/// (`batch_and_prepare_sorted_render_phase`) writes the batch range and the indirect parameters index into each
/// batch's first item alone: the items a batch covers keep those of their queueing (`0..1`, drawn directly) or of a
/// batch they began in an earlier frame. Drawn, such an item draws another item's instance again, or reads indirect
/// parameters that this frame gave another batch or another view, or that lie past the end of this frame's buffer:
/// Bevy alternates two indirect parameters buffers from frame to frame (`collect_buffers_for_phase`), each grown only
/// to the most batches of its own frames, so in the first frames of a start an index from the frame before can
/// overrun this one's, an indirect draw wgpu refuses and Bevy quits on. The light sprites batch (one mesh, a material
/// per light texture).
pub(crate) fn drawn_batches<'a, I: PhaseItem>(items: impl IntoIterator<Item = &'a I>) -> impl Iterator<Item = &'a I> {
    let mut covered = 0usize;
    items.into_iter().filter(move |item| {
        if covered > 0 {
            covered -= 1;
            return false;
        }
        covered = item.batch_range().len().saturating_sub(1);
        !item.batch_range().is_empty()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::core_pipeline::core_3d::TransparentSortingInfo3d;
    use bevy::render::render_phase::{DrawFunctionId, PhaseItemExtraIndex};
    use bevy::render::render_resource::CachedRenderPipelineId;
    use std::ops::Range;

    fn item(batch_range: Range<u32>, indirect: Option<Range<u32>>) -> Transparent3d {
        Transparent3d {
            sorting_info: TransparentSortingInfo3d::AlwaysOnTop,
            distance: 0.0,
            pipeline: CachedRenderPipelineId::INVALID,
            entity: (Entity::PLACEHOLDER, MainEntity::from(Entity::PLACEHOLDER)),
            draw_function: DrawFunctionId(0),
            batch_range,
            extra_index: indirect.map_or(PhaseItemExtraIndex::None, |range| {
                PhaseItemExtraIndex::IndirectParametersIndex { range, batch_set_index: None }
            }),
            indexed: true,
        }
    }

    /// A phase as Bevy's batching leaves it: a batch of three sprites whose second sprite was queued this frame
    /// (`0..1`, no indirect parameters) and whose third began a batch two frames ago (its indirect parameters past
    /// the end of this frame's), an item with an empty batch range, then a batch of one. Only the batches' first items
    /// draw.
    #[test]
    fn only_the_first_item_of_each_batch_draws() {
        let items = [
            item(4..7, Some(2..3)),
            item(0..1, None),
            item(9..11, Some(40..41)),
            item(0..0, None),
            item(7..8, Some(3..4)),
        ];
        let drawn = drawn_batches(&items).map(|item| item.batch_range.clone()).collect::<Vec<_>>();
        assert_eq!(drawn, [4..7, 7..8]);
    }

    /// Without batching (no indirect drawing, or unbatchable items) every item draws its own instance.
    #[test]
    fn unbatched_items_each_draw() {
        let items = [item(0..1, None), item(1..2, None), item(2..3, None)];
        assert_eq!(drawn_batches(&items).count(), items.len());
    }

    /// A batch that claims more items than are left ends the walk.
    #[test]
    fn a_batch_past_the_last_item_ends_the_walk() {
        let items = [item(0..1, None), item(1..5, Some(1..2)), item(0..1, None)];
        let drawn = drawn_batches(&items).map(|item| item.batch_range.clone()).collect::<Vec<_>>();
        assert_eq!(drawn, [0..1, 1..5]);
    }
}
