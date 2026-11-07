//! `cargo dist`: builds release binaries inside the Containerfile's image,
//! talking to Podman or Docker through its API socket, and copies each to
//! `target/dist/pretty-<version>-<target>`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bollard::container::LogOutput;
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{
    BuildImageOptionsBuilder, CreateContainerOptions, LogsOptionsBuilder, RemoveContainerOptions,
    StartContainerOptions, WaitContainerOptions,
};
use bollard::{API_DEFAULT_VERSION, Docker, body_full};
use futures_util::StreamExt;

use crate::Result;
use crate::package::{BINARY, Package};

pub const DIST_TARGETS: [&str; 2] = ["aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"];
const DIST_IMAGE: &str = "pretty-dist";
pub const DIST_DIR: &str = "target/dist";
const CONTAINER_ROOT: &str = "/src";
// Separate from target/ so container builds don't invalidate host builds and vice versa.
const CONTAINER_TARGET_DIR: &str = "target/container";

/// DOCKER_HOST when set, otherwise the rootless Podman socket, otherwise Docker's.
fn connect_engine() -> Result<Docker> {
    // bollard applies this to every request, and a container wait only answers when the build
    // ends.
    let timeout = Duration::from_secs(4 * 60 * 60);
    if std::env::var_os("DOCKER_HOST").is_some() {
        let mut docker = Docker::connect_with_defaults()?;
        docker.set_timeout(timeout);
        return Ok(docker);
    }
    let podman = std::env::var_os("XDG_RUNTIME_DIR")
        .map(|dir| Path::new(&dir).join("podman/podman.sock"))
        .filter(|path| path.exists());
    let socket = podman.unwrap_or_else(|| PathBuf::from("/var/run/docker.sock"));
    let socket = socket.to_str().ok_or("socket path is not UTF-8")?;
    Ok(Docker::connect_with_socket(
        socket,
        timeout.as_secs(),
        API_DEFAULT_VERSION,
    )?)
}

async fn build_image(docker: &Docker, root: &Path) -> Result<()> {
    let mut context = tar::Builder::new(Vec::new());
    context.append_path_with_name(root.join("Containerfile"), "Containerfile")?;
    let options = BuildImageOptionsBuilder::default()
        .dockerfile("Containerfile")
        .t(DIST_IMAGE)
        .pull("true")
        .rm(true)
        .build();
    let context = body_full(context.into_inner()?.into());
    let mut progress = docker.build_image(options, None, Some(context));
    while let Some(info) = progress.next().await {
        let info = info?;
        if let Some(detail) = info.error_detail {
            return Err(detail.message.unwrap_or_default().into());
        }
        if let Some(line) = info.stream {
            print!("{line}");
        }
    }
    Ok(())
}

async fn run_container(docker: &Docker, id: &str) -> Result<()> {
    docker
        .start_container(id, None::<StartContainerOptions>)
        .await?;
    let options = LogsOptionsBuilder::default()
        .follow(true)
        .stdout(true)
        .stderr(true)
        .build();
    let mut logs = docker.logs(id, Some(options));
    while let Some(output) = logs.next().await {
        match output? {
            LogOutput::StdErr { message } => std::io::stderr().write_all(&message)?,
            other => std::io::stdout().write_all(&other.into_bytes())?,
        }
    }
    // bollard turns a non-zero exit code into an error.
    let mut wait = docker.wait_container(id, None::<WaitContainerOptions>);
    while let Some(status) = wait.next().await {
        status?;
    }
    Ok(())
}

async fn build_in_container(docker: &Docker, root: &Path, target: &str) -> Result<()> {
    let config = ContainerCreateBody {
        image: Some(DIST_IMAGE.into()),
        cmd: Some(
            ["cargo", "build", "--release", "--target", target]
                .map(String::from)
                .into(),
        ),
        env: Some(vec![format!(
            "CARGO_TARGET_DIR={CONTAINER_ROOT}/{CONTAINER_TARGET_DIR}"
        )]),
        working_dir: Some(CONTAINER_ROOT.into()),
        host_config: Some(HostConfig {
            binds: Some(vec![
                // `z` relabels the project for SELinux, which otherwise hides it from the
                // container.
                format!("{}:{CONTAINER_ROOT}:z", root.display()),
                // A named volume keeps downloaded crates between runs.
                format!("{DIST_IMAGE}-registry:/usr/local/cargo/registry"),
            ]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let id = docker
        .create_container(None::<CreateContainerOptions>, config)
        .await?
        .id;
    let result = run_container(docker, &id).await;
    docker
        .remove_container(&id, None::<RemoveContainerOptions>)
        .await?;
    result.map_err(|err| format!("build for {target} failed: {err}").into())
}

pub fn dist_binary(root: &Path, package: &Package, target: &str) -> PathBuf {
    root.join(DIST_DIR)
        .join(format!("{BINARY}-{}-{target}", package.version))
}

async fn build(root: &Path, package: &Package) -> Result<()> {
    let docker = connect_engine()?;
    build_image(&docker, root).await?;
    let dist_dir = root.join(DIST_DIR);
    fs::create_dir_all(&dist_dir)?;
    for target in DIST_TARGETS {
        build_in_container(&docker, root, target).await?;
        // cargo hard-links the binary to its own cache, so copying over a previous output that
        // still shares that file would write into the cache too; removing it first keeps them
        // apart.
        let out = dist_binary(root, package, target);
        if out.exists() {
            fs::remove_file(&out)?;
        }
        let built = root.join(CONTAINER_TARGET_DIR).join(target).join("release");
        fs::copy(built.join(BINARY), &out)?;
        println!("{} ready", out.display());
    }
    Ok(())
}

pub fn dist(root: &Path, package: &Package) -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(build(root, package))
}
