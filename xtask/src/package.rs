//! Contents and metadata shared by the .deb and RPM packages.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::SystemTime;

use object::Endianness;
use object::elf::FileHeader64;
use object::read::elf::FileHeader;
use serde::Deserialize;

use crate::Result;
use crate::dist::dist_binary;

pub const PACKAGE_RELEASE: &str = "1";
pub const BINARY: &str = "pretty";
/// Where Fedora and Arch expect the license text.
pub const LICENSE_PATH: &str = "usr/share/licenses/pretty/LICENSE";
/// Installed path, source path relative to the project root, permissions.
const ASSETS: [(&str, &str, u32); 3] = [
    (
        "usr/share/applications/pretty.desktop",
        "assets/pretty.desktop",
        0o644,
    ),
    (
        "usr/share/icons/hicolor/512x512/apps/pretty.png",
        "assets/pretty.png",
        0o644,
    ),
    (
        "usr/share/icons/hicolor/scalable/apps/pretty.svg",
        "assets/pretty.svg",
        0o644,
    ),
];

#[derive(Deserialize)]
struct Manifest {
    package: Package,
}

/// The `[package]` table of the project's Cargo.toml.
#[derive(Deserialize)]
pub struct Package {
    pub authors: Vec<String>,
    pub description: String,
    pub license: String,
    pub version: String,
}

impl Package {
    pub fn read(root: &Path) -> Result<Self> {
        let manifest: Manifest = toml::from_str(&fs::read_to_string(root.join("Cargo.toml"))?)?;
        Ok(manifest.package)
    }

    pub fn maintainer(&self) -> Result<&str> {
        self.authors
            .first()
            .map(String::as_str)
            .ok_or_else(|| "Cargo.toml needs `authors` for the package maintainer".into())
    }
}

/// CPU part of a target triple, which is also the RPM architecture: `x86_64`, `aarch64`.
pub fn target_arch(target: &str) -> &str {
    target.split('-').next().unwrap_or(target)
}

/// Highest `GLIBC_x.y` symbol version the binary needs, which is the oldest glibc it runs on.
pub fn min_glibc(binary: &Path) -> Result<String> {
    let data = fs::read(binary)?;
    let data = &data[..];
    let header = FileHeader64::<Endianness>::parse(data)?;
    let endian = header.endian()?;
    let sections = header.sections(endian, data)?;
    let mut newest: Option<Vec<u32>> = None;
    if let Some((mut needs, link)) = sections.gnu_verneed(endian, data)? {
        let strings = sections.strings(endian, data, link)?;
        while let Some((_, mut versions)) = needs.next()? {
            while let Some(version) = versions.next()? {
                let name = String::from_utf8_lossy(version.name(endian, strings)?);
                // Skips other libraries' versions and GLIBC_PRIVATE.
                let Some(Ok(number)) = name
                    .strip_prefix("GLIBC_")
                    .map(|number| number.split('.').map(str::parse).collect())
                else {
                    continue;
                };
                newest = newest.max(Some(number));
            }
        }
    }
    let newest = newest.ok_or_else(|| format!("{}: needs no glibc version", binary.display()))?;
    Ok(newest
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join("."))
}

/// Installed path without the leading `/`, contents and permissions of every packaged file.
pub fn package_files(
    root: &Path,
    package: &Package,
    target: &str,
) -> Result<Vec<(String, Vec<u8>, u32)>> {
    let mut files = vec![(
        format!("usr/bin/{BINARY}"),
        fs::read(dist_binary(root, package, target))?,
        0o755,
    )];
    for (path, source, mode) in ASSETS {
        files.push((path.into(), fs::read(root.join(source))?, mode));
    }
    Ok(files)
}

pub fn license_text(root: &Path) -> Result<Vec<u8>> {
    Ok(fs::read(root.join("LICENSE"))?)
}

pub fn unix_time() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs())
}

fn tar_entry<W: Write>(
    tar: &mut tar::Builder<W>,
    path: &str,
    data: &[u8],
    kind: tar::EntryType,
    mode: u32,
    mtime: u64,
) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(kind);
    header.set_mode(mode);
    header.set_mtime(mtime);
    header.set_size(data.len() as u64);
    tar.append_data(&mut header, path, data)?;
    Ok(())
}

pub fn parent_dirs(files: &[(String, Vec<u8>, u32)]) -> BTreeSet<&Path> {
    files
        .iter()
        .map(|(path, ..)| path)
        .flat_map(|path| Path::new(path).ancestors().skip(1))
        .filter(|dir| !dir.as_os_str().is_empty())
        .collect()
}

/// Appends `files` to `tar`. With `dirs`, also adds every parent folder first, since dpkg only
/// creates folders that the archive lists and pacman only removes folders it knows about.
pub fn append_files<W: Write>(
    tar: &mut tar::Builder<W>,
    files: &[(String, Vec<u8>, u32)],
    dirs: bool,
    mtime: u64,
) -> Result<()> {
    if dirs {
        for dir in parent_dirs(files) {
            let dir = format!("{}/", dir.display());
            tar_entry(tar, &dir, &[], tar::EntryType::Directory, 0o755, mtime)?;
        }
    }
    for (path, data, mode) in files {
        tar_entry(tar, path, data, tar::EntryType::Regular, *mode, mtime)?;
    }
    Ok(())
}
