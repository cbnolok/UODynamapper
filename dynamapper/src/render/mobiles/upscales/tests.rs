use super::*;
use super::super::clips::CLASSIC_FRAME_SECS;
use super::super::frame_atlas::{AtlasFormat, MOBILE_ATLAS_PAGES};

const NOW: f64 = 10.0;

fn clip_key(direction: u8) -> ClipKey {
    ClipKey { source: AnimSource::Classic, body: 400, action: 0, direction }
}

/// Frames of `side` square anchored at their bottom middle, side by side on page 0.
fn clip(frames: usize, side: u32, stored: u32) -> Clip {
    Clip {
        frames: (0..frames as u32).map(|index| ClipFrame {
            rect: AtlasRect { format: AtlasFormat::Rgba8, layer: 0, x: index * (side + 1), y: 0, width: side,
                height: side },
            placement: FramePlacement { width: side as f32, height: side as f32, anchor_x: side as f32 / 2.0,
                anchor_y: side as f32 },
        }).collect(),
        mirrored: false,
        frame_secs: CLASSIC_FRAME_SECS,
        stored,
        texel_scale: 1,
    }
}

fn atlas() -> MobileFrameAtlas<AtlasEntry> {
    let mut images = Assets::<Image>::default();
    let mut atlas = MobileFrameAtlas::new(&mut images, [AtlasFormat::Rgba8]);
    atlas.set_upscaled_pages(&mut images, MOBILE_ATLAS_UPSCALED_PAGES);
    atlas
}

fn near(distance: f32) -> UpscalePriority {
    UpscalePriority { distance, area: 1.0 }
}

#[test]
fn the_gate_opens_at_one_and_a_half_and_closes_below_its_hysteresis() {
    let mut gate = MobileUpscaleGate::default();
    let open_at = |gate: &mut MobileUpscaleGate, magnification| {
        gate.update(magnification);
        gate.open
    };
    assert!(!open_at(&mut gate, 1.0));
    assert!(!open_at(&mut gate, UPSCALE_OFF_MAGNIFICATION + 0.05));
    assert!(open_at(&mut gate, UPSCALE_ON_MAGNIFICATION));
    // Between the two, it stays as it was.
    assert!(open_at(&mut gate, UPSCALE_OFF_MAGNIFICATION + 0.05));
    assert!(!open_at(&mut gate, UPSCALE_OFF_MAGNIFICATION - 0.05));
    assert!(!open_at(&mut gate, UPSCALE_OFF_MAGNIFICATION + 0.05));
    // A zoom of 0.5 on a display of scale 1 doubles the art; a HiDPI display at zoom 1 too.
    assert_eq!(view_magnification(1.0, 0.5), 2.0);
    assert_eq!(view_magnification(2.0, 1.0), 2.0);
}

#[test]
fn a_sequence_upscales_into_the_upscaled_pages_with_its_border_at_2x() {
    let clip = clip(3, 40, 1);
    let key = upscale_key(clip_key(2), &clip, 1);
    let mut upscales = MobileUpscales::default();
    let mut atlas = atlas();
    upscales.want(key, clip_key(2), near(0.0), NOW);
    upscales.schedule(&mut atlas, |_, _| Some(&clip), true, NOW);
    assert_eq!(upscales.jobs.len(), 1);
    let job = &upscales.jobs[0];
    // A 44 by 44 canvas, rounded up to 48.
    assert_eq!(job.canvas, UVec2::splat(48));
    assert!(job.snap_alpha);
    for (frame, source) in job.frames.iter().zip(&clip.frames) {
        assert_eq!(frame.source, source.rect);
        assert!(frame.target.layer >= MOBILE_ATLAS_PAGES, "{:?}", frame.target);
        assert_eq!((frame.target.width, frame.target.height), (88, 88));
    }
    // Made, it shows the frames grown by the border, at 2 texels a pixel.
    let id = job.id;
    upscales.finished.lock().push(id);
    upscales.schedule(&mut atlas, |_, _| Some(&clip), true, NOW + 0.1);
    let ready = upscales.ready(&key).expect("made");
    assert_eq!(ready.texel_scale, UPSCALE_OUTPUT_SCALE);
    assert_eq!((ready.frame_secs, ready.frames.len()), (clip.frame_secs, clip.frames.len()));
    let placement = ready.frames[0].placement;
    assert_eq!((placement.width, placement.height), (44.0, 44.0));
    assert_eq!((placement.anchor_x, placement.anchor_y), (22.0, 42.0));
    assert!(upscales.jobs.is_empty());
}

#[test]
fn the_nearest_upscales_first_then_the_largest_on_screen() {
    let (a, b, c) = (clip(2, 20, 1), clip(2, 20, 2), clip(2, 20, 3));
    let clips = [(clip_key(0), &a), (clip_key(1), &b), (clip_key(2), &c)];
    let keys = clips.map(|(key, clip)| upscale_key(key, clip, 1));
    let mut upscales = MobileUpscales::default();
    let mut atlas = atlas();
    upscales.want(keys[0], clip_key(0), UpscalePriority { distance: 5.0, area: 900.0 }, NOW);
    upscales.want(keys[1], clip_key(1), UpscalePriority { distance: 1.2, area: 100.0 }, NOW);
    upscales.want(keys[2], clip_key(2), UpscalePriority { distance: 1.5, area: 400.0 }, NOW);
    let base = |key: &InBetweenKey, _| clips.iter().find(|(_, clip)| clip.stored == key.stored).map(|(_, clip)| *clip);
    upscales.schedule(&mut atlas, base, true, NOW);
    // b and c are a tile from the player, c larger; a is farther.
    assert_eq!(upscales.jobs.len(), 3);
    let stored_order = upscales.jobs.iter().map(|job| {
        let id = job.id;
        upscales.entries.iter().find(|(_, entry)| matches!(entry.state,
            UpscaleState::Generating { job: Some(job), .. } if job == id)).map(|(key, _)| key.stored).unwrap()
    }).collect::<Vec<_>>();
    assert_eq!(stored_order, vec![3, 2, 1]);
}

#[test]
fn upscales_empty_only_upscaled_pages_and_go_with_them() {
    // Each 2x sequence of 4 frames of 250 pixels (508 by 508 at 2x) fills a page: one more than the pool holds.
    let side = 250;
    let clips = (0..=MOBILE_ATLAS_UPSCALED_PAGES).map(|stored| clip(4, side, stored)).collect::<Vec<_>>();
    let mut upscales = MobileUpscales::default();
    let mut atlas = atlas();
    let base_entry = AtlasEntry::Clip(clip_key(0));
    atlas.reserve(base_entry, AtlasFormat::Rgba8, &[(side, side)], NOW - 100.0).expect("room");
    let mut evicted = Vec::new();
    for (index, clip) in clips.iter().enumerate() {
        let now = NOW + index as f64;
        upscales.want(upscale_key(clip_key(0), clip, 1), clip_key(0), near(0.0), now);
        evicted.extend(upscales.schedule(&mut atlas, |key, _| clips.iter().find(|clip| clip.stored == key.stored),
            true, now));
    }
    // The first went for the last; the clip on the base pages, used longest ago, stayed.
    let first = upscale_key(clip_key(0), &clips[0], 1);
    assert_eq!(evicted, vec![AtlasEntry::Upscaled(first)]);
    assert!(!upscales.entries.contains_key(&first));
    assert_eq!(upscales.entries.len(), MOBILE_ATLAS_UPSCALED_PAGES as usize);
}

#[test]
fn a_frame_too_large_plays_at_1x_and_nothing_runs_while_off() {
    let huge = clip(2, 300, 1);
    let key = upscale_key(clip_key(0), &huge, 1);
    let mut upscales = MobileUpscales::default();
    let mut atlas = atlas();
    upscales.want(key, clip_key(0), near(0.0), NOW);
    upscales.schedule(&mut atlas, |_, _| Some(&huge), true, NOW);
    assert!(matches!(upscales.entries[&key].state, UpscaleState::Missing));
    assert!(upscales.jobs.is_empty());

    let small = clip(2, 20, 2);
    let key = upscale_key(clip_key(0), &small, 1);
    upscales.want(key, clip_key(0), near(0.0), NOW);
    upscales.schedule(&mut atlas, |_, _| Some(&small), false, NOW);
    assert!(upscales.jobs.is_empty() && !upscales.active);
    // Its 1x sequence gone, its job waits for it.
    upscales.schedule(&mut atlas, |_, _| Some(&small), true, NOW);
    assert_eq!(upscales.jobs.len(), 1);
    upscales.schedule(&mut atlas, |_, _| None, true, NOW);
    assert!(upscales.jobs.is_empty());
}

#[test]
fn a_job_started_resumes_after_the_sequences_drawn_until_long_undrawn() {
    let (walk, stand) = (clip(4, 20, 1), clip(4, 20, 2));
    let (walk_key, stand_key) = (upscale_key(clip_key(0), &walk, 1), upscale_key(clip_key(0), &stand, 1));
    let base = |key: &InBetweenKey, _| [&walk, &stand].into_iter().find(|clip| clip.stored == key.stored);
    let mut upscales = MobileUpscales::default();
    let mut atlas = atlas();
    upscales.want(walk_key, clip_key(0), near(0.0), NOW);
    upscales.schedule(&mut atlas, base, true, NOW);
    let walk_job = upscales.jobs[0].id;
    // Stopped: the stand's job goes first, the walk's goes on after it.
    let later = NOW + UPSCALE_QUEUE_KEEP_SECS + 1.0;
    upscales.want(stand_key, clip_key(0), near(0.0), later);
    upscales.schedule(&mut atlas, base, true, later);
    assert_eq!(upscales.jobs.len(), 2);
    assert_ne!(upscales.jobs[0].id, walk_job);
    assert_eq!(upscales.jobs[1].id, walk_job);
    // Long undrawn, it stops.
    let much_later = NOW + UPSCALE_RESUME_SECS + 1.0;
    upscales.want(stand_key, clip_key(0), near(0.0), much_later);
    upscales.schedule(&mut atlas, base, true, much_later);
    assert_eq!(upscales.jobs.len(), 1);
    assert_ne!(upscales.jobs[0].id, walk_job);
}

#[test]
fn a_layer_switches_only_as_its_sequence_starts_over() {
    let clip = clip(4, 20, 1);
    let key = upscale_key(clip_key(0), &clip, 1);
    let other = upscale_key(clip_key(1), &clip, 2);
    const TIE: u32 = 8;
    let step = |last: Option<UpscaleSwitch>, key, index, wanted, ready| {
        UpscaleSwitch::next(last.as_ref(), TIE, key, clip.frames.len(), index, wanted, ready)
    };
    // Ready from the first frame drawn: 2x at once.
    assert!(step(None, key, 2, true, true).upscaled);
    // Ready mid-cycle: 1x until the index goes back.
    let mut switch = step(None, key, 0, true, false);
    for index in [1, 2, 3] {
        switch = step(Some(switch), key, index, true, index >= 2);
        assert!(!switch.upscaled, "frame {index}");
    }
    switch = step(Some(switch), key, 0, true, true);
    assert!(switch.upscaled);
    // Zoomed out mid-cycle: 2x until the cycle ends.
    switch = step(Some(switch), key, 2, false, true);
    assert!(switch.upscaled);
    switch = step(Some(switch), key, 1, false, true);
    assert!(!switch.upscaled);
    // The 2x frames gone: 1x at once.
    switch = step(Some(step(Some(switch), key, 0, true, true)), key, 1, true, true);
    assert!(switch.upscaled);
    assert!(!step(Some(switch), key, 2, true, false).upscaled);
    // Another sequence starts as it is.
    assert!(step(Some(switch), other, 3, true, true).upscaled);
    assert!(!step(Some(switch), other, 3, true, false).upscaled);
}
