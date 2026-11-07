use std::fs::{self, File};
use std::path::PathBuf;

/// Ordered by priority, the highest last.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum SessionKind {
    #[default]
    Local,
    Root,
    Ssh,
    Container,
}

const ENGINE_SUBCOMMANDS: &[&str] = &["attach", "exec", "run", "start"];
const KUBERNETES_SUBCOMMANDS: &[&str] = &["attach", "debug", "exec", "run"];
const LXC_SUBCOMMANDS: &[&str] = &["console", "exec", "shell"];
/// Container tools and the subcommands that put the terminal inside a container, so `podman ps`
/// or `podman build` don't count. An empty list means every invocation does.
const CONTAINER_COMMANDS: [(&str, &[&str]); 11] = [
    ("distrobox", &["enter"]),
    ("docker", ENGINE_SUBCOMMANDS),
    ("incus", LXC_SUBCOMMANDS),
    ("kubectl", KUBERNETES_SUBCOMMANDS),
    ("lxc", LXC_SUBCOMMANDS),
    ("machinectl", &["login", "shell"]),
    ("nerdctl", ENGINE_SUBCOMMANDS),
    ("oc", KUBERNETES_SUBCOMMANDS),
    ("podman", ENGINE_SUBCOMMANDS),
    ("systemd-nspawn", &[]),
    ("toolbox", &["enter", "run"]),
];

fn read_proc(pid: u32, file: &str) -> Option<String> {
    fs::read_to_string(format!("/proc/{pid}/{file}")).ok()
}

/// The effective uid, the second field of the `Uid:` line, is what setuid
/// programs like sudo and su switch to.
fn effective_uid(status: &str) -> Option<u32> {
    let line = status.lines().find(|line| line.starts_with("Uid:"))?;
    line.split_whitespace().nth(2)?.parse().ok()
}

/// `cmdline` is the NUL-separated argument list from /proc. The first argument is skipped, but
/// for a script such as distrobox it is the interpreter, and the script path that follows matches
/// no subcommand.
fn is_container_command(comm: &str, cmdline: &str) -> bool {
    let Some((_, subcommands)) = CONTAINER_COMMANDS.iter().find(|(name, _)| *name == comm) else {
        return false;
    };
    subcommands.is_empty()
        || cmdline
            .split('\0')
            .skip(1)
            .any(|arg| subcommands.contains(&arg))
}

fn children(pid: u32) -> Vec<u32> {
    let Ok(tasks) = fs::read_dir(format!("/proc/{pid}/task")) else {
        return Vec::new();
    };
    tasks
        .flatten()
        .filter_map(|task| fs::read_to_string(task.path().join("children")).ok())
        .flat_map(|list| {
            list.split_whitespace()
                .filter_map(|pid| pid.parse().ok())
                .collect::<Vec<u32>>()
        })
        .collect()
}

pub fn working_directory(pid: u32) -> Option<PathBuf> {
    fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

pub fn shell_in_foreground(pty: &File, shell_pid: u32) -> bool {
    rustix::termios::tcgetpgrp(pty)
        .is_ok_and(|group| group.as_raw_nonzero().get() as u32 == shell_pid)
}

/// Classifies what runs in the foreground of a PTY. While the shell itself is
/// in the foreground the session is local, so background jobs are ignored.
/// Otherwise the whole process tree of the foreground job is searched, because
/// sudo runs its command on a separate PTY that would hide e.g. an ssh client.
/// A container wins over ssh and ssh over root, so `sudo podman exec` counts
/// as a container.
pub fn detect(pty: &File, shell_pid: u32) -> SessionKind {
    let Ok(group) = rustix::termios::tcgetpgrp(pty) else {
        return SessionKind::Local;
    };
    let leader = group.as_raw_nonzero().get() as u32;
    if leader == shell_pid {
        return SessionKind::Local;
    }

    let mut session = SessionKind::Local;
    let mut pending = vec![leader];
    while let Some(pid) = pending.pop() {
        let comm = read_proc(pid, "comm").unwrap_or_default();
        let comm = comm.trim_end();
        if read_proc(pid, "cmdline").is_some_and(|args| is_container_command(comm, &args)) {
            return SessionKind::Container;
        }
        if comm == "ssh" {
            session = SessionKind::Ssh;
        } else if read_proc(pid, "status").and_then(|status| effective_uid(&status)) == Some(0) {
            session = session.max(SessionKind::Root);
        }
        pending.extend(children(pid));
    }
    session
}

#[cfg(test)]
mod tests {
    use super::{effective_uid, is_container_command, working_directory};

    #[test]
    fn detects_container_commands() {
        assert!(is_container_command(
            "podman",
            "podman\0exec\0-it\0box\0bash\0"
        ));
        assert!(is_container_command(
            "docker",
            "docker\0container\0run\0-it\0debian\0"
        ));
        assert!(is_container_command(
            "distrobox",
            "/bin/sh\0/usr/bin/distrobox\0enter\0"
        ));
        assert!(is_container_command(
            "systemd-nspawn",
            "systemd-nspawn\0-D\0/srv/box\0"
        ));
        assert!(!is_container_command("podman", "podman\0ps\0"));
        assert!(!is_container_command("bash", "bash\0exec\0"));
    }

    #[test]
    fn reads_effective_uid() {
        let status = "Name:\tsudo\nUid:\t1000\t0\t0\t0\nGid:\t1000\t1000\t1000\t1000\n";
        assert_eq!(effective_uid(status), Some(0));
    }

    #[test]
    fn reads_working_directory() {
        let expected = std::env::current_dir().ok();
        assert_eq!(working_directory(std::process::id()), expected);
    }
}
