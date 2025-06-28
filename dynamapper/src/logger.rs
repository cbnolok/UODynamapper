use pad::PadStr;
use regex::Regex;
use strum::VariantNames; // For the trait.
use strum_macros::{Display, EnumString, VariantNames}; // For the derive macros.
use std::sync::OnceLock;

#[derive(Display, EnumString, VariantNames, PartialEq)]
pub enum Severity {
    Debug,
    Error,
    Info,
    InternalVerbose1,
    InternalVerbose2,
    Warn,
}

#[derive(Display, EnumString, VariantNames, PartialEq)]
pub enum About {
    AppState,
    Camera,
    General,
    Input,
    InternalAssets,
    Player,
    Plugins,
    Rendering,
    Systems,
    UoFiles,
}

//type MsgData = (Severity, About);

#[allow(unused)]
fn enum_severity_str_vals() -> &'static Regex {
    static ENUM_SEVERITY: OnceLock<Regex> = OnceLock::new();
    ENUM_SEVERITY.get_or_init(|| {
        // Piece together the expression from enum's variant names.
        let expr_str = Severity::VARIANTS.join("|");
        Regex::new(&expr_str).unwrap()
    })
}

#[allow(unused)]
fn enum_about_str_vals() -> &'static Regex {
    static ENUM_ABOUT: OnceLock<Regex> = OnceLock::new();
    ENUM_ABOUT.get_or_init(|| {
        // Piece together the expression from enum's variant names.
        let expr_str = About::VARIANTS.join("|");
        Regex::new(&expr_str).unwrap()
    })
}

#[allow(unused)]
fn enum_about_variant_name_validate(val: &str) -> bool {
    /*
        if let Some(captures) = ENUM_ABOUT.captures(val) {

            // Get the substring that matched one of the variants.
            let variant_name = &captures[0];

            // Convert the string to the actual variant.
            let variant = Thing::from_str(variant_name).unwrap();

            println!("variant name: {:<8} --  variant: {:?}",
                     variant_name, variant);
        }
     */

    true
}

#[allow(unused)]
fn can_show_msg(severity: Severity, about: About) -> bool {
    true
}

#[track_caller]
pub fn one(
    mut show_caller_location_override: Option<bool>,
    severity: Severity,
    about: About,
    msg: &str,
) {
    if show_caller_location_override == None {
        // Default, for now.
        show_caller_location_override = Some(true);
    }

    let mut location_str = String::new();
    if show_caller_location_override == Some(true) {
        let caller_location = std::panic::Location::caller();
        //location_str = format!("{{{}:{}}}\t", caller_location.file(), caller_location.line());
        location_str = format!(
            "{} | ",
            format!("{{{}:{}}}", caller_location.file(), caller_location.line()).pad_to_width(40)
        );
    }

    let full_msg = format!(
        "Dynamapper | {location_str}{} | {msg}",
        format!("[{about}]").pad_to_width(12)
    );

    match severity {
        Severity::Debug => paris::log!("<bright-magenta><bold><info></bold></> {full_msg}"),
        Severity::Error => paris::log!("<red><bold><cross></bold></> {full_msg}"),
        Severity::Info  => paris::log!("<cyan><bold><info></bold></> {full_msg}"),
        Severity::Warn  => paris::log!("<yellow><bold><warn></bold></> {full_msg}"),
        Severity::InternalVerbose1 => paris::log!("<bright-magenta><bold><info></bold></> {full_msg}"),
        Severity::InternalVerbose2 => paris::log!("<bright-magenta><bold><info></bold></> {full_msg}"),
    }
}

