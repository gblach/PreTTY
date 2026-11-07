//! `cargo pacman`: runs `cargo dist`, then packs each binary into a pacman package for Arch Linux.

use std::fs;
use std::io::Write;
use std::path::Path;

use flate2::Compression;
use flate2::write::GzEncoder;

use crate::Result;
use crate::dist::{DIST_DIR, DIST_TARGETS, dist};
use crate::package::{
    BINARY, MIN_GLIBC, PACKAGE_RELEASE, Package, append_files, package_files, parent_dirs,
    target_arch, unix_time,
};
use crate::setup::sha256_hex;

const PACMAN_DEPENDS: [&str; 7] = [
    "fontconfig",
    "gcc-libs",
    "libxcb",
    "libxkbcommon",
    "libxkbcommon-x11",
    "vulkan-icd-loader",
    "wayland",
];
// The default level of makepkg.
const ZSTD_LEVEL: i32 = 19;

/// Gzipped mtree listing of `.PKGINFO`, `files` and their parent folders, which
/// `pacman -Qkk` checks the installed files against.
fn mtree(pkginfo: &[u8], files: &[(String, Vec<u8>, u32)], mtime: u64) -> Result<Vec<u8>> {
    let owner = format!("uid=0 gid=0 time={mtime}.0");
    let mut text = String::from("#mtree\n");
    for dir in parent_dirs(files) {
        text.push_str(&format!("./{} type=dir mode=755 {owner}\n", dir.display()));
    }
    let entries = [(".PKGINFO", pkginfo, 0o644)].into_iter().chain(
        files
            .iter()
            .map(|(path, data, mode)| (path.as_str(), &data[..], *mode)),
    );
    for (path, data, mode) in entries {
        text.push_str(&format!(
            "./{path} type=file mode={mode:o} {owner} size={} sha256digest={}\n",
            data.len(),
            sha256_hex(data),
        ));
    }
    let mut gz = GzEncoder::new(Vec::new(), Compression::best());
    gz.write_all(text.as_bytes())?;
    Ok(gz.finish()?)
}

pub fn build(root: &Path, package: &Package) -> Result<()> {
    dist(root, package)?;
    let name = BINARY;
    let version = &package.version;
    let mtime = unix_time()?;
    for target in DIST_TARGETS {
        let arch = target_arch(target);
        let files = package_files(root, package, target)?;
        let size = files.iter().map(|(_, data, _)| data.len()).sum::<usize>();
        let mut pkginfo = format!(
            "pkgname = {name}\n\
             pkgbase = {name}\n\
             pkgver = {version}-{PACKAGE_RELEASE}\n\
             pkgdesc = {}\n\
             builddate = {mtime}\n\
             packager = {}\n\
             size = {size}\n\
             arch = {arch}\n\
             license = {}\n\
             depend = glibc>={MIN_GLIBC}\n",
            package.description,
            package.maintainer()?,
            package.license,
        );
        for dep in PACMAN_DEPENDS {
            pkginfo.push_str(&format!("depend = {dep}\n"));
        }

        let encoder = zstd::Encoder::new(Vec::new(), ZSTD_LEVEL)?;
        let mut tar = tar::Builder::new(encoder);
        // pacman reads the metadata from the start of the archive.
        let metadata = [
            (
                ".MTREE".to_string(),
                mtree(pkginfo.as_bytes(), &files, mtime)?,
                0o644,
            ),
            (".PKGINFO".to_string(), pkginfo.into_bytes(), 0o644),
        ];
        append_files(&mut tar, &metadata, false, mtime)?;
        append_files(&mut tar, &files, true, mtime)?;
        let data = tar.into_inner()?.finish()?;

        let out = root.join(DIST_DIR).join(format!(
            "{name}-{version}-{PACKAGE_RELEASE}-{arch}.pkg.tar.zst"
        ));
        fs::write(&out, data)?;
        println!("{} ready", out.display());
    }
    Ok(())
}
