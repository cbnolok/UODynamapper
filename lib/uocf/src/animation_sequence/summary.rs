//! What an `AnimationSequence` entry plays, for viewers, reports and editors: per-action timing (frames, fps, cycle
//! length), alias state and the frames the action's frame file stores; the move actions of a body as a rider and as a
//! mount, and the timing of both side by side; checked edits of the action records and group moves.
//!
//! Semantics follow `docs/client_research/ghidra-animation/results/uosa_animation.md` (Enhanced Client) and
//! `docs/client_research/ghidra-classic-frames/results/classic_frames.md` (Classic Client, mobtypes 0x10000 bodies):
//! the resolved action gives the frames and fps, the requested action its own frame effects and skipped frames; a
//! mount plays `GetMoveAction` of its own group 2 when its sequence has one, else of group 1, with the rider's
//! movement.

use std::collections::BTreeSet;

use color_eyre::eyre::{bail, ensure, eyre, Result};

use super::{
    AnimationSequence, MoveActions, SequenceAction, FIRST_BASE_FRAME, GROUP_MOUNTED, GROUP_PEACE, MOVE_RUN, MOVE_STAND,
    MOVE_WALK, STORED_DIRECTION_COUNT, UNSET_BYTE_OFFSET, UNSET_SHORT_OFFSET,
};
use crate::enhanced::animationframe::AnimationFrame;

/// Flying switch the summaries resolve actions with: the Classic Client always passes it on (`classic_frames.md`,
/// "UOP bodies"), and it is the Enhanced Client's default `flying_animations` setting.
pub const SUMMARY_FLYING_ENABLED: bool = true;
/// Cycle lengths (seconds) closer than this count as equal in the rider / mount check.
pub const CYCLE_TOLERANCE_SECONDS: f32 = 0.001;
/// Movements of the rider / mount check, in the order the ride code numbers them (`UpdateFrame` 0x00436d40).
pub const RIDE_MOVEMENTS: [u8; 3] = [MOVE_STAND, MOVE_WALK, MOVE_RUN];

/// Name of a `MOVE_*` movement.
pub fn movement_name(movement: u8) -> &'static str {
    match movement {
        MOVE_STAND => "stand",
        MOVE_WALK => "walk",
        MOVE_RUN => "run",
        _ => "other",
    }
}

/// Name of a `GROUP_*` key.
pub fn group_name(group: i8) -> &'static str {
    match group {
        super::GROUP_WAR => "war",
        GROUP_PEACE => "peace",
        GROUP_MOUNTED => "mounted",
        super::GROUP_FLYING => "flying",
        super::GROUP_STATE_4 | super::GROUP_STATE_5 => "state",
        super::GROUP_SEATED => "seated",
        _ => "unknown",
    }
}

/// The frames an `AnimationFrame` entry stores (Enhanced Client, Kingdom Reborn or Classic `animationlegacyframe`:
/// the same container and frame table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredFrames {
    /// Frame records of the entry.
    pub count: u32,
    /// Highest 1-based frame number (numbers can have gaps); 0 without frames.
    pub highest_number: u32,
}

impl StoredFrames {
    /// Reads the frame table of an entry payload.
    pub fn read(payload: &[u8]) -> Result<Self> {
        let metadata = AnimationFrame::load_metadata(payload)?;
        let count = u32::try_from(metadata.frames.len()).map_err(|_| eyre!("too many frames"))?;
        let highest_number = metadata.frames.iter().map(|frame| u32::from(frame.frame_number)).max().unwrap_or(0);
        Ok(Self { count, highest_number })
    }

    /// Frames per direction, when the count splits evenly over the stored directions.
    pub fn per_direction(&self) -> Option<u32> {
        (self.count % STORED_DIRECTION_COUNT == 0).then_some(self.count / STORED_DIRECTION_COUNT)
    }

    /// Whether the entry holds `frame_count` frames in each stored direction, numbered without gaps: what the
    /// clients read for an action of `frame_count` frames (`(direction - 3) * frame_count + frame + 1`).
    pub fn matches(&self, frame_count: u32) -> bool {
        let expected = STORED_DIRECTION_COUNT * frame_count;
        self.count == expected && self.highest_number == expected
    }

    /// "12" when the count splits evenly, else "60 frames / 5".
    pub fn per_direction_text(&self) -> String {
        match self.per_direction() {
            Some(per_direction) => per_direction.to_string(),
            None => format!("{} frames / {STORED_DIRECTION_COUNT}", self.count),
        }
    }
}

/// How an action record gets its frames (`ResolveAction`, one alias level).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasState {
    /// The action has its own frames.
    Plain,
    /// An alias of a plain action.
    Alias(u32),
    /// An alias of an alias: the client resolves it to the lowest plain action instead.
    AliasOfAlias(u32),
    /// An alias of an action the sequence does not have: the client resolves it to the lowest plain action instead.
    MissingTarget(u32),
}

impl AliasState {
    /// The alias target, if any.
    pub fn target(self) -> Option<u32> {
        match self {
            Self::Plain => None,
            Self::Alias(target) | Self::AliasOfAlias(target) | Self::MissingTarget(target) => Some(target),
        }
    }

    /// Whether the client falls back to the lowest plain action instead of the alias target.
    pub fn is_broken(self) -> bool {
        matches!(self, Self::AliasOfAlias(_) | Self::MissingTarget(_))
    }
}

/// What one action of a sequence plays.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActionSummary {
    pub action: u32,
    pub alias: AliasState,
    /// Action whose frames play (`ResolveAction`); `None` when the sequence has no plain action.
    pub resolved: Option<u32>,
    /// Frames per direction that play: the resolved action's frame count.
    pub frame_count: u32,
    /// Frames per second that play (`GetActionFps`: the resolved action's, `MISSING_ACTION_FPS` without one).
    pub fps: f32,
    /// The action's own skipped frames below `frame_count` (the frame set is keyed by the requested action;
    /// probable).
    pub skipped: u32,
    /// Frames of the action's own record with frame effects (not inherited by aliases).
    pub effect_frames: usize,
    /// Effect definitions over those frames.
    pub effects: usize,
    /// Some i8 rider offset of the record is set to a value other than 0 (and `UNSET_BYTE_OFFSET`): the data
    /// stores explicit zeros, which change nothing as the sequence offsets are zero too.
    pub offsets_set: bool,
    /// Some i16 extra offset of the record is set to a value other than 0 (and `UNSET_SHORT_OFFSET`).
    pub extra_offsets_set: bool,
}

impl ActionSummary {
    /// Frames shown per cycle: the frame count less the skipped frames, whose time the player passes over.
    pub fn shown_frames(&self) -> u32 {
        self.frame_count.saturating_sub(self.skipped)
    }

    /// Seconds of one cycle (shown frames / fps); `None` at 0 fps.
    pub fn cycle_seconds(&self) -> Option<f32> {
        (self.fps > 0.0).then(|| self.shown_frames() as f32 / self.fps)
    }
}

/// One movement of a rider on a mount: what each plays.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RideTiming {
    pub movement: u8,
    /// The rider's mounted-group action, when the rider has the group.
    pub rider: Option<ActionSummary>,
    /// The mount's action ([`AnimationSequence::mount_move_action`]).
    pub mount: Option<ActionSummary>,
}

impl RideTiming {
    /// Whether both cycle and their lengths differ by more than `CYCLE_TOLERANCE_SECONDS`.
    pub fn cycles_differ(&self) -> bool {
        match (self.rider.and_then(|rider| rider.cycle_seconds()), self.mount.and_then(|mount| mount.cycle_seconds())) {
            (Some(rider), Some(mount)) => (rider - mount).abs() > CYCLE_TOLERANCE_SECONDS,
            _ => false,
        }
    }
}

/// The rider's mounted actions and the mount's ridden actions, per `RIDE_MOVEMENTS`.
pub fn ride_timings(rider: &AnimationSequence, mount: &AnimationSequence) -> Vec<RideTiming> {
    RIDE_MOVEMENTS
        .iter()
        .map(|&movement| RideTiming {
            movement,
            rider: rider.rider_move_action(movement).and_then(|action| rider.action_summary(action)),
            mount: mount.mount_move_action(movement).and_then(|action| mount.action_summary(action)),
        })
        .collect()
}

impl AnimationSequence {
    /// How `action` gets its frames; `None` when the sequence does not have it.
    pub fn alias_state(&self, action: u32) -> Option<AliasState> {
        let record = self.actions.get(&action)?;
        Some(match record.alias {
            None => AliasState::Plain,
            Some(target) => match self.actions.get(&target) {
                None => AliasState::MissingTarget(target),
                Some(target_record) if target_record.alias.is_some() => AliasState::AliasOfAlias(target),
                Some(_) => AliasState::Alias(target),
            },
        })
    }

    /// What `action` plays, with `SUMMARY_FLYING_ENABLED`; `None` when the sequence does not have it.
    pub fn action_summary(&self, action: u32) -> Option<ActionSummary> {
        let record = self.actions.get(&action)?;
        let resolved = self.resolve_action(action, SUMMARY_FLYING_ENABLED);
        let frame_count = resolved.and_then(|resolved| self.actions.get(&resolved)).map_or(0, |target| target.frame_count);
        let skipped = record.skipped_frames.range(..frame_count).count() as u32;
        Some(ActionSummary {
            action,
            alias: self.alias_state(action)?,
            resolved,
            frame_count,
            fps: self.action_fps(action, SUMMARY_FLYING_ENABLED),
            skipped,
            effect_frames: record.frame_effects.len(),
            effects: record.frame_effects.values().map(Vec::len).sum(),
            offsets_set: record.offsets.iter().flat_map(|&(x, y)| [x, y]).any(|value| value != 0 && value != UNSET_BYTE_OFFSET),
            extra_offsets_set: record
                .extra_offsets
                .iter()
                .flat_map(|&(x, y)| [x, y])
                .any(|value| value != 0 && value != UNSET_SHORT_OFFSET),
        })
    }

    /// [`Self::action_summary`] of every action, in id order.
    pub fn action_summaries(&self) -> Vec<ActionSummary> {
        self.actions.keys().filter_map(|&action| self.action_summary(action)).collect()
    }

    /// Group the client gives this body as a mount: `GROUP_MOUNTED` when its sequence has it, else `GROUP_PEACE`
    /// (`UpdateFrame` 0x00436d40, `HasGroup` 0x0083710b; confirmed).
    pub fn mount_group(&self) -> i8 {
        if self.groups.contains_key(&GROUP_MOUNTED) { GROUP_MOUNTED } else { GROUP_PEACE }
    }

    /// Action this body plays as a mount for the rider's `movement`: `GetMoveAction(mount_group, movement, false)`.
    pub fn mount_move_action(&self, movement: u8) -> Option<u32> {
        self.move_action(self.mount_group(), movement, false)
    }

    /// Action this body plays riding for `movement`: its mounted group's move action; `None` without the group.
    pub fn rider_move_action(&self, movement: u8) -> Option<u32> {
        self.groups.contains_key(&GROUP_MOUNTED).then(|| self.move_action(GROUP_MOUNTED, movement, false)).flatten()
    }

    /// Recomputes `base_frame` after an edit of frame counts or aliases, as the reader numbers them: plain actions
    /// own `STORED_DIRECTION_COUNT * frame_count` consecutive numbers from `FIRST_BASE_FRAME` in id order (the
    /// file order of every client entry and of [`Self::to_bytes_dialect`]); aliases own none.
    pub fn renumber_base_frames(&mut self) {
        let mut next = FIRST_BASE_FRAME;
        for record in self.actions.values_mut() {
            if record.alias.is_none() {
                record.base_frame = next;
                next = next.saturating_add(STORED_DIRECTION_COUNT.saturating_mul(record.frame_count));
            } else {
                record.base_frame = 0;
            }
        }
    }

    fn action_mut(&mut self, action: u32) -> Result<&mut SequenceAction> {
        self.actions.get_mut(&action).ok_or_else(|| eyre!("the sequence has no action {action}"))
    }

    /// Sets the frames per direction of the plain `action`; returns the previous count. Refused for an alias (it
    /// owns no frames) and when a frame effect or skipped frame of the action lies at or past the new count. The
    /// frame file's numbering depends on it: direction `d` holds frames `(d - 3) * frame_count + 1 ..`.
    pub fn set_frame_count(&mut self, action: u32, frame_count: u32) -> Result<u32> {
        let record = self.action_mut(action)?;
        ensure!(record.alias.is_none(), "action {action} is an alias: it owns no frames");
        if let Some(frame) = record.frame_effects.keys().chain(&record.skipped_frames).find(|&&frame| frame >= frame_count) {
            bail!("action {action} has effects or a skipped frame at frame {frame}, past {frame_count} frames");
        }
        let previous = std::mem::replace(&mut record.frame_count, frame_count);
        self.renumber_base_frames();
        Ok(previous)
    }

    /// Sets the fps of `action`'s record; returns the previous value. Refused unless finite and not negative. Only
    /// a plain action's fps plays: an alias plays its target's.
    pub fn set_fps(&mut self, action: u32, fps: f32) -> Result<f32> {
        ensure!(fps.is_finite() && fps >= 0.0, "action {action}: fps {fps} is not a finite value of 0 or more");
        Ok(std::mem::replace(&mut self.action_mut(action)?.fps, fps))
    }

    /// Makes `action` an alias of `alias`, or a plain action with `None`; returns the previous alias. The target
    /// must be another action of the sequence. Frame count and fps are kept as they are (the client ignores them on
    /// an alias); a plain action needs a frame count matching its frame file.
    pub fn set_alias(&mut self, action: u32, alias: Option<u32>) -> Result<Option<u32>> {
        if let Some(target) = alias {
            ensure!(target != action, "action {action} cannot be an alias of itself");
            ensure!(self.actions.contains_key(&target), "the sequence has no action {target} to alias");
        }
        let previous = std::mem::replace(&mut self.action_mut(action)?.alias, alias);
        self.renumber_base_frames();
        Ok(previous)
    }

    /// Sets the skipped frames (0-based frame indices within a direction) of `action`; returns the previous ones.
    /// A plain action's must lie below its frame count. The Kingdom Reborn layout has none.
    pub fn set_skipped_frames(&mut self, action: u32, frames: BTreeSet<u32>) -> Result<BTreeSet<u32>> {
        let record = self.action_mut(action)?;
        if record.alias.is_none() {
            if let Some(frame) = frames.iter().find(|&&frame| frame >= record.frame_count) {
                bail!("action {action}: skipped frame {frame} is past its {} frames", record.frame_count);
            }
        }
        Ok(std::mem::replace(&mut record.skipped_frames, frames))
    }

    /// Sets the `movement` actions (`MOVE_*`) of `group`; returns the previous ones. The group must exist and the
    /// actions be actions of the sequence.
    pub fn set_move_actions(&mut self, group: i8, movement: u8, actions: MoveActions) -> Result<Option<MoveActions>> {
        for action in [actions.action, actions.alternate] {
            ensure!(self.actions.contains_key(&action), "the sequence has no action {action}");
        }
        let lookup = self.groups.get_mut(&group).ok_or_else(|| eyre!("the sequence has no group {group}"))?;
        Ok(lookup.moves.insert(movement, actions))
    }

    /// What an edit left inconsistent, which the writers refuse: an fps that is not a finite value of 0 or more, an
    /// alias of the action itself, a frame effect or skipped frame of a plain action at or past its frame count.
    /// Every client entry has none.
    pub fn edit_errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        for (&id, record) in &self.actions {
            if !(record.fps.is_finite() && record.fps >= 0.0) {
                errors.push(format!("action {id}: fps {} is not a finite value of 0 or more", record.fps));
            }
            if record.alias == Some(id) {
                errors.push(format!("action {id} is an alias of itself"));
            }
            if record.alias.is_none() {
                if let Some(frame) = record.frame_effects.keys().find(|&&frame| frame >= record.frame_count) {
                    errors.push(format!("action {id}: frame effects at frame {frame}, past its {} frames", record.frame_count));
                }
                if let Some(frame) = record.skipped_frames.iter().find(|&&frame| frame >= record.frame_count) {
                    errors.push(format!("action {id}: skipped frame {frame}, past its {} frames", record.frame_count));
                }
            }
        }
        errors
    }
}

/// "-" for `None`.
fn optional(value: Option<impl std::fmt::Display>) -> String {
    value.map_or_else(|| "-".to_string(), |value| value.to_string())
}

/// "(x,y) ..." per direction; `unset` values print as "-".
fn offsets_text<T: PartialEq + std::fmt::Display + Copy>(offsets: &[(T, T)], unset: T) -> String {
    let value = |value: T| if value == unset { "-".to_string() } else { value.to_string() };
    offsets.iter().map(|&(x, y)| format!("({},{})", value(x), value(y))).collect::<Vec<_>>().join(" ")
}

impl AnimationSequence {
    /// Readable dump: header, one line per action (what it plays and its own record), groups with their moves,
    /// transitions and typed actions.
    pub fn dump_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "stored id {}, {} actions, {} groups{}",
            self.stored_id,
            self.actions.len(),
            self.groups.len(),
            if self.repeated_actions.is_empty() { String::new() } else { format!(", {} repeated action records", self.repeated_actions.len()) }
        );
        let _ = writeln!(out, "sequence offsets: {}", offsets_text(&self.offsets, UNSET_BYTE_OFFSET));
        let _ = writeln!(out, "sequence extra offsets: {}", offsets_text(&self.extra_offsets, UNSET_SHORT_OFFSET));
        let _ = writeln!(out, "\naction\tframes\tfps\tcycle_s\talias\tresolved\tskipped\teffects\tbase_frame");
        for summary in self.action_summaries() {
            let record = &self.actions[&summary.action];
            let alias = match summary.alias {
                AliasState::Plain => "-".to_string(),
                AliasState::Alias(target) => target.to_string(),
                AliasState::AliasOfAlias(target) => format!("{target} (alias of alias)"),
                AliasState::MissingTarget(target) => format!("{target} (missing)"),
            };
            let skipped: Vec<String> = record.skipped_frames.iter().map(u32::to_string).collect();
            let _ = writeln!(
                out,
                "{}\t{}\t{}\t{}\t{alias}\t{}\t{}\t{}\t{}",
                summary.action,
                summary.frame_count,
                summary.fps,
                optional(summary.cycle_seconds().map(|seconds| format!("{seconds:.3}"))),
                optional(summary.resolved),
                if skipped.is_empty() { "-".to_string() } else { skipped.join(",") },
                summary.effects,
                record.base_frame
            );
            for (frame, effects) in &record.frame_effects {
                for effect in effects {
                    let _ = writeln!(out, "\t  frame {frame}: {effect:?}");
                }
            }
            if summary.offsets_set {
                let _ = writeln!(out, "\t  rider offsets: {}", offsets_text(&record.offsets, UNSET_BYTE_OFFSET));
            }
            if summary.extra_offsets_set {
                let _ = writeln!(out, "\t  extra offsets: {}", offsets_text(&record.extra_offsets, UNSET_SHORT_OFFSET));
            }
        }
        for (&key, group) in &self.groups {
            let _ = writeln!(
                out,
                "\ngroup {key} ({}): parent {}, default action {}",
                group_name(key),
                optional(group.parent),
                group.default_action
            );
            for (&movement, actions) in &group.moves {
                let alternate =
                    if actions.alternate == actions.action { String::new() } else { format!(" (alternate {})", actions.alternate) };
                let _ = writeln!(out, "  {} {}{alternate}", movement_name(movement), actions.action);
            }
            for (transition, action) in &group.transitions {
                let _ = writeln!(out, "  transition {transition}: {action}");
            }
            for ((kind, sub_type), actions) in &group.typed {
                let actions: Vec<String> = actions.iter().map(u32::to_string).collect();
                let _ = writeln!(out, "  typed {kind}/{sub_type}: [{}]", actions.join(" "));
            }
        }
        out
    }

    /// The entry as JSON: every field of every record, with what each action plays (effects as debug text).
    pub fn dump_json(&self) -> serde_json::Value {
        use serde_json::json;
        let actions: Vec<serde_json::Value> = self
            .action_summaries()
            .into_iter()
            .map(|summary| {
                let record = &self.actions[&summary.action];
                let effects: serde_json::Map<String, serde_json::Value> = record
                    .frame_effects
                    .iter()
                    .map(|(frame, effects)| (frame.to_string(), json!(effects.iter().map(|effect| format!("{effect:?}")).collect::<Vec<_>>())))
                    .collect();
                json!({
                    "action": summary.action,
                    "frame_count": record.frame_count,
                    "fps": record.fps,
                    "alias": record.alias,
                    "alias_state": format!("{:?}", summary.alias),
                    "resolved": summary.resolved,
                    "played_frame_count": summary.frame_count,
                    "played_fps": summary.fps,
                    "cycle_seconds": summary.cycle_seconds(),
                    "skipped_frames": record.skipped_frames,
                    "frame_effects": effects,
                    "offsets": record.offsets,
                    "extra_offsets": record.extra_offsets,
                    "base_frame": record.base_frame,
                    "kr_extra": record.kr_extra,
                })
            })
            .collect();
        let groups: Vec<serde_json::Value> = self
            .groups
            .iter()
            .map(|(&key, group)| {
                json!({
                    "key": key,
                    "name": group_name(key),
                    "parent": group.parent,
                    "default_action": group.default_action,
                    "moves": group.moves.iter().map(|(&movement, actions)| json!({
                        "movement": movement,
                        "name": movement_name(movement),
                        "action": actions.action,
                        "alternate": actions.alternate,
                    })).collect::<Vec<_>>(),
                    "transitions": group.transitions.iter().map(|(key, action)| json!({"key": key, "action": action})).collect::<Vec<_>>(),
                    "typed": group.typed.iter().map(|((kind, sub_type), actions)| json!({
                        "type": kind,
                        "sub_type": sub_type,
                        "actions": actions,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        json!({
            "stored_id": self.stored_id,
            "offsets": self.offsets,
            "extra_offsets": self.extra_offsets,
            "actions": actions,
            "groups": groups,
            "repeated_actions": self.repeated_actions.iter().map(|repeat| json!({"position": repeat.position, "id": repeat.id})).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::parsed;
    use super::*;

    #[test]
    fn summaries_give_resolved_timing_and_alias_state() {
        let sequence = parsed();
        let walk = sequence.action_summary(4).unwrap();
        // Action 4: 8 frames at 12 fps, frame 0 skipped, one effect frame.
        assert_eq!((walk.alias, walk.resolved, walk.frame_count, walk.skipped), (AliasState::Plain, Some(4), 8, 1));
        assert_eq!((walk.effect_frames, walk.effects), (1, 1));
        assert_eq!(walk.cycle_seconds(), Some(7.0 / 12.0));
        // Alias 1 -> 4 plays 4's frames and fps, without its effects or skipped frames.
        let alias = sequence.action_summary(1).unwrap();
        assert_eq!((alias.alias, alias.resolved, alias.frame_count, alias.fps), (AliasState::Alias(4), Some(4), 8, 12.0));
        assert_eq!((alias.skipped, alias.effects), (0, 0));
        assert_eq!(sequence.alias_state(2), Some(AliasState::AliasOfAlias(1)));
        assert_eq!(sequence.alias_state(3), Some(AliasState::MissingTarget(9)));
        assert_eq!(sequence.action_summary(2).unwrap().resolved, Some(0));
        assert!(sequence.action_summary(99).is_none());
        assert_eq!(sequence.action_summaries().len(), sequence.actions.len());
    }

    #[test]
    fn mounts_use_their_mounted_group_when_they_have_it() {
        let mut mount = parsed();
        assert_eq!(mount.mount_group(), GROUP_PEACE);
        assert_eq!(mount.mount_move_action(MOVE_RUN), Some(19));
        assert_eq!(mount.rider_move_action(MOVE_RUN), None);
        let mut rider = parsed();
        let mut mounted = rider.groups[&GROUP_PEACE].clone();
        mounted.moves.insert(MOVE_RUN, MoveActions { action: 61, alternate: 61 });
        rider.groups.insert(GROUP_MOUNTED, mounted);
        assert_eq!(rider.mount_group(), GROUP_MOUNTED);
        assert_eq!(rider.rider_move_action(MOVE_RUN), Some(61));

        // Run: the rider's 61 (6 frames, 31 fps) against the mount's 19 (6 frames, 9 fps).
        let timings = ride_timings(&rider, &mount);
        let run = timings.iter().find(|timing| timing.movement == MOVE_RUN).unwrap();
        assert_eq!((run.rider.unwrap().action, run.mount.unwrap().action), (61, 19));
        assert!(run.cycles_differ());
        mount.set_fps(19, 31.0).unwrap();
        assert!(!ride_timings(&rider, &mount).iter().any(RideTiming::cycles_differ));
    }

    #[test]
    fn edits_check_their_values_and_renumber_frames() {
        let mut sequence = parsed();
        let base_19 = sequence.actions[&19].base_frame;
        assert_eq!(sequence.set_frame_count(4, 9).unwrap(), 8);
        assert_eq!(sequence.actions[&19].base_frame, base_19 + STORED_DIRECTION_COUNT);
        // The effect is at frame 1 and the skipped frame at 0.
        assert!(sequence.set_frame_count(4, 1).is_err());
        assert!(sequence.set_frame_count(1, 3).is_err(), "an alias owns no frames");
        assert!(sequence.set_fps(4, f32::NAN).is_err() && sequence.set_fps(4, -1.0).is_err());
        assert_eq!(sequence.set_fps(4, 10.0).unwrap(), 12.0);
        assert!(sequence.set_alias(4, Some(4)).is_err() && sequence.set_alias(4, Some(77)).is_err());
        assert_eq!(sequence.set_alias(19, Some(0)).unwrap(), None);
        assert_eq!(sequence.actions[&19].base_frame, 0);
        assert_eq!(sequence.actions[&61].base_frame, base_19 + STORED_DIRECTION_COUNT);
        assert!(sequence.set_skipped_frames(0, BTreeSet::from([10])).is_err());
        sequence.set_skipped_frames(0, BTreeSet::from([0, 9])).unwrap();
        assert!(sequence.set_move_actions(GROUP_MOUNTED, MOVE_WALK, MoveActions { action: 0, alternate: 0 }).is_err());
        assert!(sequence.set_move_actions(GROUP_PEACE, MOVE_WALK, MoveActions { action: 0, alternate: 77 }).is_err());
        sequence.set_move_actions(GROUP_PEACE, MOVE_WALK, MoveActions { action: 61, alternate: 61 }).unwrap();
        assert!(sequence.edit_errors().is_empty());

        let reparsed = AnimationSequence::parse(&sequence.to_bytes().unwrap()).unwrap();
        assert_eq!(reparsed, sequence, "the renumbered base frames are the reader's");
        sequence.actions.get_mut(&0).unwrap().frame_count = 5;
        assert_eq!(sequence.edit_errors().len(), 1);
    }

    #[test]
    fn dumps_list_every_action_and_group() {
        let sequence = parsed();
        let text = sequence.dump_text();
        assert!(text.contains("\n4\t8\t12\t0.583\t-\t4\t0\t1\t"), "{text}");
        assert!(text.contains("\n2\t") && text.contains("1 (alias of alias)"), "{text}");
        assert!(text.contains("group 1 (peace)"), "{text}");
        let json = sequence.dump_json();
        assert_eq!(json["actions"].as_array().unwrap().len(), sequence.actions.len());
        assert_eq!(json["groups"].as_array().unwrap().len(), sequence.groups.len());
    }

    #[test]
    fn stored_frames_match_whole_gapless_directions() {
        let whole = StoredFrames { count: 40, highest_number: 40 };
        assert!(whole.matches(8) && !whole.matches(9));
        assert_eq!((whole.per_direction(), whole.per_direction_text()), (Some(8), "8".to_string()));
        let gaps = StoredFrames { count: 38, highest_number: 40 };
        assert!(!gaps.matches(8));
        assert_eq!(gaps.per_direction_text(), format!("38 frames / {STORED_DIRECTION_COUNT}"));
    }
}
