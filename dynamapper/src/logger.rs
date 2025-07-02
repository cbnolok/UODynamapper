use pad::PadStr;
use regex::Regex;
use strum::VariantNames; // For the trait.
use strum_macros::{Display, EnumString, VariantNames}; // For the derive macros.
//use std::io::Write; // for flush().
use std::sync::OnceLock;

// Event severity.
#[derive(Display, EnumString, VariantNames, PartialEq)]
pub enum LogSev {
    Debug,
    DebugVerbose,
    Error,
    Info,
    Warn,
}

// Event context.
#[derive(Display, EnumString, VariantNames, PartialEq)]
pub enum LogAbout {
    AppState,
    Camera,
    General,
    Input,
    InternalAssets,
    Player,
    Plugins,
    Renderer,
    RenderWorldArt,
    RenderWorldLand,
    SystemsGeneral,
    UoFiles,
}

//type MsgData = (Severity, About);

#[allow(unused)]
fn enum_severity_str_vals() -> &'static Regex {
    static ENUM_SEVERITY: OnceLock<Regex> = OnceLock::new();
    ENUM_SEVERITY.get_or_init(|| {
        // Piece together the expression from enum's variant names.
        let expr_str = LogSev::VARIANTS.join("|");
        Regex::new(&expr_str).unwrap()
    })
}

#[allow(unused)]
fn enum_about_str_vals() -> &'static Regex {
    static ENUM_ABOUT: OnceLock<Regex> = OnceLock::new();
    ENUM_ABOUT.get_or_init(|| {
        // Piece together the expression from enum's variant names.
        let expr_str = LogAbout::VARIANTS.join("|");
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
fn can_show_msg(severity: LogSev, about: LogAbout) -> bool {
    true
}

#[track_caller]
pub fn one(
    mut show_caller_location_override: Option<bool>,
    severity: LogSev,
    about: LogAbout,
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

        let msg = format!("{}:{}", caller_location.file(), caller_location.line());

        let pad_width = 46;
        let mut cut_left_chr_amount = msg.len().saturating_sub(pad_width);
        if cut_left_chr_amount != 0 {
            cut_left_chr_amount += 2;
            location_str += "..";
        }
        location_str += &msg[cut_left_chr_amount..msg.len()];
        location_str = location_str.with_exact_width(pad_width);
    }

    let about_msg = format!("[{about}]").pad_to_width(18); //.pad(18, ' ', pad::Alignment::Middle, true)
    let full_msg = format!("<d>{{ {location_str} }}</d> <b>{about_msg}</b> {msg}");

    match severity {
        LogSev::Debug => paris::log!("<bright-magenta><bold><info></bold></> {full_msg}"),
        LogSev::DebugVerbose => paris::log!("<bright-magenta><bold><info></bold></> {full_msg}"),
        LogSev::Error => paris::log!("<red><bold><cross></bold></> {full_msg}"),
        LogSev::Info => paris::log!("<cyan><bold><info></bold></> {full_msg}"),
        LogSev::Warn => paris::log!("<yellow><bold><warn></bold></> {full_msg}"),
    }
}
