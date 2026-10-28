//! Read-only family/stage census, keeping the source blocks separate.
use std::collections::BTreeMap;
use uocf::enhanced::tileart::ArtData;

#[derive(Default)]
pub(super) struct ItemCensus {
    items: usize,
    // Counts are per material block, not texture-reference rows.
    families: BTreeMap<(u8, Option<String>), (usize, usize)>,
    stages: BTreeMap<(u8, Option<String>, u32), usize>,
}

impl ItemCensus {
    pub(super) fn record(&mut self, art: &ArtData) {
        self.items += 1;
        for material in &art.materials {
            let refs: Vec<_> = art.texture_items.iter().flatten()
                .filter(|r| r.block_index == material.block_index).collect();
            let counts = self.families.entry((material.block_index, material.shader_name.clone())).or_default();
            counts.0 += 1;
            counts.1 += usize::from(!refs.is_empty());
            for reference in refs {
                *self.stages.entry((material.block_index, material.shader_name.clone(), reference.sampler_index)).or_default() += 1;
            }
        }
    }

    pub(super) fn print(&self) {
        println!("items={}", self.items);
        for ((block, shader), (owners, with_refs)) in &self.families {
            println!("block={block} shader={shader:?} owners={owners} with_refs={with_refs}");
            for ((stage_block, stage_shader, stage), count) in &self.stages {
                if (stage_block, stage_shader) == (block, shader) {
                    println!("  sampler={stage} refs={count}");
                }
            }
        }
    }
}
