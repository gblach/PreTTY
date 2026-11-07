//! `cargo setup`: for every patches/<crate>.diff, rebuilds vendor/<crate> from
//! the published crate with that patch applied. The main Cargo.toml points at
//! those folders through [patch.crates-io], so they must exist before any build.

use std::fs;
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::Result;

#[derive(Deserialize)]
struct Lockfile {
    package: Vec<LockedPackage>,
}

#[derive(Deserialize)]
struct LockedPackage {
    name: String,
    version: String,
}

#[derive(Deserialize)]
struct IndexEntry {
    name: String,
    vers: String,
    cksum: String,
}

fn locked_version(lockfile: &Lockfile, name: &str) -> Result<String> {
    lockfile
        .package
        .iter()
        .find(|package| package.name == name)
        .map(|package| package.version.clone())
        .ok_or_else(|| format!("{name} not found in Cargo.lock").into())
}

fn registry_dir() -> Result<PathBuf> {
    if let Ok(cargo_home) = std::env::var("CARGO_HOME") {
        return Ok(Path::new(&cargo_home).join("registry"));
    }
    Ok(Path::new(&std::env::var("HOME")?).join(".cargo/registry"))
}

/// Files named `name` in each `<registry>/<kind>/<source>/<relative>` folder;
/// there is one source folder per registry Cargo has used.
fn registry_files(registry: &Path, kind: &str, relative: &str, name: &str) -> Vec<PathBuf> {
    let Ok(sources) = fs::read_dir(registry.join(kind)) else {
        return Vec::new();
    };
    sources
        .flatten()
        .map(|source| source.path().join(relative).join(name))
        .filter(|path| path.is_file())
        .collect()
}

/// Finds the checksum in crates.io index data: one JSON entry per version,
/// separated by newlines (online) or NUL bytes (Cargo's local cache, which
/// also has a binary header that never parses as an entry).
fn index_checksum(index: &[u8], name: &str, version: &str) -> Option<String> {
    index
        .split(|byte| *byte == b'\n' || *byte == 0)
        .filter_map(|record| serde_json::from_slice::<IndexEntry>(record).ok())
        .find(|entry| entry.name == name && entry.vers == version)
        .map(|entry| entry.cksum)
}

fn download(url: &str) -> Result<Vec<u8>> {
    Ok(ureq::get(url).call()?.body_mut().read_to_vec()?)
}

/// The index splits names of four or more characters into two-letter
/// folders: gp/ui/gpui-component. Shorter names use other layouts.
fn index_dir(name: &str) -> Result<String> {
    match (name.get(..2), name.get(2..4)) {
        (Some(first), Some(second)) => Ok(format!("{first}/{second}")),
        _ => Err(format!("{name}: crate names under 4 characters are not supported").into()),
    }
}

/// Cargo.lock has no checksum to offer: with [patch] the entry points at
/// vendor/. Cargo's local index copy works offline; the online index otherwise.
fn crates_io_checksum(registry: &Path, name: &str, version: &str) -> Result<String> {
    let dir = index_dir(name)?;
    for cached in registry_files(registry, "index", &format!(".cache/{dir}"), name) {
        if let Some(checksum) = index_checksum(&fs::read(cached)?, name, version) {
            return Ok(checksum);
        }
    }
    let online = download(&format!("https://index.crates.io/{dir}/{name}"))?;
    index_checksum(&online, name, version)
        .ok_or_else(|| format!("no crates.io checksum found for {name} {version}").into())
}

fn crate_archive(registry: &Path, name: &str, version: &str) -> Result<Vec<u8>> {
    let file_name = format!("{name}-{version}.crate");
    if let Some(cached) = registry_files(registry, "cache", "", &file_name).first() {
        return Ok(fs::read(cached)?);
    }
    download(&format!(
        "https://static.crates.io/crates/{name}/{file_name}"
    ))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn unpack(archive: &[u8], dest: &Path) -> Result<()> {
    let mut tar = tar::Archive::new(GzDecoder::new(archive));
    for entry in tar.entries()? {
        let mut entry = entry?;
        // Every path starts with `<crate>-<version>/`; drop that folder.
        let path = entry.path()?.components().skip(1).collect::<PathBuf>();
        if path.as_os_str().is_empty() {
            continue;
        }
        let target = dest.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        entry.unpack(&target)?;
    }
    Ok(())
}

/// Applies a multi-file unified diff (`diff -ru a/... b/...` output). diffy
/// parses one file at a time, so the text is cut at each `--- ` header.
fn apply_patch(patch: &str, dest: &Path) -> Result<()> {
    let mut starts: Vec<usize> = patch
        .match_indices("\n--- ")
        .map(|(index, _)| index + 1)
        .collect();
    if patch.starts_with("--- ") {
        starts.insert(0, 0);
    }
    starts.push(patch.len());
    for range in starts.windows(2) {
        let file_patch = &patch[range[0]..range[1]];
        // Drop the `diff -ru ...` line that precedes the next file's header.
        let file_patch = match file_patch.rfind("\ndiff ") {
            Some(end) => &file_patch[..end + 1],
            None => file_patch,
        };
        let parsed = diffy::Patch::from_str(file_patch)?;
        let target = parsed
            .modified()
            .and_then(|path| path.strip_prefix("b/"))
            .ok_or("patch header lacks a `+++ b/<path>` line")?;
        let path = dest.join(target);
        let patched = diffy::apply(&fs::read_to_string(&path)?, &parsed)
            .map_err(|err| format!("{target}: {err}"))?;
        fs::write(&path, patched)?;
    }
    Ok(())
}

fn vendor_crate(root: &Path, registry: &Path, name: &str, version: &str) -> Result<()> {
    let expected = crates_io_checksum(registry, name, version)?;
    let archive = crate_archive(registry, name, version)?;
    let actual = sha256_hex(&archive);
    if actual != expected {
        return Err(format!("{name} {version}: checksum {actual}, expected {expected}").into());
    }

    let dest = root.join("vendor").join(name);
    if dest.exists() {
        fs::remove_dir_all(&dest)?;
    }
    fs::create_dir_all(&dest)?;
    unpack(&archive, &dest)?;
    let patch = fs::read_to_string(root.join("patches").join(format!("{name}.diff")))?;
    apply_patch(&patch, &dest).map_err(|err| format!("{name}: {err}"))?;

    println!("vendor/{name} ready ({name} {version} + patches)");
    Ok(())
}

/// Crate names that have a patches/<crate>.diff file, sorted.
fn patched_crates(root: &Path) -> Result<Vec<String>> {
    let mut names: Vec<String> = fs::read_dir(root.join("patches"))?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let is_patch = path.extension().is_some_and(|ext| ext == "diff");
            is_patch.then(|| path.file_stem()?.to_str().map(String::from))?
        })
        .collect();
    names.sort();
    Ok(names)
}

pub fn setup(root: &Path) -> Result<()> {
    let registry = registry_dir()?;
    let lockfile: Lockfile = toml::from_str(&fs::read_to_string(root.join("Cargo.lock"))?)?;
    for name in patched_crates(root)? {
        let version = locked_version(&lockfile, &name)?;
        vendor_crate(root, &registry, &name, &version)?;
    }
    Ok(())
}
