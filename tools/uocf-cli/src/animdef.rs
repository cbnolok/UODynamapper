use std::path::{Path, PathBuf};

use clap::{Subcommand, ValueEnum};
use color_eyre::eyre::{self, WrapErr};
use uocf::enhanced::animation_definition::{
    animation_definition_kdl_file_name, AnimationDefinitionCollection, AnimationIssueSeverity,
};
use uocf::enhanced::client_dialect::ClientDialect;
use uocf::enhanced::effect_definition::EffectValidationContext;
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::utils::path::find_file_ignoring_case;

/// The string dictionary next to the package, which names the particles of frame effects.
const STRING_DICTIONARY_UOP_NAME: &str = "string_dictionary.uop";

#[derive(Subcommand, Debug)]
pub enum AnimdefCmd {
    /// Write every entry of an AnimationDefinition.uop (or the given items) as one KDL file each,
    /// `<item id, 8 digits>.kdl`.
    Export {
        /// Source AnimationDefinition.uop.
        uop: PathBuf,
        /// Output folder, created when missing.
        #[arg(long)]
        output: PathBuf,
        /// Item ids to export (repeatable); every entry when omitted.
        #[arg(long = "item", value_name = "ID")]
        items: Vec<u32>,
        #[command(flatten)]
        options: AnimdefOptions,
    },
    /// Replace or add the entries of KDL files (or folders of them) and write a copy of the package; untouched
    /// entries keep their bytes and headers.
    Import {
        /// Source AnimationDefinition.uop.
        uop: PathBuf,
        /// KDL files, or folders whose .kdl files are read.
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Output AnimationDefinition.uop; never the source.
        #[arg(long)]
        output: PathBuf,
        #[command(flatten)]
        options: AnimdefOptions,
    },
}

#[derive(clap::Args, Debug)]
pub struct AnimdefOptions {
    /// Effect grammar of the package: `auto` is Kingdom Reborn next to UOKR.exe, else Enhanced Client.
    #[arg(long, value_enum, default_value_t = DialectArg::Auto)]
    dialect: DialectArg,
    /// Do not read string_dictionary.uop next to the package (particle names are then indices only).
    #[arg(long)]
    no_names: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum DialectArg {
    #[default]
    Auto,
    Enhanced,
    KingdomReborn,
}

impl DialectArg {
    fn resolve(self, uop: &Path) -> ClientDialect {
        match self {
            Self::Auto => ClientDialect::of_package_path(uop),
            Self::Enhanced => ClientDialect::Enhanced,
            Self::KingdomReborn => ClientDialect::KingdomReborn,
        }
    }
}

pub fn run(cmd: AnimdefCmd) -> eyre::Result<()> {
    match cmd {
        AnimdefCmd::Export { uop, output, items, options } => {
            let (collection, strings) = load(&uop, &options)?;
            if items.is_empty() {
                let count = collection.export_kdl_files(&output, strings.as_ref())?;
                println!("Wrote {count} KDL files ({}) to {}.", collection.dialect.label(), output.display());
                return Ok(());
            }
            std::fs::create_dir_all(&output).wrap_err_with(|| format!("failed to create {}", output.display()))?;
            for &item in &items {
                collection.export_kdl_file(item, output.join(animation_definition_kdl_file_name(item)), strings.as_ref())?;
            }
            println!("Wrote {} KDL files ({}) to {}.", items.len(), collection.dialect.label(), output.display());
        }
        AnimdefCmd::Import { uop, inputs, output, options } => {
            let (mut collection, strings) = load(&uop, &options)?;
            let import = collection.import_kdl_paths(&inputs, strings.as_ref())?;
            let context = EffectValidationContext { strings: strings.as_ref(), particle_file_exists: None };
            let mut errors = 0usize;
            for &item in import.replaced.iter().chain(&import.added) {
                for mut issue in collection.get(item).expect("imported").validate(&context) {
                    issue.item_id = Some(item);
                    errors += usize::from(issue.severity() == AnimationIssueSeverity::Error);
                    println!("{:?}: {issue}", issue.severity());
                }
            }
            eyre::ensure!(errors == 0, "{errors} errors in the imported entries; nothing written");
            let changes = collection.write_package(&uop, &output)?;
            println!(
                "Read {} KDL files ({} entries replaced, {} added); wrote {} ({}): {} entries changed, {} added.",
                import.files.len(),
                import.replaced.len(),
                import.added.len(),
                output.display(),
                collection.dialect.label(),
                changes.replaced,
                changes.added
            );
        }
    }
    Ok(())
}

fn load(uop: &Path, options: &AnimdefOptions) -> eyre::Result<(AnimationDefinitionCollection, Option<UoStringDictionary>)> {
    let collection = AnimationDefinitionCollection::load_dialect(uop, options.dialect.resolve(uop))?;
    let strings_path = (!options.no_names)
        .then(|| uop.parent().and_then(|dir| find_file_ignoring_case(dir, STRING_DICTIONARY_UOP_NAME)))
        .flatten();
    let strings = strings_path
        .map(|path| UoStringDictionary::load(&path).wrap_err_with(|| format!("failed to load {}", path.display())))
        .transpose()?;
    Ok((collection, strings))
}
