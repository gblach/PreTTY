//! `cargo deb`: runs `cargo dist`, then packs each binary into a .deb for Debian / Ubuntu.

use std::fs;
use std::path::Path;

use flate2::Compression;
use flate2::write::GzEncoder;

use crate::Result;
use crate::dist::{DIST_DIR, DIST_TARGETS, dist};
use crate::package::{
    BINARY, MIN_GLIBC, PACKAGE_RELEASE, Package, append_files, package_files, target_arch,
    unix_time,
};

// Runtime libraries: the binary links the first ones and loads Vulkan and Wayland at runtime.
const DEB_DEPENDS: [&str; 7] = [
    "libfontconfig1",
    "libgcc-s1",
    "libvulkan1",
    "libwayland-client0",
    "libxcb1",
    "libxkbcommon-x11-0",
    "libxkbcommon0",
];

fn tar_gz(files: &[(String, Vec<u8>, u32)], dirs: bool, mtime: u64) -> Result<Vec<u8>> {
    let mut tar = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::best()));
    append_files(&mut tar, files, dirs, mtime)?;
    Ok(tar.into_inner()?.finish()?)
}

fn deb_arch(target: &str) -> Result<&'static str> {
    match target_arch(target) {
        "aarch64" => Ok("arm64"),
        "x86_64" => Ok("amd64"),
        _ => Err(format!("{target}: no Debian architecture").into()),
    }
}

pub fn build(root: &Path, package: &Package) -> Result<()> {
    dist(root, package)?;
    let name = BINARY;
    let version = &package.version;
    let mtime = unix_time()?;
    for target in DIST_TARGETS {
        let arch = deb_arch(target)?;
        let files = package_files(root, package, target)?;
        let installed_kib = files.iter().map(|(_, data, _)| data.len()).sum::<usize>() / 1024 + 1;
        let control = format!(
            "Package: {name}\n\
             Version: {version}-{PACKAGE_RELEASE}\n\
             Architecture: {arch}\n\
             Maintainer: {}\n\
             Installed-Size: {installed_kib}\n\
             Depends: libc6 (>= {MIN_GLIBC}), {}\n\
             Section: x11\n\
             Priority: optional\n\
             Description: {}\n",
            package.maintainer()?,
            DEB_DEPENDS.join(", "),
            package.description,
        );
        let control = tar_gz(
            &[("control".into(), control.into_bytes(), 0o644)],
            false,
            mtime,
        )?;
        let data = tar_gz(&files, true, mtime)?;

        let out = root
            .join(DIST_DIR)
            .join(format!("{name}_{version}-{PACKAGE_RELEASE}_{arch}.deb"));
        // dpkg requires these three members in this order.
        let mut archive = ar::Builder::new(fs::File::create(&out)?);
        let members: [(&str, &[u8]); 3] = [
            ("debian-binary", b"2.0\n"),
            ("control.tar.gz", &control),
            ("data.tar.gz", &data),
        ];
        for (member, contents) in members {
            let mut header = ar::Header::new(member.into(), contents.len() as u64);
            header.set_mode(0o100644);
            header.set_mtime(mtime);
            archive.append(&header, contents)?;
        }
        println!("{} ready", out.display());
    }
    Ok(())
}
