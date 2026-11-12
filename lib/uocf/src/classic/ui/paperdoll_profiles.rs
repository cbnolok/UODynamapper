//! The gump paperdoll's profiles, `PaperdollProfiles.kdl`: which gumps a body's worn items draw and where. Dynamapper
//! reads the file (its bundled copy in `assets/runtime_specs/paperdoll/`, or the user's) and the uocf editor builds the
//! bundled file in; both decode it here, strictly (`uot_kdl::strict`).

use color_eyre::eyre;
use knuffel::Decode;

/// `profile "<id>" equipment_offset=N body=N [label=] [fallback_equipment_offset=] [canvas_width=] [canvas_height=]
/// [body_x=] [body_y=] [equipment_x=] [equipment_y=]`, decoded by `PaperdollProfilesKdl`.
pub const PAPERDOLL_PROFILES_SCHEMA: &[uot_kdl::strict::NodeSchema] = &[uot_kdl::strict::NodeSchema::leaf(
    "profile",
    &[
        "label",
        "equipment_offset",
        "fallback_equipment_offset",
        "body",
        "canvas_width",
        "canvas_height",
        "body_x",
        "body_y",
        "equipment_x",
        "equipment_y",
    ],
)];

/// A body's gump paperdoll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperdollProfile {
    pub id: String,
    /// The name shown; the id when the file gives none.
    pub label: String,
    /// Added to an item's tiledata anim id: the gump the item draws.
    pub equipment_offset: u32,
    /// The offset of the gump drawn when the one at `equipment_offset` is missing: the female profiles fall back to the
    /// male gumps, as ClassicUO and the Enhanced Client's gump paperdoll do. `None`: no fallback.
    pub fallback_equipment_offset: Option<u32>,
    /// The server body: Dynamapper's 3D paperdoll draws its frame, and its race picks the overrides of the draw order.
    pub body: u32,
    /// The minimum size of the composed paperdoll.
    pub canvas_width: u32,
    pub canvas_height: u32,
    /// Where the body gump and the item gumps are drawn.
    pub body_x: i32,
    pub body_y: i32,
    pub equipment_x: i32,
    pub equipment_y: i32,
}

impl PaperdollProfile {
    /// The gump an item whose tiledata anim id is `anim_id` draws, and the one drawn when that one is missing.
    pub fn equipment_gumps(&self, anim_id: u16) -> (u32, Option<u32>) {
        let anim_id = u32::from(anim_id);
        (anim_id + self.equipment_offset, self.fallback_equipment_offset.map(|offset| anim_id + offset))
    }
}

#[derive(Decode)]
struct PaperdollProfilesKdl {
    #[knuffel(children(name = "profile"))]
    profiles: Vec<PaperdollProfileKdl>,
}

#[derive(Decode)]
struct PaperdollProfileKdl {
    #[knuffel(argument)]
    id: String,
    #[knuffel(property)]
    label: Option<String>,
    #[knuffel(property(name = "equipment_offset"))]
    equipment_offset: u32,
    #[knuffel(property(name = "fallback_equipment_offset"))]
    fallback_equipment_offset: Option<u32>,
    #[knuffel(property)]
    body: u32,
    #[knuffel(property(name = "canvas_width"))]
    canvas_width: Option<u32>,
    #[knuffel(property(name = "canvas_height"))]
    canvas_height: Option<u32>,
    #[knuffel(property(name = "body_x"))]
    body_x: Option<i32>,
    #[knuffel(property(name = "body_y"))]
    body_y: Option<i32>,
    #[knuffel(property(name = "equipment_x"))]
    equipment_x: Option<i32>,
    #[knuffel(property(name = "equipment_y"))]
    equipment_y: Option<i32>,
}

impl From<PaperdollProfileKdl> for PaperdollProfile {
    fn from(value: PaperdollProfileKdl) -> Self {
        Self {
            label: value.label.unwrap_or_else(|| value.id.clone()),
            id: value.id,
            equipment_offset: value.equipment_offset,
            fallback_equipment_offset: value.fallback_equipment_offset,
            body: value.body,
            canvas_width: value.canvas_width.unwrap_or(0),
            canvas_height: value.canvas_height.unwrap_or(0),
            body_x: value.body_x.unwrap_or(0),
            body_y: value.body_y.unwrap_or(0),
            equipment_x: value.equipment_x.unwrap_or(0),
            equipment_y: value.equipment_y.unwrap_or(0),
        }
    }
}

/// The profiles of `text`, at least one; `source_name` names the file in the errors.
pub fn decode_paperdoll_profiles(source_name: &str, text: &str) -> eyre::Result<Vec<PaperdollProfile>> {
    let decoded: PaperdollProfilesKdl = uot_kdl::strict::decode(source_name, text, PAPERDOLL_PROFILES_SCHEMA)?;
    let profiles: Vec<_> = decoded.profiles.into_iter().map(PaperdollProfile::from).collect();
    if profiles.is_empty() {
        eyre::bail!("{source_name}: contains no profile nodes");
    }
    Ok(profiles)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE_NAME: &str = "PaperdollProfiles.kdl";

    #[test]
    fn a_profile_takes_its_fallback_and_defaults() {
        let profiles = decode_paperdoll_profiles(
            SOURCE_NAME,
            "profile \"female\" equipment_offset=60000 fallback_equipment_offset=50000 body=401\n\
             profile \"male\" label=\"Male\" equipment_offset=50000 body=400 canvas_width=260 equipment_y=-3\n",
        )
        .unwrap();
        let (female, male) = (&profiles[0], &profiles[1]);
        assert_eq!((female.label.as_str(), female.fallback_equipment_offset), ("female", Some(50_000)));
        assert_eq!(female.equipment_gumps(7), (60_007, Some(50_007)));
        assert_eq!((male.label.as_str(), male.fallback_equipment_offset), ("Male", None));
        assert_eq!(male.equipment_gumps(7), (50_007, None));
        assert_eq!((male.body, male.canvas_width, male.canvas_height, male.equipment_y), (400, 260, 0, -3));
    }

    #[test]
    fn unknown_properties_missing_values_and_no_profile_are_errors() {
        let error = |text: &str| decode_paperdoll_profiles(SOURCE_NAME, text).unwrap_err().to_string();
        let unknown = error("profile \"human_male\" equipment_offset=50000 body_z=1\n");
        let expected = "PaperdollProfiles.kdl: line 1, column 45: unknown property `body_z`";
        assert!(unknown.starts_with(expected), "{unknown}");
        assert!(unknown.contains("`fallback_equipment_offset`"), "{unknown}");
        let missing = error("profile \"human_male\" body=400\n");
        assert!(missing.contains("equipment_offset"), "{missing}");
        assert_eq!(error("// none\n"), "PaperdollProfiles.kdl: contains no profile nodes");
    }
}
