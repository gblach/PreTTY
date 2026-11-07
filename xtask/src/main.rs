//! Project tasks run through the aliases in .cargo/config.toml: `cargo setup`,
//! `cargo dist`, `cargo deb`, `cargo rpm` and `cargo pacman`. This crate stays outside the main
//! package so it builds without vendor/.

mod deb;
mod dist;
mod package;
mod pacman;
mod rpm;
mod setup;

use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

use package::Package;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("xtask must live inside the project")?;
    match std::env::args().nth(1).as_deref() {
        Some("setup") => setup::setup(root),
        Some("dist") => dist::dist(root, &Package::read(root)?),
        Some("deb") => deb::build(root, &Package::read(root)?),
        // `crate::` because the rpm crate has the same name.
        Some("rpm") => crate::rpm::build(root, &Package::read(root)?),
        Some("pacman") => pacman::build(root, &Package::read(root)?),
        _ => Err("usage: cargo setup | cargo dist | cargo deb | cargo rpm | cargo pacman".into()),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
