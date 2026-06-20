use std::ffi::OsString;

fn main() -> color_eyre::eyre::Result<()> {
    let args: Vec<OsString> = std::env::args_os().collect();
    match args.get(1).and_then(|s| s.to_str()) {
        Some("tool") => {
            let forwarded: Vec<OsString> = std::iter::once(args[0].clone())
                .chain(args[2..].iter().cloned())
                .collect();
            udd_conv_cli::tool_cli::run_with_args(forwarded)
        }
        Some("pack") => {
            let forwarded: Vec<OsString> = std::iter::once(args[0].clone())
                .chain(args[2..].iter().cloned())
                .collect();
            udd_conv_cli::pack_cli::run_with_args(forwarded)
        }
        _ => {
            eprintln!("Usage: udd-cli <tool|pack> [args...]");
            eprintln!("  tool  UDD package inspection and editing");
            eprintln!("  pack  UDD asset packing");
            std::process::exit(2);
        }
    }
}
