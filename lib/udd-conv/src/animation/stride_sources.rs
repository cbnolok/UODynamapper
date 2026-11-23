//! The sources of a mobile animation package's stride clips (`udd_assets::animation::mobile_strides`,
//! `StrideSourceRecord`): each clip's hash in the package next to the hash of the client frames it was made from,
//! decoded as the client's files hold them, before the packer filters, trims, scales or encodes them. A repack
//! carries a stride over to new frames of the same source (`carry_strides`), since none of that moves a foot.
//!
//! The clips' frames are followed back to the client's files through the package's records
//! (`MobileAnimClips::clip_sources`), so the sources of a package can be recorded after it was packed, from the client
//! files it was packed from (`udd-cli tool strides sources`).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use color_eyre::eyre::{self, WrapErr};
use rayon::prelude::*;
use udd_assets::animation::mobile_strides::{
    source_frame_digest, stride_clip_hashes, stride_source_hash, ClipFrameSource, MobileAnimClips, MobileClip,
    StrideSource, StrideSourceRecord, STRIDE_FACINGS, STRIDE_FACING_COUNT,
};
use uocf::classic::anim::{AnimFrame, AnimMap};
use uocf::classic::animationframe_cc::AnimationFrameCc;
use uocf::enhanced::animationframe::AnimationFrame;
use uocf::uop_container::package::{LoadMode, UopPackage};

use super::mobile_anim_cc::CLASSIC_ANIMATIONFRAME_FILES;
use super::mobile_anim_ec::ec_source_frames;
use super::mobile_strides::hash_threads;

/// The source of every clip of `package` a stride is measured for that has frames, its frames read from the client
/// files in `client_dir` (a Classic client's for a Classic package, an Enhanced one's for an Enhanced package), on all
/// threads. Fails when the package's records name frames those files do not have.
pub fn stride_sources<P: MobileAnimClips + Sync + ?Sized>(package: &P, client_dir: &Path)
    -> eyre::Result<Vec<StrideSourceRecord>> {
    let clips = package.stride_clips();
    let hashes = stride_clip_hashes(package, &clips, hash_threads(), |_, _, _| {});
    let mut sides = Vec::new();
    let mut wanted = BTreeSet::new();
    for (&(body, action), hash) in clips.iter().zip(hashes) {
        let Some(clip_hash) = hash.wrap_err_with(|| format!("body {body} action {action}"))? else { continue };
        let side: [Option<MobileClip<ClipFrameSource>>; STRIDE_FACING_COUNT] =
            std::array::from_fn(|index| package.clip_sources(body, action, STRIDE_FACINGS[index]));
        wanted.extend(side.iter().flatten().flat_map(|clip| clip.frames.iter().copied()));
        sides.push((body, action, clip_hash, side));
    }
    let digests = match package.stride_source() {
        StrideSource::Classic => classic_frame_digests(&wanted, client_dir)?,
        StrideSource::Enhanced => enhanced_frame_digests(&wanted, client_dir)?,
    };
    sides.into_iter().map(|(body, action, clip_hash, side)| {
        let digested = side.map(|clip| clip.map(|clip| MobileClip {
            frames: clip.frames.iter().map(|source| digests[source]).collect(),
            mirrored: clip.mirrored, fps: clip.fps, stored: clip.stored,
        }));
        let source_hash = stride_source_hash(&digested)
            .ok_or_else(|| eyre::eyre!("body {body} action {action} has frames but no frame sources"))?;
        Ok(StrideSourceRecord { body_id: body, action_id: action, clip_hash, source_hash })
    }).collect()
}

/// A Classic animation: of an `anim*.mul` file, or of an `AnimationFrame*.uop` entry's stored direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ClassicAnimation {
    Mul { file_index: u8, source_index: u32 },
    Uop { file_index: u8, body: u16, action: u16, direction: u8 },
}

/// The [`source_frame_digest`] of each of the Classic frames `wanted`, decoded from `client_dir` as the packer
/// decodes them before its filter.
fn classic_frame_digests(wanted: &BTreeSet<ClipFrameSource>, client_dir: &Path)
    -> eyre::Result<HashMap<ClipFrameSource, u64>> {
    let mut animations: BTreeMap<ClassicAnimation, Vec<(u16, ClipFrameSource)>> = BTreeMap::new();
    for &source in wanted {
        let (animation, frame_index) = match source {
            ClipFrameSource::ClassicMul { file_index, source_index, frame_index } => {
                (ClassicAnimation::Mul { file_index, source_index }, frame_index)
            }
            ClipFrameSource::ClassicUop { file_index, body, action, direction, frame_index } => {
                (ClassicAnimation::Uop { file_index, body, action, direction }, frame_index)
            }
            ClipFrameSource::Enhanced { .. } => eyre::bail!("a Classic package names an Enhanced frame: {source:?}"),
        };
        animations.entry(animation).or_default().push((frame_index, source));
    }
    let anim_map = animations.keys().any(|animation| matches!(animation, ClassicAnimation::Mul { .. }))
        .then(|| AnimMap::load(client_dir).wrap_err_with(|| format!("load the animations of {}", client_dir.display())))
        .transpose()?;
    let mut uop_packages = HashMap::new();
    for animation in animations.keys() {
        if let ClassicAnimation::Uop { file_index, .. } = *animation {
            if let std::collections::hash_map::Entry::Vacant(entry) = uop_packages.entry(file_index) {
                let path = CLASSIC_ANIMATIONFRAME_FILES.get(usize::from(file_index))
                    .map(|name| client_dir.join(name))
                    .ok_or_else(|| eyre::eyre!("no AnimationFrame file has index {file_index}"))?;
                entry.insert(UopPackage::load_with_mode(&path, LoadMode::Lazy)
                    .wrap_err_with(|| format!("load {}", path.display()))?);
            }
        }
    }
    let animations: Vec<_> = animations.into_iter().collect();
    let digests = animations.par_iter().map(|(animation, frames)| -> eyre::Result<Vec<(ClipFrameSource, u64)>> {
        let decoded: Vec<AnimFrame> = match *animation {
            ClassicAnimation::Mul { file_index, source_index } => anim_map.as_ref().expect("loaded above")
                .decode_animation_index(file_index, source_index),
            ClassicAnimation::Uop { file_index, body, action, direction } => {
                let package = &uop_packages[&file_index];
                let data = package.unpack_file_by_hash(AnimationFrameCc::animationframe_hash(u32::from(body), action))?
                    .ok_or_else(|| eyre::eyre!("no entry {}", AnimationFrameCc::animationframe_path(u32::from(body),
                        action)))?;
                AnimationFrameCc::decode_direction(&data, direction)
            }
        }.wrap_err_with(|| format!("decode the Classic animation {animation:?} of {}", client_dir.display()))?;
        frames.iter().map(|&(frame_index, source)| {
            let frame = decoded.get(usize::from(frame_index)).ok_or_else(|| eyre::eyre!("the Classic animation \
                {animation:?} has {} frames, not frame {frame_index}: the package was not packed from {}",
                decoded.len(), client_dir.display()))?;
            Ok((source, source_frame_digest(frame.width, frame.height, i32::from(frame.center_x),
                i32::from(frame.center_y), &frame.data)))
        }).collect()
    }).collect::<eyre::Result<Vec<_>>>()?;
    Ok(digests.into_iter().flatten().collect())
}

/// The [`source_frame_digest`] of each of the Enhanced frames `wanted`, decoded from `client_dir` as the packer
/// decodes them before its filter.
fn enhanced_frame_digests(wanted: &BTreeSet<ClipFrameSource>, client_dir: &Path)
    -> eyre::Result<HashMap<ClipFrameSource, u64>> {
    let frames = ec_source_frames(client_dir)?;
    let mut entries: BTreeMap<(Arc<PathBuf>, u64), Vec<(u16, ClipFrameSource)>> = BTreeMap::new();
    for &source in wanted {
        let ClipFrameSource::Enhanced { body, source_frame_index, action, frame_number } = source else {
            eyre::bail!("an Enhanced package names a Classic frame: {source:?}");
        };
        let frame = frames.get(&(body, source_frame_index))
            .filter(|frame| (frame.action, frame.frame_number) == (action, frame_number))
            .ok_or_else(|| eyre::eyre!("{} has no frame {frame_number} of action {action} at {source_frame_index} \
                among body {body}'s: the package was not packed from it", client_dir.display()))?;
        entries.entry((Arc::clone(&frame.path), frame.file_hash)).or_default().push((frame.entry_index, source));
    }
    drop(frames);
    let mut packages = HashMap::new();
    for (path, _) in entries.keys() {
        if !packages.contains_key(path) {
            packages.insert(Arc::clone(path), UopPackage::load_with_mode(path.as_ref(), LoadMode::Lazy)
                .wrap_err_with(|| format!("load {}", path.display()))?);
        }
    }
    let entries: Vec<_> = entries.into_iter().collect();
    let digests = entries.par_iter().map(|((path, file_hash), frames)| -> eyre::Result<Vec<(ClipFrameSource, u64)>> {
        let label = || format!("AnimationFrame entry {file_hash:016x} of {}", path.display());
        let data = packages[path].unpack_file_by_hash(*file_hash)?.ok_or_else(|| eyre::eyre!("no {}", label()))?;
        let animation = AnimationFrame::load(&data).wrap_err_with(label)?;
        frames.iter().map(|&(entry_index, source)| {
            let entry = animation.frames.get(usize::from(entry_index))
                .ok_or_else(|| eyre::eyre!("{} has no frame entry {entry_index}", label()))?;
            let frame = animation.decode_frame(entry).wrap_err_with(label)?;
            Ok((source, source_frame_digest(frame.width, frame.height, i32::from(frame.center_x),
                i32::from(frame.center_y), &frame.data)))
        }).collect()
    }).collect::<eyre::Result<Vec<_>>>()?;
    Ok(digests.into_iter().flatten().collect())
}
