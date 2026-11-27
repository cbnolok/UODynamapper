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
        // One pass, the frame past the border on the canvas, all of whose output's top left goes.
        assert_eq!(frame.passes, vec![UpscalePass {
            origin: IVec2::splat(UPSCALE_BORDER as i32), from: UVec2::ZERO, to: UVec2::ZERO, size: UVec2::splat(88),
        }]);
    }
    // Made, it shows the frames grown by the border, at 2 texels a pixel.
    let id = job.id;
    upscales.finished.lock().push(id);
    upscales.schedule(&mut atlas, |_, _| Some(&clip), true, NOW + 0.1);
    let parts = upscales.ready(&key).expect("made");
    assert_eq!(parts.len(), 1);
    let ready = &parts[0];
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

/// Checks the passes of `frame` cover its target once, each within its workspace's output.
fn check_passes(frame: &UpscaleFrame, canvas: UVec2) {
    let target = UVec2::new(frame.target.width, frame.target.height);
    let mut covered = vec![0u8; target.element_product() as usize];
    for pass in &frame.passes {
        assert!((pass.from + pass.size).cmple(canvas * UPSCALE_OUTPUT_SCALE).all(), "{pass:?}");
        assert!((pass.to + pass.size).cmple(target).all(), "{pass:?}");
        for y in pass.to.y..pass.to.y + pass.size.y {
            for x in pass.to.x..pass.to.x + pass.size.x {
                covered[(y * target.x + x) as usize] += 1;
            }
        }
    }
    assert!(covered.iter().all(|&count| count == 1), "{:?}", frame.passes);
}

#[test]
fn a_sequence_too_large_for_one_canvas_upscales_in_tiles() {
    // 300 pixels: a 304 by 304 canvas, past 260 by 260; on the 256 by 256 tiles, 2 by 2 passes a frame.
    assert_eq!(UPSCALE_TILE_SIDE, 256);
    let huge = clip(2, 300, 1);
    let key = upscale_key(clip_key(0), &huge, 1);
    let mut upscales = MobileUpscales::default();
    let mut atlas = atlas();
    upscales.want(key, clip_key(0), near(0.0), NOW);
    upscales.schedule(&mut atlas, |_, _| Some(&huge), true, NOW);
    let job = &upscales.jobs[0];
    assert_eq!(job.canvas, UVec2::splat(UPSCALE_TILE_SIDE));
    assert_eq!(job.frames.len(), 2);
    for frame in &job.frames {
        assert_eq!((frame.target.width, frame.target.height), (608, 608));
        assert_eq!(frame.passes.len(), 4);
        check_passes(frame, job.canvas);
    }
}

/// A clip of frames 100 high of (width, anchor from the left edge), on page 0.
fn wide_clip(frames: &[(u32, f32)]) -> Clip {
    let mut clip = clip(frames.len(), 100, 1);
    for (frame, &(width, anchor_x)) in clip.frames.iter_mut().zip(frames) {
        frame.rect.width = width;
        frame.placement.width = width as f32;
        frame.placement.anchor_x = anchor_x;
    }
    clip
}

#[test]
fn a_sequence_wider_than_a_page_at_2x_is_stored_and_drawn_in_parts() {
    // 520 pixels: a 524 wide canvas, 1048 texels at 2x, past the 1023 an RGBA8 page holds; it splits at its anchor.
    // The second frame, 100 wide, ends left of its anchor: all in the first part.
    let wide = wide_clip(&[(520, 260.0), (100, 110.0)]);
    let key = upscale_key(clip_key(0), &wide, 1);
    let mut upscales = MobileUpscales::default();
    let mut atlas = atlas();
    upscales.want(key, clip_key(0), near(0.0), NOW);
    upscales.schedule(&mut atlas, |_, _| Some(&wide), true, NOW);
    let job = &upscales.jobs[0];
    // Frame 0 in two parts, frame 1 in the first only.
    assert_eq!(job.frames.len(), 3);
    for frame in &job.frames {
        assert!(frame.target.width <= AtlasFormat::Rgba8.page_room(), "{:?}", frame.target);
        check_passes(frame, job.canvas);
    }
    upscales.finished.lock().push(job.id);
    upscales.schedule(&mut atlas, |_, _| Some(&wide), true, NOW + 0.1);
    let parts = upscales.ready(&key).expect("made");
    assert_eq!(parts.len(), 2);
    for (index, frame) in wide.frames.iter().enumerate() {
        let [left, right] = [0, 1].map(|part| parts[part].frames[index].placement);
        // The parts meet, and together are the frame's canvas.
        assert_eq!(left.width - left.anchor_x, -right.anchor_x, "frame {index}");
        assert_eq!(left.width + right.width, (frame.rect.width + 2 * UPSCALE_BORDER) as f32, "frame {index}");
        assert_eq!(left.anchor_x, frame.placement.anchor_x + UPSCALE_BORDER as f32);
    }
    // The first frame splits at its anchor; the second's second part is empty.
    assert_eq!(parts[1].frames[0].placement.anchor_x, 0.0);
    assert_eq!(parts[1].frames[1].placement.width, 0.0);
}

#[test]
fn a_frame_taller_than_a_page_at_2x_plays_at_1x_and_nothing_runs_while_off() {
    let mut tall = clip(2, 100, 1);
    tall.frames.iter_mut().for_each(|frame| frame.rect.height = 520);
    let key = upscale_key(clip_key(0), &tall, 1);
    let mut upscales = MobileUpscales::default();
    let mut atlas = atlas();
    upscales.want(key, clip_key(0), near(0.0), NOW);
    upscales.schedule(&mut atlas, |_, _| Some(&tall), true, NOW);
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

/// A clip of frames of `sizes` (width, height), anchored at their bottom middle.
fn extents(sizes: &[(u16, u16)]) -> MobileClip<MobileClipFrameExtent> {
    MobileClip {
        frames: sizes.iter().map(|&(width, height)| MobileClipFrameExtent {
            width, height, anchor_x: i32::from(width / 2), anchor_y: i32::from(height),
        }).collect(),
        mirrored: false,
        fps: 0.0,
        stored: 0,
    }
}

/// A side a canvas fits with room, and one it does not.
const FITTING_SIDE: u16 = 200;
const TOO_LARGE_SIDE: u16 = 300;

/// Whether a body of `clips` upscales, without in-betweens and in a RIFE mode.
fn body_fits(clips: Vec<MobileClip<MobileClipFrameExtent>>) -> [bool; 2] {
    let canvases = body_canvases(clips);
    [false, true].map(|rife| canvas_fits(canvases.of(rife)))
}

fn square_fits(side: u16) -> bool {
    canvas_fits(canvas_for(UVec2::splat(u32::from(side))))
}

#[test]
fn a_body_upscales_whole_or_not_at_all() {
    const BOTH: [bool; 2] = [true; 2];
    const NEITHER: [bool; 2] = [false; 2];
    assert!(square_fits(FITTING_SIDE) && !square_fits(TOO_LARGE_SIDE));
    let walk = extents(&[(FITTING_SIDE, FITTING_SIDE); 3]);
    assert_eq!(body_fits(vec![walk.clone(), walk.clone()]), BOTH);
    // Its walk fits, but a frame of its attack does not: none of its clips upscales.
    let attack = extents(&[(FITTING_SIDE, FITTING_SIDE), (TOO_LARGE_SIDE, TOO_LARGE_SIDE)]);
    assert_eq!(body_fits(vec![walk, attack]), NEITHER);
    // A wide clip and a tall one each fit a canvas of their own: the body upscales, though no one canvas holds both.
    let (wide, tall) = ((TOO_LARGE_SIDE, FITTING_SIDE / 2), (FITTING_SIDE / 2, TOO_LARGE_SIDE));
    assert_eq!(body_fits(vec![extents(&[wide]), extents(&[tall])]), BOTH);
    assert_eq!(body_fits(vec![extents(&[wide, tall])]), NEITHER);
    // A body without frames has nothing too large.
    assert_eq!(body_fits(Vec::new()), BOTH);
}

#[test]
fn off_upscales_none_always_all_but_never_and_balanced_what_the_file_picks() {
    use spec::{BodyUpscale, MobileUpscaleSpec};
    use MobileUpscale::{Always, Balanced, Off};
    let never_asked = || -> bool { panic!("the size rule is asked only in Balanced, for a body not listed") };
    for listed in [None, Some(BodyUpscale::Always), Some(BodyUpscale::Never)] {
        assert!(!body_choice(Off, listed, never_asked));
    }
    assert!(body_choice(Always, None, never_asked));
    assert!(body_choice(Always, Some(BodyUpscale::Always), never_asked));
    assert!(!body_choice(Always, Some(BodyUpscale::Never), never_asked));
    assert!(body_choice(Balanced, Some(BodyUpscale::Always), never_asked));
    assert!(!body_choice(Balanced, Some(BodyUpscale::Never), never_asked));
    assert!(body_choice(Balanced, None, || true) && !body_choice(Balanced, None, || false));

    // Through the resource: a body without frames fits the size rule, unless the file keeps it at 1x.
    let spec = MobileUpscaleSpec::decode("Upscale.kdl", "body 400 package=\"cc\" upscale=\"never\"").expect("spec");
    let mut upscales = MobileUpscales::new(spec);
    let packages = AnimPackages::default();
    let classic = clip_key(0);
    let enhanced = ClipKey { source: AnimSource::Enhanced, ..classic };
    assert!(!upscales.body_upscales(classic, &packages, false, Balanced));
    assert!(!upscales.body_upscales(classic, &packages, false, Always));
    assert!(upscales.body_upscales(enhanced, &packages, false, Balanced));
    assert!(upscales.body_upscales(enhanced, &packages, false, Always));
    assert!(!upscales.body_upscales(enhanced, &packages, false, Off));
}

#[test]
fn a_body_whose_rife_cards_are_too_large_upscales_only_without_in_betweens() {
    // Each frame fits, but the two hang far apart from one anchor: the card holding both, the RIFE in-between's, is
    // twice as wide, and the clip's RIFE sequence needs its canvas. The crossfade draws the two frames apart.
    let mut clip = extents(&[(FITTING_SIDE, FITTING_SIDE); 2]);
    clip.frames[0].anchor_x = 0;
    clip.frames[1].anchor_x = i32::from(FITTING_SIDE);
    let side = u32::from(FITTING_SIDE);
    assert_eq!(body_canvases(vec![clip.clone()]), BodyCanvases {
        clips: canvas_for(UVec2::splat(side)),
        rife: canvas_for(UVec2::new(2 * side, side)),
    });
    assert_eq!(body_fits(vec![clip.clone()]), [true, false]);
    // On one anchor, the card is the frame.
    clip.frames[1].anchor_x = 0;
    assert_eq!(body_fits(vec![clip]), [true, true]);
}

/// The 2x frames a package would hold of `frames` frames: canvases of `canvas` texels a side.
fn packed(frames: usize, canvas: u32) -> PackedFrames {
    let pixels = Arc::<[u8]>::from(vec![0; AtlasFormat::Rgba8.frame_bytes(canvas, canvas)]);
    PackedFrames::Read((0..frames).map(|_| (canvas, canvas, pixels.clone())).collect())
}

/// A 40-pixel frame's 2x canvas: 44 with the border, at 2x.
const PACKED_CANVAS: u32 = 88;

#[test]
fn a_clip_the_package_holds_at_2x_is_ready_without_a_job() {
    let clip = clip(3, 40, 1);
    let key = upscale_key(clip_key(2), &clip, 1);
    let mut upscales = MobileUpscales::default();
    let mut atlas = atlas();
    upscales.want(key, clip_key(2), near(0.0), NOW);
    upscales.entries.get_mut(&key).unwrap().packed = packed(3, PACKED_CANVAS);
    upscales.schedule(&mut atlas, |_, _| Some(&clip), true, NOW);
    assert!(upscales.jobs.is_empty());
    let parts = upscales.ready(&key).expect("ready from the package");
    // Placed as the run time's: the frames grown by the border, at 2 texels a pixel.
    assert_eq!(parts[0].texel_scale, UPSCALE_OUTPUT_SCALE);
    assert_eq!((parts[0].frames[0].placement.width, parts[0].frames[0].placement.anchor_x), (44.0, 22.0));
    assert!(parts[0].frames.iter().all(|frame| frame.rect.layer >= MOBILE_ATLAS_PAGES));
}

#[test]
fn a_rife_sequence_upscales_its_in_betweens_alone_beside_the_packed_frames() {
    // The sequence of a clip of 2 frames at RIFE x2: frame, in-between, frame, in-between.
    const FACTOR: u32 = 2;
    let sequence = clip(4, 40, 1);
    let key = InBetweenKey { frames: 2, ..upscale_key(clip_key(2), &sequence, FACTOR) };
    let mut upscales = MobileUpscales::default();
    let mut atlas = atlas();
    upscales.want(key, clip_key(2), near(0.0), NOW);
    upscales.entries.get_mut(&key).unwrap().packed = packed(2, PACKED_CANVAS);
    upscales.schedule(&mut atlas, |_, _| Some(&sequence), true, NOW);
    assert_eq!(upscales.jobs.len(), 1);
    let sources = upscales.jobs[0].frames.iter().map(|frame| frame.source).collect::<Vec<_>>();
    assert_eq!(sources, vec![sequence.frames[1].rect, sequence.frames[3].rect]);
    let id = upscales.jobs[0].id;
    upscales.finished.lock().push(id);
    upscales.schedule(&mut atlas, |_, _| Some(&sequence), true, NOW + 0.1);
    assert_eq!(upscales.ready(&key).expect("made")[0].frames.len(), 4);
}

#[test]
fn packed_frames_that_are_not_the_canvases_are_left_to_the_run_time() {
    let clip = clip(3, 40, 1);
    let key = upscale_key(clip_key(2), &clip, 1);
    for wrong in [packed(2, PACKED_CANVAS), packed(3, PACKED_CANVAS - 4)] {
        let mut upscales = MobileUpscales::default();
        let mut atlas = atlas();
        upscales.want(key, clip_key(2), near(0.0), NOW);
        upscales.entries.get_mut(&key).unwrap().packed = wrong;
        upscales.schedule(&mut atlas, |_, _| Some(&clip), true, NOW);
        assert_eq!(upscales.jobs.len(), 1);
        assert_eq!(upscales.jobs[0].frames.len(), 3);
    }
}
