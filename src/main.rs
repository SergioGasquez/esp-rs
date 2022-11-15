use anyhow::Result;
use clap::Parser;
use directories_next::ProjectDirs;
use dirs::home_dir;
use embuild::{
    cmd,
    espidf::{parse_esp_idf_git_ref, EspIdfRemote},
};
use espup::{
    config::Config,
    emoji,
    error::Error,
    host_triple::get_host_triple,
    logging::initialize_logger,
    targets::{parse_targets, Target},
    toolchain::{
        espidf::{
            get_dist_path, get_install_path, get_tool_path, EspIdfRepo, DEFAULT_GIT_REPOSITORY,
        },
        gcc::{get_toolchain_name, install_gcc_targets},
        llvm::Llvm,
        rust::{
            check_rust_installation, install_extra_crates, install_riscv_target, Crate, XtensaRust,
        },
    },
    update::check_for_update,
};
use log::{debug, info, warn};
use std::{
    collections::HashSet,
    fs::{remove_dir_all, remove_file, File},
    io::Write,
    path::{Path, PathBuf},
};

#[cfg(windows)]
const DEFAULT_EXPORT_FILE: &str = "export-esp.ps1";
#[cfg(not(windows))]
const DEFAULT_EXPORT_FILE: &str = "export-esp.sh";

#[derive(Parser)]
#[command(
    name = "espup",
    bin_name = "espup",
    version,
    propagate_version = true,
    about,
    arg_required_else_help(true)
)]
struct Cli {
    #[command(subcommand)]
    subcommand: SubCommand,
}

#[derive(Parser)]
pub enum SubCommand {
    /// Installs esp-rs environment
    Install(Box<InstallOpts>),
    /// Uninstalls esp-rs environment
    Uninstall(UninstallOpts),
    /// Updates Xtensa Rust toolchain
    Update(UpdateOpts),
}

#[derive(Debug, Parser)]
pub struct InstallOpts {
    /// Target triple of the host.
    #[arg(short = 'd', long, required = false)]
    pub default_host: Option<String>,
    /// ESP-IDF version to install. If empty, no esp-idf is installed. Version format:
    ///
    /// - `commit:<hash>`: Uses the commit `<hash>` of the `esp-idf` repository.
    ///
    /// - `tag:<tag>`: Uses the tag `<tag>` of the `esp-idf` repository.
    ///
    /// - `branch:<branch>`: Uses the branch `<branch>` of the `esp-idf` repository.
    ///
    /// - `v<major>.<minor>` or `<major>.<minor>`: Uses the tag `v<major>.<minor>` of the `esp-idf` repository.
    ///
    /// - `<branch>`: Uses the branch `<branch>` of the `esp-idf` repository.
    ///
    /// When using this option, `ldproxy` crate will also be installed.
    #[arg(short = 'e', long, required = false)]
    pub esp_idf_version: Option<String>,
    /// Destination of the generated export file.
    #[arg(short = 'f', long)]
    pub export_file: Option<PathBuf>,
    /// Comma or space list of extra crates to install.
    #[arg(short = 'c', long, required = false, value_parser = Crate::parse_crates)]
    pub extra_crates: Option<HashSet<Crate>>,
    /// LLVM version.
    #[arg(short = 'x', long, default_value = "15", value_parser = ["15"])]
    pub llvm_version: String,
    /// Verbosity level of the logs.
    #[arg(short = 'l', long, default_value = "info", value_parser = ["debug", "info", "warn", "error"])]
    pub log_level: String,
    /// Nightly Rust toolchain version.
    #[arg(short = 'n', long, default_value = "nightly")]
    pub nightly_version: String,
    ///  Minifies the installation.
    #[arg(short = 'm', long)]
    pub profile_minimal: bool,
    /// Comma or space separated list of targets [esp32,esp32s2,esp32s3,esp32c2,esp32c3,all].
    #[arg(short = 't', long, default_value = "all", value_parser = parse_targets)]
    pub targets: HashSet<Target>,
    /// Xtensa Rust toolchain version.
    #[arg(short = 'v', long, value_parser = XtensaRust::parse_version)]
    pub toolchain_version: Option<String>,
}

#[derive(Debug, Parser)]
pub struct UpdateOpts {
    /// Target triple of the host.
    #[arg(short = 'd', long, required = false)]
    pub default_host: Option<String>,
    /// Verbosity level of the logs.
    #[arg(short = 'l', long, default_value = "info", value_parser = ["debug", "info", "warn", "error"])]
    pub log_level: String,
    /// Xtensa Rust toolchain version.
    #[arg(short = 'v', long, value_parser = XtensaRust::parse_version)]
    pub toolchain_version: Option<String>,
}

#[derive(Debug, Parser)]
pub struct UninstallOpts {
    /// Verbosity level of the logs.
    #[arg(short = 'l', long, default_value = "info", value_parser = ["debug", "info", "warn", "error"])]
    pub log_level: String,
}

/// Installs the Rust for ESP chips environment
fn install(args: InstallOpts) -> Result<()> {
    initialize_logger(&args.log_level);
    check_for_update(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
    info!("{} Installing esp-rs", emoji::DISC);
    let targets = args.targets;
    let host_triple = get_host_triple(args.default_host)?;
    let mut extra_crates = args.extra_crates;
    let mut exports: Vec<String> = Vec::new();
    let xtensa_rust = if targets.contains(&Target::ESP32)
        || targets.contains(&Target::ESP32S2)
        || targets.contains(&Target::ESP32S3)
    {
        let xtensa_rust: XtensaRust = if let Some(toolchain_version) = &args.toolchain_version {
            XtensaRust::new(toolchain_version, &host_triple)
        } else {
            let latest_version = XtensaRust::get_latest_version()?;
            XtensaRust::new(&latest_version, &host_triple)
        };
        Some(xtensa_rust)
    } else {
        None
    };
    let export_file = get_export_file(args.export_file)?;
    let llvm = Llvm::new(args.llvm_version, args.profile_minimal, &host_triple);

    debug!(
        "{} Arguments:
            - Host triple: {}
            - Targets: {:?}
            - ESP-IDF version: {:?}
            - Export file: {:?}
            - Extra crates: {:?}
            - LLVM Toolchain: {:?}
            - Nightly version: {:?}
            - Rust Toolchain: {:?}
            - Profile Minimal: {:?}
            - Toolchain version: {:?}",
        emoji::INFO,
        host_triple,
        targets,
        &args.esp_idf_version,
        &export_file,
        &extra_crates,
        llvm,
        &args.nightly_version,
        xtensa_rust,
        args.profile_minimal,
        args.toolchain_version,
    );

    #[cfg(windows)]
    check_arguments(&targets, &args.esp_idf_version)?;

    check_rust_installation(&args.nightly_version)?;

    if let Some(ref xtensa_rust) = xtensa_rust {
        xtensa_rust.install()?;
    }

    exports.extend(llvm.install()?);

    if targets.contains(&Target::ESP32C3) {
        install_riscv_target(&args.nightly_version)?;
    }

    if let Some(esp_idf_version) = &args.esp_idf_version {
        let repo = EspIdfRepo::new(esp_idf_version, args.profile_minimal, &targets);
        exports.extend(repo.install()?);
        if let Some(ref mut extra_crates) = extra_crates {
            extra_crates.insert(Crate::new("ldproxy"));
        } else {
            let mut crates = HashSet::new();
            crates.insert(Crate::new("ldproxy"));
            extra_crates = Some(crates);
        };
    } else {
        exports.extend(install_gcc_targets(&targets, &host_triple)?);
    }

    if let Some(ref extra_crates) = &extra_crates {
        install_extra_crates(extra_crates)?;
    }

    if args.profile_minimal {
        clear_dist_folder()?;
    }

    export_environment(&export_file, &exports)?;

    info!("{} Saving configuration file", emoji::WRENCH);
    let config = Config {
        esp_idf_version: args.esp_idf_version,
        export_file,
        extra_crates: extra_crates.as_ref().map(|extra_crates| {
            extra_crates
                .iter()
                .map(|x| x.name.clone())
                .collect::<HashSet<String>>()
        }),
        host_triple,
        llvm_path: llvm.path,
        nightly_version: args.nightly_version,
        targets,
        xtensa_rust,
    };
    config.save()?;

    info!("{} Installation successfully completed!", emoji::CHECK);
    warn!(
        "{} Please, source the export file, as state above, to properly setup the environment!",
        emoji::WARN
    );
    Ok(())
}

/// Uninstalls the Rust for ESP chips environment
fn uninstall(args: UninstallOpts) -> Result<()> {
    initialize_logger(&args.log_level);
    check_for_update(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));

    info!("{} Uninstalling esp-rs", emoji::DISC);
    let config = Config::load().unwrap();

    debug!(
        "{} Arguments:
            - Config: {:#?}",
        emoji::INFO,
        config
    );

    if let Some(xtensa_rust) = config.xtensa_rust {
        xtensa_rust.uninstall()?;
    }

    info!("{} Deleting Xtensa LLVM", emoji::WRENCH);
    remove_dir_all(config.llvm_path)?;

    if let Some(esp_idf_version) = config.esp_idf_version {
        info!("{} Deleting ESP-IDF {}", emoji::WRENCH, esp_idf_version);
        let repo = EspIdfRemote {
            git_ref: parse_esp_idf_git_ref(&esp_idf_version),
            repo_url: Some(DEFAULT_GIT_REPOSITORY.to_string()),
        };
        remove_dir_all(get_install_path(repo).parent().unwrap())?;
    } else {
        info!("{} Deleting GCC targets", emoji::WRENCH);
        for target in &config.targets {
            let gcc_path = get_tool_path(&get_toolchain_name(target));
            remove_dir_all(gcc_path)?;
        }
    }

    info!("{} Uninstalling extra crates", emoji::WRENCH);
    if let Some(extra_crates) = &config.extra_crates {
        for extra_crate in extra_crates {
            cmd!("cargo", "uninstall", extra_crate).run()?;
        }
    }

    clear_dist_folder()?;

    info!("{} Deleting export file", emoji::WRENCH);
    remove_file(Path::new(&config.export_file))?;

    info!("{} Deleting config file", emoji::WRENCH);
    let conf_dirs = ProjectDirs::from("rs", "esp", "espup").unwrap();
    let conf_file = conf_dirs.config_dir().join("espup.toml");
    remove_file(conf_file)?;

    info!("{} Uninstallation successfully completed!", emoji::CHECK);
    Ok(())
}

/// Updates Xtensa Rust toolchain.
fn update(args: UpdateOpts) -> Result<()> {
    initialize_logger(&args.log_level);
    check_for_update(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));

    info!("{} Updating ESP Rust environment", emoji::DISC);
    let host_triple = get_host_triple(args.default_host)?;
    let mut config = Config::load().unwrap();
    let xtensa_rust: XtensaRust = if let Some(toolchain_version) = args.toolchain_version {
        XtensaRust::new(&toolchain_version, &host_triple)
    } else {
        let latest_version = XtensaRust::get_latest_version()?;
        XtensaRust::new(&latest_version, &host_triple)
    };

    debug!(
        "{} Arguments:
            - Host triple: {}
            - Toolchain version: {:#?}
            - Config: {:#?}",
        emoji::INFO,
        host_triple,
        xtensa_rust,
        config
    );

    if let Some(config_xtensa_rust) = config.xtensa_rust {
        if config_xtensa_rust.version == xtensa_rust.version {
            info!(
                "{} Toolchain '{}' is already up to date",
                emoji::CHECK,
                xtensa_rust.version
            );
            return Ok(());
        }
        config_xtensa_rust.uninstall()?;
        xtensa_rust.install()?;
        config.xtensa_rust = Some(xtensa_rust);
    }

    config.save()?;

    info!("{} Update successfully completed!", emoji::CHECK);
    Ok(())
}

fn main() -> Result<()> {
    match Cli::parse().subcommand {
        SubCommand::Install(args) => install(*args),
        SubCommand::Update(args) => update(args),
        SubCommand::Uninstall(args) => uninstall(args),
    }
}

/// Deletes dist folder.
fn clear_dist_folder() -> Result<()> {
    let dist_path = PathBuf::from(get_dist_path(""));
    if dist_path.exists() {
        info!("{} Clearing dist folder", emoji::WRENCH);
        remove_dir_all(&dist_path)?;
    }
    Ok(())
}

/// Returns the absolute path to the export file, uses the DEFAULT_EXPORT_FILE if no arg is provided.
fn get_export_file(export_file: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(export_file) = export_file {
        if export_file.is_absolute() {
            Ok(export_file)
        } else {
            let current_dir = std::env::current_dir()?;
            Ok(current_dir.join(export_file))
        }
    } else {
        let home_dir = home_dir().unwrap();
        Ok(home_dir.join(DEFAULT_EXPORT_FILE))
    }
}

/// Creates the export file with the necessary environment variables.
fn export_environment(export_file: &PathBuf, exports: &[String]) -> Result<()> {
    info!("{} Creating export file", emoji::WRENCH);
    let mut file = File::create(export_file)?;
    for e in exports.iter() {
        file.write_all(e.as_bytes())?;
        file.write_all(b"\n")?;
    }
    #[cfg(windows)]
    warn!(
        "{} PLEASE set up the environment variables running: '.\\{}'",
        emoji::INFO,
        export_file.display()
    );
    #[cfg(unix)]
    warn!(
        "{} PLEASE set up the environment variables running: '. {}'",
        emoji::INFO,
        export_file.display()
    );
    warn!(
        "{} This step must be done every time you open a new terminal.",
        emoji::WARN
    );
    Ok(())
}

#[cfg(windows)]
/// For Windows, we need to check that we are installing all the targets if we are installing esp-idf.
pub fn check_arguments(
    targets: &HashSet<Target>,
    espidf_version: &Option<String>,
) -> Result<(), Error> {
    if espidf_version.is_some()
        && (!targets.contains(&Target::ESP32)
            || !targets.contains(&Target::ESP32C3)
            || !targets.contains(&Target::ESP32S2)
            || !targets.contains(&Target::ESP32S3))
    {
        return Err(Error::WrongWindowsArguments);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{get_export_file, DEFAULT_EXPORT_FILE};
    use dirs::home_dir;
    use std::{env::current_dir, path::PathBuf};

    #[test]
    #[allow(unused_variables)]
    fn test_get_export_file() {
        // No arg provided
        let home_dir = home_dir().unwrap();
        let export_file = home_dir.join(DEFAULT_EXPORT_FILE);
        assert!(matches!(get_export_file(None), Ok(export_file)));
        // Relative path
        let current_dir = current_dir().unwrap();
        let export_file = current_dir.join("export.sh");
        assert!(matches!(
            get_export_file(Some(PathBuf::from("export.sh"))),
            Ok(export_file)
        ));
        // Absolute path
        let export_file = PathBuf::from("/home/user/export.sh");
        assert!(matches!(
            get_export_file(Some(PathBuf::from("/home/user/export.sh"))),
            Ok(export_file)
        ));
    }
}
