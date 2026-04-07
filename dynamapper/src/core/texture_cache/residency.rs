//! Shared residency-planning helpers for texture collections.
//!
//! This module intentionally does not know anything about land texmaps, art tiles,
//! animation frames, or any other specific asset family. It only models the common
//! questions that every texture collection has to answer before allocating GPU array
//! layers:
//!
//! - should the collection run in LRU mode or preload mode?
//! - how are texture IDs grouped for storage purposes?
//! - how many layers are needed for each group?
//! - in what deterministic order should preloaded layers be assigned?
//!
//! Collection-specific modules are expected to provide the group type and build a
//! `TextureResidencyPlan<Group>` from their source data.

/// High-level residency mode for a texture collection.
///
/// `LruCache` keeps only a working set resident and relies on eviction/expansion.
/// `PreloadFullCollection` reserves deterministic layers for every planned texture
/// ID at startup so the collection behaves like a fully resident atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureResidencyStrategy {
    LruCache,
    PreloadFullCollection,
}

impl TextureResidencyStrategy {
    /// Maps a user-facing boolean toggle to the shared residency strategy enum.
    pub fn from_preload_enabled(preload_enabled: bool) -> Self {
        if preload_enabled {
            Self::PreloadFullCollection
        } else {
            Self::LruCache
        }
    }

    pub fn preloads_full_collection(self) -> bool {
        matches!(self, Self::PreloadFullCollection)
    }
}

/// Texture IDs belonging to the same logical storage group.
///
/// A group is collection-defined. For terrain this is `Small` vs `Big`, but future
/// systems could use animation body IDs, art texture classes, or any other split that
/// maps cleanly onto separate GPU arrays or residency policies.
#[derive(Clone, Debug)]
pub struct TextureResidencyBucket<Group> {
    pub group: Group,
    pub texture_ids: Vec<u16>,
}

/// Collection-agnostic plan describing which texture IDs belong to each group.
///
/// The plan is deliberately lightweight: it is only about grouping and deterministic
/// iteration order. Upload mechanics, byte decoding, and cache bookkeeping remain in
/// the collection-specific cache implementation.
#[derive(Clone, Debug, Default)]
pub struct TextureResidencyPlan<Group> {
    buckets: Vec<TextureResidencyBucket<Group>>,
}

/// Static layer-budget configuration for one storage group.
///
/// `initial_layers` is used by LRU mode. In preload mode it acts as the fallback when
/// no plan is available, while `max_layers` is used as a hard safety check.
#[derive(Clone, Copy, Debug)]
pub struct TextureResidencyGroupLayers<Group> {
    pub group: Group,
    pub debug_name: &'static str,
    pub initial_layers: u32,
    pub max_layers: u32,
}

/// Resolved layer count for a group after the residency strategy is applied.
#[derive(Clone, Copy, Debug)]
pub struct TextureResidencyLayerAllocation<Group> {
    pub group: Group,
    pub layers: u32,
}

impl<Group: Copy + PartialEq> TextureResidencyPlan<Group> {
    /// Creates an empty plan. Groups are added lazily on first insertion.
    pub fn new() -> Self {
        Self {
            buckets: Vec::new(),
        }
    }

    /// Appends a texture ID to the bucket for `group`, creating that bucket if needed.
    pub fn push(&mut self, group: Group, texture_id: u16) {
        if let Some(bucket) = self.buckets.iter_mut().find(|bucket| bucket.group == group) {
            bucket.texture_ids.push(texture_id);
            return;
        }

        self.buckets.push(TextureResidencyBucket {
            group,
            texture_ids: vec![texture_id],
        });
    }

    /// Returns the texture IDs for a group in deterministic insertion order.
    pub fn ids_for_group(&self, group: Group) -> &[u16] {
        self.buckets
            .iter()
            .find(|bucket| bucket.group == group)
            .map(|bucket| bucket.texture_ids.as_slice())
            .unwrap_or(&[])
    }

    /// Returns the layer count needed to store all IDs in the group plus layer `0`.
    ///
    /// Layer `0` is conventionally reserved for a permanent fallback tile in the current
    /// texture-array implementations, so preloaded IDs start at layer `1`.
    pub fn required_layers(&self, group: Group) -> u32 {
        self.ids_for_group(group).len() as u32 + 1
    }

    /// Returns the total number of IDs across all groups.
    pub fn total_texture_count(&self) -> usize {
        self.buckets.iter().map(|bucket| bucket.texture_ids.len()).sum()
    }
}

/// Resolves the runtime layer budget for each group.
///
/// In LRU mode, the configured initial layer counts are preserved. In preload mode,
/// the counts are expanded to exactly fit the provided plan. Every resolved count is
/// asserted against its group-specific maximum so collections fail fast instead of
/// silently overrunning their GPU allocation contract.
pub fn resolve_layer_allocations<Group: Copy + PartialEq>(
    strategy: TextureResidencyStrategy,
    plan: Option<&TextureResidencyPlan<Group>>,
    group_layers: &[TextureResidencyGroupLayers<Group>],
) -> Vec<TextureResidencyLayerAllocation<Group>> {
    group_layers
        .iter()
        .map(|group_layer| {
            let layers = if strategy.preloads_full_collection() {
                plan.map(|plan| plan.required_layers(group_layer.group))
                    .unwrap_or(group_layer.initial_layers)
            } else {
                group_layer.initial_layers
            };

            assert!(
                layers <= group_layer.max_layers,
                "Preloaded {} layer budget exceeded: required {}, max {}",
                group_layer.debug_name,
                layers,
                group_layer.max_layers
            );

            TextureResidencyLayerAllocation {
                group: group_layer.group,
                layers,
            }
        })
        .collect()
}

/// Visits grouped texture IDs in a caller-defined deterministic group order.
///
/// This is used by preload flows so both bookkeeping and upload scheduling can rely on
/// a stable ordering even if the plan was constructed incrementally.
pub fn visit_grouped_texture_ids<Group: Copy + PartialEq>(
    plan: &TextureResidencyPlan<Group>,
    ordered_groups: &[Group],
    mut visitor: impl FnMut(Group, &[u16]),
) {
    for &group in ordered_groups {
        let texture_ids = plan.ids_for_group(group);
        if texture_ids.is_empty() {
            continue;
        }

        visitor(group, texture_ids);
    }
}

/// Visits `(group, texture_id, layer)` tuples for a deterministic preload layout.
///
/// `first_layer` is usually `1`, keeping layer `0` reserved as the permanent fallback.
pub fn visit_grouped_layer_assignments<Group: Copy + PartialEq>(
    plan: &TextureResidencyPlan<Group>,
    ordered_groups: &[Group],
    first_layer: u32,
    mut visitor: impl FnMut(Group, u16, u32),
) {
    visit_grouped_texture_ids(plan, ordered_groups, |group, texture_ids| {
        for (index, &texture_id) in texture_ids.iter().enumerate() {
            visitor(group, texture_id, first_layer + index as u32);
        }
    });
}
