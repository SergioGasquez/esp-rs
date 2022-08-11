extern crate clap;
extern crate json;
use clap::Parser;
use clap_nested::Commander;
use dirs::home_dir;
// use std::error::Error;
use std::path::{Path, PathBuf};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
use crate::config::get_tool_path;
use crate::idf::install_espidf;
use crate::package::{prepare_package_strip_prefix, prepare_single_binary};
use crate::shell::{run_command, update_env_path};
use espflash::Chip;
use std::env;
use std::io::{Error, ErrorKind};
use std::process::Stdio;
use std::str::FromStr;
mod config;
mod idf;
mod package;
mod shell;
use std::fs;

// General TODOs:
// - Prettify prints (add emojis)
// - Add subcommand test that downloads a projects and builds it
// - Esp-idf version should be contained in an enum with the possible values (see chips in espflash for reference)
// - Check if LdProxy is needed when no esp-idf is installed (if not needed only install it for esp-idf)
// - Do a Tauri App so we can install it with gui
// - Add tests
// - Clean unused code
// - Add progress bar

#[derive(Parser)]
struct Opts {
    #[clap(subcommand)]
    subcommand: SubCommand,
}

#[derive(Parser)]
pub enum SubCommand {
    /// Installs esp-rs environment
    Install(InstallOpts),
    /// Updates esp-rs Rust toolchain
    Update(UpdateOpts),
    /// Uninstalls esp-rs environment
    Uninstall(UninstallOpts),
    /// Reinstalls esp-rs environment
    Reinstall(InstallOpts),
}

#[derive(Parser, Debug)]
pub struct InstallOpts {
    /// Comma or space separated list of targets [esp32,esp32s2,esp32s3,esp32c3,all].
    // Make it vector and have splliter =" "
    #[clap(short = 'b', long, default_value = "esp32,esp32s2,esp32s3")]
    pub build_target: String,
    /// Path to .cargo.
    // TODO: Use home_dir to make it diferent in every OS: #[clap(short = 'c', long, default_value_t: &'a Path = Path::new(format!("{}/.cargo",home_dir())))]
    #[clap(short = 'c', long, default_value = "/home/esp/.cargo")]
    pub cargo_home: PathBuf,
    /// Toolchain instalation folder.
    #[clap(short = 'd', long, default_value = "/home/esp/.rustup/toolchains/esp")]
    pub toolchain_destination: PathBuf,
    /// Comma or space list of extra crates to install.
    // Make it vector and have splliter =" "
    #[clap(short = 'e', long, default_value = "ldproxy cargo-espflash")]
    pub extra_crates: String,
    /// Destination of the export file generated.
    #[clap(short = 'f', long)]
    pub export_file: Option<PathBuf>,
    /// LLVM version. [13, 14, 15]
    // TODO: Use Enum with 13, 14 and, when released, 15
    #[clap(short = 'l', long, default_value = "14")]
    pub llvm_version: String,
    ///  [Only applies if using -s|--esp-idf-version]. Deletes some esp-idf folders to save space.
    #[clap(short = 'm', long)]
    pub minified_espidf: Option<bool>,
    /// Nightly Rust toolchain version.
    #[clap(short = 'n', long, default_value = "nightly")]
    pub nightly_version: String,
    // /// Path to .rustup.
    #[clap(short = 'r', long, default_value = "/home/esp/.rustup")]
    pub rustup_home: PathBuf,
    // /// ESP-IDF branch to install. If empty, no esp-idf is installed.
    #[clap(short = 's', long)]
    pub espidf_version: Option<String>,
    /// Xtensa Rust toolchain version.
    #[clap(short = 't', long, default_value = "1.62.1.0")]
    pub toolchain_version: String,
    /// Removes cached distribution files.
    #[clap(short = 'x', long)]
    pub clear_cache: bool,
}

#[derive(Parser, Debug)]
pub struct UpdateOpts {
    /// Xtensa Rust toolchain version.
    #[clap(short = 't', long, default_value = "1.62.1.0")]
    pub toolchain_version: String,
}

#[derive(Parser, Debug)]
pub struct UninstallOpts {
    /// Removes clang.
    #[clap(short = 'r', long)]
    pub remove_clang: bool,
    // TODO: Other options to remove?
}

fn install(args: InstallOpts) -> Result<()> {
    println!("{:?}", args);
    let arch = guess_host_triple::guess_host_triple().unwrap();
    println!("{}", arch);
    let targets: Vec<Chip> = parse_targets(&args.build_target)?;
    println!("targets: {:?}", targets);
    let llvm_version = parse_llvm_version(&args.llvm_version).unwrap();
    println!("llvm_version: {:?}", llvm_version);

    let llvm_release = args.llvm_version.clone();
    let artifact_file_extension = get_artifact_file_extension(arch).to_string();
    let llvm_arch = get_llvm_arch(arch).to_string();
    let llvm_file = format!(
        "xtensa-esp32-elf-llvm{}-{}-{}.{}",
        get_llvm_version_with_underscores(&llvm_version),
        &llvm_version,
        llvm_arch,
        artifact_file_extension
    );
    let rust_dist = format!("rust-{}-{}", args.toolchain_version, arch);
    let rust_src_dist = format!("rust-src-{}", args.toolchain_version);
    let rust_dist_file = format!("{}.{}", rust_dist, artifact_file_extension);
    let rust_src_dist_file = format!("{}.{}", rust_src_dist, artifact_file_extension);
    let rust_dist_url = format!(
        "https://github.com/esp-rs/rust-build/releases/download/v{}/{}",
        args.toolchain_version, rust_dist_file
    );
    let rust_src_dist_url = format!(
        "https://github.com/esp-rs/rust-build/releases/download/v{}/{}",
        args.toolchain_version, rust_src_dist_file
    );
    let llvm_url = format!(
        "https://github.com/espressif/llvm-project/releases/download/{}/{}",
        &llvm_version, llvm_file
    );
    let idf_tool_xtensa_elf_clang = format!(
        "{}/{}-{}",
        get_tool_path("xtensa-esp32-elf-clang".to_string()),
        &llvm_version,
        arch
    );
    let mut exports: Vec<String> = Vec::new();
    check_rust_installation(&args.nightly_version);
    // TODO: Move to a function

    if args.toolchain_destination.exists() {
        println!(
            "Previous installation of Rust Toolchain exist in: {}",
            args.toolchain_destination.display().to_string()
        );
        println!("Please, remove the directory before new installation.");
        return Ok(());
    } else {
        // install_rust_xtensa_toolchain
        // Some platfroms like Windows are available in single bundle rust + src, because install
        // script in dist is not available for the plaform. It's sufficient to extract the toolchain
        println!("Installing Xtensa Rust toolchain");
        if get_rust_installer(arch).to_string().is_empty() {
            // TODO: Check idf_env and adjust
            // match prepare_package_strip_prefix(&rust_dist_url,
            //                              &rust_dist_file,
            //                              get_tool_path("rust".to_string()),
            //                              "esp") {
            //                                 Ok(_) => { println!("Package ready"); },
            //                                 Err(_e) => { println!("Unable to prepare package"); }
            //                             }
        } else {
            match prepare_package_strip_prefix(
                &rust_dist_url,
                get_tool_path("rust".to_string()),
                &format!("rust-nightly-{}", arch),
            ) {
                Ok(_) => {
                    println!("Package rust ready");
                }
                Err(_e) => {
                    println!("Unable to prepare rust");
                }
            }

            let mut arguments: Vec<String> = [].to_vec();
            println!(
                "{}/install.sh --destdir={} --prefix='' --without=rust-docs",
                get_tool_path("rust".to_string()),
                args.toolchain_destination.display()
            );
            arguments.push("-c".to_string());
            arguments.push(format!(
                "{}/install.sh --destdir={} --prefix='' --without=rust-docs",
                get_tool_path("rust".to_string()),
                args.toolchain_destination.display()
            ));

            match run_command("/bin/bash".to_string(), arguments.clone(), "".to_string()) {
                Ok(_) => {
                    println!("rust/install.sh command succeeded");
                }
                Err(_e) => {
                    println!("rust/install.sh command failed");
                }
            }

            match prepare_package_strip_prefix(
                &rust_src_dist_url,
                get_tool_path("rust-src".to_string()),
                "rust-src-nightly",
            ) {
                Ok(_) => {
                    println!("Package rust-src ready");
                }
                Err(_e) => {
                    println!("Unable to prepare rust-src");
                }
            }

            let mut arguments: Vec<String> = [].to_vec();
            println!(
                "{}/install.sh --destdir={} --prefix='' --without=rust-docs",
                get_tool_path("rust-src".to_string()),
                args.toolchain_destination.display()
            );
            arguments.push("-c".to_string());
            arguments.push(format!(
                "{}/install.sh --destdir={} --prefix='' --without=rust-docs",
                get_tool_path("rust-src".to_string()),
                args.toolchain_destination.display()
            ));
            match run_command("/bin/bash".to_string(), arguments, "".to_string()) {
                Ok(_) => {
                    println!("rust-src/install.sh Command succeeded");
                }
                Err(_e) => {
                    println!("rust-src/install.sh Command failed");
                }
            }
        }
    }

    // install_llvm_clang
    if Path::new(idf_tool_xtensa_elf_clang.as_str()).exists() {
        println!(
            "Previous installation of LLVM exist in: {}",
            idf_tool_xtensa_elf_clang
        );
        println!("Please, remove the directory before new installation.");
    } else {
        println!("Downloading xtensa-esp32-elf-clang");
        match prepare_package_strip_prefix(
            &llvm_url,
            get_tool_path(
                format!("xtensa-esp32-elf-clang-{}-{}", &llvm_version, llvm_arch).to_string(),
            ),
            "",
        ) {
            Ok(_) => {
                println!("Package xtensa-esp32-elf-clang ready");
            }
            Err(_e) => {
                println!("Unable to prepare xtensa-esp32-elf-clang");
            }
        }
    }
    let libclang_path = format!(
        "{}/lib",
        get_tool_path("xtensa-esp32-elf-clang".to_string())
    );
    println!("export LIBCLANG_PATH=\"{}\"", &libclang_path);
    exports.push(format!("export LIBCLANG_PATH=\"{}\"", &libclang_path));

    // TODO: Insall riscv target in nigthly if installing esp32c3

    if args.espidf_version.is_some() {
        idf::install_espidf(&args.build_target, args.espidf_version.unwrap())?;
        exports.push(format!(
            "export IDF_TOOLS_PATH=\"{}\"",
            config::get_espressif_base_path()
        ));
        exports.push(format!(". ./{}/export.sh\"", "TODO:UPDATE"));
    } else {
        println!("No esp-idf version provided. Installing gcc for targets");
        exports.extend(install_gcc_targets(targets)?.iter().cloned());
    }

    // TODO: Install extra crates
    // match args.extra_crates {
    //     // args.extra_crates.contains("cargo") => {
    //     //     println!("Installing cargo");
    //     //     install_cargo();
    //     // }
    //     //     "mingw" => {
    //     //         // match arch {
    //     //         //     "x86_64-pc-windows-gnu" => {
    //     //         //         install_mingw(toolchain);
    //     //         //     }
    //     //         //     _ => { println!("Ok"); }
    //     //         // }
    //     //     },
    //     _ => {
    //         println!("No extra tools selected");
    //     }
    // }

    // TODO: Clear cache

    // TODO: Set environment
    println!("Updating environment variables:");
    for e in exports.iter() {
        println!("{}", e);
    }

    // #[cfg(windows)]
    // println!("PATH+=\";{}\"", libclang_bin);
    // #[cfg(unix)]
    // println!("export PATH=\"{}:$PATH\"", libclang_bin);

    // update_env_path(&libclang_bin);

    return Ok(());
}

fn update(args: UpdateOpts) -> Result<()> {
    // TODO: Update Rust toolchain
    todo!();
}

fn uninstall(args: UninstallOpts) -> Result<()> {
    // TODO: Uninstall
    todo!();
}

fn reinstall(args: InstallOpts) -> Result<()> {
    todo!();
    // uninstall();
    // install(args);
}

#[tokio::main]
async fn main() -> Result<()> {
    match Opts::parse().subcommand {
        SubCommand::Install(args) => install(args),
        SubCommand::Update(args) => update(args),
        SubCommand::Uninstall(args) => uninstall(args),
        SubCommand::Reinstall(args) => reinstall(args),
    }
}

fn get_rust_installer(arch: &str) -> &str {
    match arch {
        "x86_64-pc-windows-msvc" => "",
        "x86_64-pc-windows-gnu" => "",
        _ => "./install.sh",
    }
}

fn install_rust_nightly(version: &str) {
    println!("installing nightly toolchain");
    match std::process::Command::new("rustup")
        .arg("toolchain")
        .arg("install")
        .arg(version)
        .arg("--profile")
        .arg("minimal")
        .stdout(Stdio::piped())
        .output()
    {
        Ok(child_output) => {
            let result = String::from_utf8_lossy(&child_output.stdout);
            println!("Result: {}", result);
        }
        Err(e) => {
            println!("Error: {}", e);
        }
    }
}

fn install_rustup() {
    #[cfg(windows)]
    let rustup_init_path =
        prepare_single_binary("https://win.rustup.rs/x86_64", "rustup-init.exe", "rustup");
    #[cfg(unix)]
    let rustup_init_path = prepare_single_binary("https://sh.rustup.rs/", "rustup-init", "rustup");
    println!("rustup stable");
    match std::process::Command::new(rustup_init_path)
        .arg("--default-toolchain")
        .arg("none")
        .arg("--profile")
        .arg("minimal")
        .arg("-y")
        .stdout(Stdio::piped())
        .output()
    {
        Ok(child_output) => {
            let result = String::from_utf8_lossy(&child_output.stdout);
            println!("{}", result);
        }
        Err(e) => {
            println!("Error: {}", e);
        }
    }
}

fn get_llvm_version_with_underscores(llvm_version: &str) -> String {
    let version: Vec<&str> = llvm_version.split("-").collect();
    let llvm_dot_version = version[1];
    llvm_dot_version.replace(".", "_")
}

fn get_artifact_file_extension(arch: &str) -> &str {
    match arch {
        "x86_64-pc-windows-msvc" => "zip",
        "x86_64-pc-windows-gnu" => "zip",
        _ => "tar.xz",
    }
}

fn get_llvm_arch(arch: &str) -> &str {
    match arch {
        "aarch64-apple-darwin" => "macos",
        "x86_64-apple-darwin" => "macos",
        "x86_64-unknown-linux-gnu" => "linux-amd64",
        "x86_64-pc-windows-msvc" => "win64",
        "x86_64-pc-windows-gnu" => "win64",
        _ => arch,
    }
}

fn get_gcc_arch(arch: &str) -> &str {
    match arch {
        "aarch64-apple-darwin" => "macos",
        "aarch64-unknown-linux-gnu" => "linux-arm64",
        "x86_64-apple-darwin" => "macos",
        "x86_64-unknown-linux-gnu" => "linux-amd64",
        "x86_64-pc-windows-msvc" => "win64",
        "x86_64-pc-windows-gnu" => "win64",
        _ => arch,
    }
}

fn install_gcc_targets(targets: Vec<Chip>) -> Result<Vec<String>> {
    let mut exports: Vec<String> = Vec::new();
    for target in targets {
        match target {
            Chip::Esp32 => {
                install_gcc("xtensa-esp32-elf");
                exports.push(format!(
                    "export PATH={}:$PATH",
                    get_tool_path("xtensa-esp32-elf/bin".to_string())
                ));
            }
            Chip::Esp32s2 => {
                install_gcc("xtensa-esp32s2-elf");
                exports.push(format!(
                    "export PATH={}:$PATH",
                    get_tool_path("xtensa-esp32s2-elf/bin".to_string())
                ));
            }
            Chip::Esp32s3 => {
                install_gcc("xtensa-esp32s3-elf");
                exports.push(format!(
                    "export PATH={}:$PATH",
                    get_tool_path("xtensa-esp32s3-elf/bin".to_string())
                ));
            }
            Chip::Esp32c3 => {
                install_gcc("riscv32-esp-elf");
                exports.push(format!(
                    "export PATH={}:$PATH",
                    get_tool_path("riscv32-esp-elf/bin".to_string())
                ));
            }
            _ => {
                println!("Unknown target")
            }
        }
    }
    Ok(exports)
}

fn install_gcc(gcc_target: &str) {
    let gcc_path = get_tool_path(gcc_target.to_string());
    println!("gcc path: {}", gcc_path);
    // if Path::new(&gcc_path).exists() {
    //     println!("Previous installation of GCC for target: {}", gcc_path);
    //     // return Ok(());
    // } else {
    // fs::create_dir_all(&gcc_path).unwrap();
    let gcc_file = format!(
        "{}-gcc8_4_0-esp-2021r2-patch3-{}.tar.gz",
        gcc_target,
        get_gcc_arch(guess_host_triple::guess_host_triple().unwrap())
    );
    let gcc_dist_url = format!(
        "https://github.com/espressif/crosstool-NG/releases/download/esp-2021r2-patch3/{}",
        gcc_file
    );
    match prepare_package_strip_prefix(&gcc_dist_url, gcc_path, "") {
        Ok(_) => {
            println!("Package {} ready", gcc_file);
        }
        Err(_e) => {
            println!("Unable to prepare {}", gcc_file);
        }
    }
    // }
}
// TODO: Create test for this function
fn parse_targets(build_target: &str) -> Result<Vec<Chip>> {
    println!("Parsing targets: {}", build_target);
    let mut chips: Vec<Chip> = Vec::new();
    if build_target.contains("all") {
        chips.push(Chip::Esp32);
        chips.push(Chip::Esp32s2);
        chips.push(Chip::Esp32s3);
        chips.push(Chip::Esp32c3);
        return Ok(chips);
    }
    let mut targets: Vec<&str>;
    if build_target.contains(' ') || build_target.contains(',') {
        targets = build_target.split([',', ' ']).collect();
    } else {
        targets = vec![build_target];
    }
    for target in targets {
        match target {
            "esp32" => chips.push(Chip::Esp32),
            "esp32s2" => chips.push(Chip::Esp32s2),
            "esp32s3" => chips.push(Chip::Esp32s3),
            "esp32c3" => chips.push(Chip::Esp32c3),
            _ => {
                return Err(Box::new(Error::new(
                    ErrorKind::Other,
                    format!("Unknown target: {}", target),
                )));
            }
        };
    }

    Ok(chips)
}

fn parse_llvm_version(llvm_version: &str) -> Result<String> {
    let parsed_version = match llvm_version {
        "13" => "esp-13.0.0-20211203",
        "14" => "esp-14.0.0-20220415",
        "15" => "", // TODO: Fill when released
        _ => {
            return Err(Box::new(Error::new(
                ErrorKind::Other,
                format!("Unknown LLVM Version: {}", llvm_version),
            )));
        }
    };

    Ok(parsed_version.to_string())
}

fn check_rust_installation(nightly_version: &str) {
    match std::process::Command::new("rustup")
        .args(["toolchain", "list"])
        .stdout(Stdio::piped())
        .output()
    {
        Ok(child_output) => {
            println!("rustup found.");
            let result = String::from_utf8_lossy(&child_output.stdout);
            if !result.contains(nightly_version) {
                println!("nightly toolchain not found");
                install_rust_nightly(nightly_version);
            } else {
                println!("nightly toolchain found.");
            }
        }
        Err(e) => {
            println!("Error: {}", e);
            install_rustup();
        }
    }
}
