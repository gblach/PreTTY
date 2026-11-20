//! `cargo rpm`: runs `cargo dist` if its binaries are out of date, then
//! packs each binary into an RPM v6 package for Fedora.

use std::path::Path;

use crate::Result;
use crate::dist::{DIST_DIR, DIST_TARGETS, dist_binary, dist_if_stale};
use crate::package::{
    BINARY, LICENSE_PATH, PACKAGE_RELEASE, Package, license_text, min_glibc, package_files,
    target_arch,
};

const RPM_REQUIRES: [&str; 7] = [
    "fontconfig",
    "libgcc",
    "libwayland-client",
    "libxcb",
    "libxkbcommon",
    "libxkbcommon-x11",
    "vulkan-loader",
];

pub fn build(root: &Path, package: &Package) -> Result<()> {
    dist_if_stale(root, package)?;
    let name = BINARY;
    let version = &package.version;
    for target in DIST_TARGETS {
        let arch = target_arch(target);
        let glibc = min_glibc(&dist_binary(root, package, target))?;
        let mut builder =
            rpm::PackageBuilder::new(name, version, &package.license, arch, &package.description);
        builder
            .using_config(rpm::BuildConfig::v6())
            .release(PACKAGE_RELEASE)
            .description(&package.description)
            .packager(package.maintainer()?)
            .requires(rpm::Dependency::greater_eq("glibc", glibc));
        for dep in RPM_REQUIRES {
            builder.requires(rpm::Dependency::any(dep));
        }
        for (path, data, mode) in package_files(root, package, target)? {
            let options = rpm::FileOptions::new(format!("/{path}")).permissions(mode as u16);
            builder.with_file_contents(data, options)?;
        }
        let options = rpm::FileOptions::new(format!("/{LICENSE_PATH}"))
            .permissions(0o644)
            .license();
        builder.with_file_contents(license_text(root)?, options)?;
        let out = root
            .join(DIST_DIR)
            .join(format!("{name}-{version}-{PACKAGE_RELEASE}.{arch}.rpm"));
        builder.build()?.write_file(&out)?;
        println!("{} ready", out.display());
    }
    Ok(())
}
