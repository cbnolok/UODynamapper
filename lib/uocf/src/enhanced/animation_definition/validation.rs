//! Checks of an animation definition against what the client and the entry layout accept, and of its frame
//! effects with the effect definition checks.

use std::fmt;

use super::{AnimationDefinition, FRAME_EFFECT_COUNT_MAX};
use crate::enhanced::effect_definition::{
    validate_effect_definitions, EffectIssue, EffectIssueSeverity, EffectValidationContext,
};

/// How bad an issue is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AnimationIssueSeverity {
    /// The client plays it, but likely not as intended.
    Warning,
    /// The client rejects the entry, or the layout cannot hold it: the writer refuses it.
    Error,
}

/// One finding of [`AnimationDefinition::validate`].
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationDefinitionIssue {
    /// The item whose entry it is; `None` when validating a bare definition.
    pub item_id: Option<u32>,
    pub kind: AnimationDefinitionIssueKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnimationDefinitionIssueKind {
    /// The start set names no frame set: the client rejects the entry.
    MissingStartSet { key: u32 },
    /// The second set names no frame set: the client rejects the entry.
    MissingSecondSet { key: u32 },
    /// A set's `next` names no frame set.
    MissingNextSet { set: u32, next: u32 },
    /// A set's `next` does not fit the signed file field (`-1` is "no successor").
    NextOutOfRange { set: u32, next: u32 },
    /// A frame interval that is NaN, infinite or negative.
    InvalidFrameInterval { set: u32, interval: f32 },
    /// `value` is negative or NaN: the client rejects the entry (`value >= 0`).
    InvalidValue { value: f32 },
    /// More effects on one frame than its u8 count holds.
    TooManyEffects { set: u32, frame: usize, count: usize },
    /// A frame set without frames shows nothing while it plays.
    EmptyFrameSet { set: u32 },
    /// A set that neither the start set, the second set nor any `next` chain from them reaches never plays.
    UnreachableSet { set: u32 },
    /// An issue of the effects of frame `frame` of set `set`.
    Effect { set: u32, frame: usize, issue: EffectIssue },
}

impl AnimationDefinitionIssue {
    pub fn severity(&self) -> AnimationIssueSeverity {
        match &self.kind {
            AnimationDefinitionIssueKind::MissingStartSet { .. }
            | AnimationDefinitionIssueKind::MissingSecondSet { .. }
            | AnimationDefinitionIssueKind::MissingNextSet { .. }
            | AnimationDefinitionIssueKind::NextOutOfRange { .. }
            | AnimationDefinitionIssueKind::InvalidFrameInterval { .. }
            | AnimationDefinitionIssueKind::InvalidValue { .. }
            | AnimationDefinitionIssueKind::TooManyEffects { .. } => AnimationIssueSeverity::Error,
            AnimationDefinitionIssueKind::EmptyFrameSet { .. } | AnimationDefinitionIssueKind::UnreachableSet { .. } => {
                AnimationIssueSeverity::Warning
            }
            AnimationDefinitionIssueKind::Effect { issue, .. } => match issue.severity() {
                EffectIssueSeverity::Warning => AnimationIssueSeverity::Warning,
                EffectIssueSeverity::Error => AnimationIssueSeverity::Error,
            },
        }
    }
}

impl fmt::Display for AnimationDefinitionIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.item_id {
            Some(item_id) => write!(f, "item {item_id}: ")?,
            None => f.write_str("animation definition: ")?,
        }
        match &self.kind {
            AnimationDefinitionIssueKind::MissingStartSet { key } => {
                write!(f, "start frame set {key} does not exist: the client rejects the entry")
            }
            AnimationDefinitionIssueKind::MissingSecondSet { key } => {
                write!(f, "second frame set {key} does not exist: the client rejects the entry")
            }
            AnimationDefinitionIssueKind::MissingNextSet { set, next } => {
                write!(f, "frame set {set}: next {next} is not a frame set")
            }
            AnimationDefinitionIssueKind::NextOutOfRange { set, next } => {
                write!(f, "frame set {set}: next {next} exceeds i32 (the file field is signed)")
            }
            AnimationDefinitionIssueKind::InvalidFrameInterval { set, interval } => {
                write!(f, "frame set {set}: frame interval {interval} is not a finite non-negative number")
            }
            AnimationDefinitionIssueKind::InvalidValue { value } => {
                write!(f, "value {value} is negative or NaN: the client rejects the entry")
            }
            AnimationDefinitionIssueKind::TooManyEffects { set, frame, count } => {
                write!(f, "frame set {set}, frame {frame}: {count} effects, at most {FRAME_EFFECT_COUNT_MAX}")
            }
            AnimationDefinitionIssueKind::EmptyFrameSet { set } => write!(f, "frame set {set} has no frames"),
            AnimationDefinitionIssueKind::UnreachableSet { set } => {
                write!(f, "frame set {set} is neither the start nor the second set nor any next: never plays")
            }
            AnimationDefinitionIssueKind::Effect { set, frame, issue } => {
                write!(f, "frame set {set}, frame {frame}, {issue}")
            }
        }
    }
}

impl AnimationDefinition {
    /// Every issue of the definition: its own fields, then each frame's effects checked with `context`.
    pub fn validate(&self, context: &EffectValidationContext<'_>) -> Vec<AnimationDefinitionIssue> {
        let mut issues = self.field_issues();
        for (&set, frame_set) in &self.sets {
            for (frame, definition_frame) in frame_set.frames.iter().enumerate() {
                for issue in validate_effect_definitions(&definition_frame.effects, context) {
                    let kind = AnimationDefinitionIssueKind::Effect { set, frame, issue };
                    issues.push(AnimationDefinitionIssue { item_id: None, kind });
                }
            }
        }
        issues
    }

    /// The issues of the definition's own fields, without the effects.
    pub(super) fn field_issues(&self) -> Vec<AnimationDefinitionIssue> {
        let mut kinds = Vec::new();
        if !self.sets.contains_key(&self.start_set) {
            kinds.push(AnimationDefinitionIssueKind::MissingStartSet { key: self.start_set });
        }
        if !self.sets.contains_key(&self.second_set) {
            kinds.push(AnimationDefinitionIssueKind::MissingSecondSet { key: self.second_set });
        }
        if !(self.value >= 0.0) {
            kinds.push(AnimationDefinitionIssueKind::InvalidValue { value: self.value });
        }
        let reachable = self.reachable_sets();
        for (&set, frame_set) in &self.sets {
            if let Some(next) = frame_set.next {
                if i32::try_from(next).is_err() {
                    kinds.push(AnimationDefinitionIssueKind::NextOutOfRange { set, next });
                } else if !self.sets.contains_key(&next) {
                    kinds.push(AnimationDefinitionIssueKind::MissingNextSet { set, next });
                }
            }
            let interval = frame_set.frame_interval;
            if !interval.is_finite() || interval < 0.0 {
                kinds.push(AnimationDefinitionIssueKind::InvalidFrameInterval { set, interval });
            }
            for (frame, definition_frame) in frame_set.frames.iter().enumerate() {
                let count = definition_frame.effects.len();
                if count > FRAME_EFFECT_COUNT_MAX {
                    kinds.push(AnimationDefinitionIssueKind::TooManyEffects { set, frame, count });
                }
            }
            if frame_set.frames.is_empty() {
                kinds.push(AnimationDefinitionIssueKind::EmptyFrameSet { set });
            }
            if !reachable.contains(&set) {
                kinds.push(AnimationDefinitionIssueKind::UnreachableSet { set });
            }
        }
        kinds.into_iter().map(|kind| AnimationDefinitionIssue { item_id: None, kind }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::{DefinitionFrame, FrameSet};
    use super::*;
    use crate::enhanced::effect_definition::{EffectDefinition, WaitEffect};
    use std::collections::BTreeMap;

    const TILE_ID: u32 = 0x1E5D;

    fn set(next: Option<u32>, frames: Vec<DefinitionFrame>) -> FrameSet {
        FrameSet { next, frame_interval: 0.25, frames }
    }

    fn frame(effects: Vec<EffectDefinition>) -> DefinitionFrame {
        DefinitionFrame { tile_id: TILE_ID, effects }
    }

    fn definition(sets: BTreeMap<u32, FrameSet>) -> AnimationDefinition {
        AnimationDefinition { stored_id: TILE_ID, start_set: 0, second_set: 0, value: 0.0, sets }
    }

    fn kinds(definition: &AnimationDefinition) -> Vec<AnimationDefinitionIssueKind> {
        definition.validate(&EffectValidationContext::default()).into_iter().map(|issue| issue.kind).collect()
    }

    #[test]
    fn a_looping_definition_has_no_issues() {
        let looping = definition(BTreeMap::from([(0, set(Some(0), vec![frame(Vec::new())]))]));
        assert_eq!(kinds(&looping), []);
    }

    #[test]
    fn reports_structural_errors_and_warnings() {
        let mut broken = definition(BTreeMap::from([
            (0, set(Some(5), vec![frame(Vec::new())])),
            (1, set(None, Vec::new())),
        ]));
        broken.second_set = 3;
        broken.sets.get_mut(&0).unwrap().frame_interval = f32::NAN;
        let issues = broken.validate(&EffectValidationContext::default());
        let severities: Vec<_> = issues.iter().map(AnimationDefinitionIssue::severity).collect();
        assert!(matches!(
            issues.iter().map(|issue| &issue.kind).collect::<Vec<_>>()[..],
            [
                AnimationDefinitionIssueKind::MissingSecondSet { key: 3 },
                AnimationDefinitionIssueKind::MissingNextSet { set: 0, next: 5 },
                AnimationDefinitionIssueKind::InvalidFrameInterval { set: 0, .. },
                AnimationDefinitionIssueKind::EmptyFrameSet { set: 1 },
                AnimationDefinitionIssueKind::UnreachableSet { set: 1 },
            ]
        ));
        use AnimationIssueSeverity::{Error, Warning};
        assert_eq!(severities, [Error, Error, Error, Warning, Warning]);
        assert_eq!(issues[1].to_string(), "animation definition: frame set 0: next 5 is not a frame set");
    }

    #[test]
    fn reports_effect_issues_with_their_frame() {
        let wait = EffectDefinition::Wait(WaitEffect { children: Vec::new(), min_delay_ms: 9, max_delay_ms: 1, aligned: false });
        let with_effect = definition(BTreeMap::from([(0, set(Some(0), vec![frame(Vec::new()), frame(vec![wait])]))]));
        let issues = with_effect.validate(&EffectValidationContext::default());
        assert_eq!(issues.len(), 1);
        assert!(matches!(issues[0].kind, AnimationDefinitionIssueKind::Effect { set: 0, frame: 1, .. }));
        assert_eq!(issues[0].severity(), AnimationIssueSeverity::Error);
        assert!(issues[0].to_string().starts_with("animation definition: frame set 0, frame 1, "), "{}", issues[0]);
        // The writer checks the entry's own fields only.
        assert!(with_effect.to_bytes().is_ok());
    }
}
