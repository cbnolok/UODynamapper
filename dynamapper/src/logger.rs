use chrono::Timelike;
//use pad::PadStr;
use regex::Regex;
use strum::VariantNames; // For the trait.
use strum_macros::{Display, EnumString, VariantNames};
//use std::io::Write; // for flush().
use std::sync::OnceLock;

// Event severity.
#[derive(Display, EnumString, VariantNames, PartialEq)]
pub enum LogSev {
    Debug,
    DebugVerbose,
    Diagnostics,
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
    Startup,
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
    show_caller_location_override: Option<bool>,
    severity: LogSev,
    about: LogAbout,
    msg: &str,
) {
    use std::fmt::Write;
    let show_location = show_caller_location_override.unwrap_or(true);

    //let now: OffsetDateTime = SystemTime::now().into(); // not adjusted by time zone
    let now = chrono::Local::now();
    let (h, m, s) = (now.hour(), now.minute(), now.second());

    // Format time without allocation
    let mut full_msg = String::with_capacity(256);
    write!(full_msg, "<d>{h:02}:{m:02}:{s:02} {{ ").unwrap();

    // Add file:line if enabled
    if show_location {
        let caller = std::panic::Location::caller();
        let loc_str: String = format!("{}:{}", caller.file(), caller.line());

        const PAD_WIDTH: usize = 46;
        let loc_trimmed: String = if loc_str.len() > PAD_WIDTH {
            let slice: &str = &loc_str[loc_str.len() - (PAD_WIDTH - 2)..];
            format!("..{}", slice)
        } else {
            loc_str
        };

        // Right-pad or truncate to PAD_WIDTH
        write!(full_msg, "{:width$}", loc_trimmed, width = PAD_WIDTH).unwrap();
    }

    full_msg.push_str(" }}</d> ");

    // About tag, pad to fixed width (18)
    let about_str = format!("[{about}]");
    write!(full_msg, "<b>{: <18}</b> ", about_str).unwrap();

    // Severity symbol (static &str)
    let sev_symbol: &'static str = match severity {
        LogSev::Debug => "<bright-magenta><bold><info></bold></>",
        LogSev::DebugVerbose => "<bright-magenta><bold><info></bold></>",
        LogSev::Diagnostics => "<dark-green><bold><info></bold></>",
        LogSev::Error => "<red><bold><cross></bold></>",
        LogSev::Info => "<cyan><bold><info></bold></>",
        LogSev::Warn => "<bright-yellow><bold><warn></bold></>",
    };
    full_msg.push_str(sev_symbol);
    full_msg.push(' ');

    // Style message (only clone/format if needed)
    match severity {
        LogSev::Diagnostics => write!(full_msg, "<dark-green>{msg}</>").unwrap(),
        LogSev::Error => write!(full_msg, "<red><bold>{msg}</></bold>").unwrap(),
        LogSev::Info => write!(full_msg, "<cyan>{msg}</>").unwrap(),
        LogSev::Warn => write!(full_msg, "<bright-yellow>{msg}</>").unwrap(),
        _ => full_msg.push_str(msg),
    }

    paris::log!("{full_msg}");
}

pub fn system(msg: &str) {
    paris::log!("<dark-green>{msg}</>");
}
