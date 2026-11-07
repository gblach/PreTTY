# PreTTY

A pretty terminal emulator for Linux, clean, GPU-rendered and easy on the eyes, that always
shows where your shell is: over SSH, as root or inside a container.

## Features

- Tabs: open, close, reorder by dragging, switch with `Ctrl+Tab` or `Alt+1` to `Alt+0`; a new
  tab starts in the working directory of the active one
- Tabs show the title the running program sets; the window title is
  `<tab title> - PreTTY [<tab>/<tabs>]`
- The background color shows where the shell's foreground command runs:
  - blue: an SSH session
  - red: root, e.g. through `sudo` or `su`
  - green: a container, e.g. `podman exec`, `docker run`, `distrobox enter`, `toolbox enter`,
    `kubectl exec`, `incus shell` or `systemd-nspawn`

  A container wins over SSH and SSH over root, so `sudo podman exec` shows green.
- Optional zebra stripes (every other row shaded) and a background opacity setting
- Clickable `http://` and `https://` links, opened in the default browser
- Mouse selection: drag, double click for a word, triple click for a line; dragging past the
  top or bottom edge scrolls, faster the further away the pointer is
- Clipboard: right click copies the selection and clears it; a finished selection also becomes
  the primary selection, which middle click and `Shift+Insert` paste
- Scrollback with the mouse wheel and keyboard; in full-screen programs like `less` or `vim`
  the wheel sends arrow keys instead
- Visual bell: the terminal flashes, and a background tab that rang is marked
- Without the title bar, dragging the tab bar moves the window and right clicking it opens the
  window menu; when the desktop does not draw a title bar, PreTTY draws its own

## Screenshots

With zebra stripes:

![PreTTY with zebra stripes](assets/screenshots/zebra-stripes.jxl)

Without zebra stripes:

![PreTTY without zebra stripes](assets/screenshots/no-stripes.jxl)

During an SSH session the background turns blue:

![PreTTY in an SSH session](assets/screenshots/ssh-session.jxl)

Inside a container the background turns green:

![PreTTY inside a container](assets/screenshots/container-session.jxl)

As root the background turns red:

![PreTTY in a root session](assets/screenshots/root-session.jxl)

With a semi-transparent background the desktop shows through:

![PreTTY with a semi-transparent background](assets/screenshots/transparent.jxl)

## Install

On Fedora, install it from the
[gblachx/apps](https://copr.fedorainfracloud.org/coprs/gblachx/apps/) COPR repository,
which also keeps it updated:

```sh
sudo dnf copr enable gblachx/apps
sudo dnf install pretty
```

On other distributions, download the package for your distribution from the GitHub Releases
page and install it:

```sh
sudo dnf install ./pretty-<version>-1.x86_64.rpm           # Fedora
sudo apt install ./pretty_<version>-1_amd64.deb            # Debian / Ubuntu
sudo pacman -U ./pretty-<version>-1-x86_64.pkg.tar.zst     # Arch Linux
```

The packages are built for `x86_64` and `aarch64` and need glibc 2.41 or newer: Debian 13,
Ubuntu 25.04, Fedora 42, a current Arch Linux or later. They install the desktop entry and icon
too. PreTTY renders with Vulkan, so it also needs a Vulkan driver.

### From source

On other distributions, or to build it yourself, you need:

- Rust 1.95 or newer (install with [rustup](https://rustup.rs))
- `pkg-config` and the fontconfig / XCB / xkbcommon development files
- At runtime: a Vulkan driver and, for Wayland sessions, the Wayland client library

| Distribution    | Install command                                                                                                                           |
| --------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| Fedora          | `sudo dnf install pkgconf-pkg-config fontconfig-devel libxcb-devel libxkbcommon-devel libxkbcommon-x11-devel vulkan-loader wayland-devel` |
| Debian / Ubuntu | `sudo apt install pkg-config libfontconfig-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev libvulkan1 libwayland-client0`           |
| Arch / CachyOS  | `sudo pacman -S pkgconf fontconfig libxcb libxkbcommon libxkbcommon-x11 vulkan-icd-loader wayland`                                        |

PreTTY builds against patched copies of two dependencies (see
[Patched dependencies](#patched-dependencies)), so install it from a clone of this repository:

```sh
cargo setup              # creates the patched crates in vendor/
cargo install --path .   # installs the pretty binary to ~/.cargo/bin
```

The desktop entry runs `pretty` by name, so it needs `~/.cargo/bin` on your `PATH`. Install the
desktop entry and icon for the current user:

```sh
install -Dm644 assets/pretty.desktop ~/.local/share/applications/pretty.desktop
install -Dm644 assets/pretty.png ~/.local/share/icons/hicolor/512x512/apps/pretty.png
install -Dm644 assets/pretty.svg ~/.local/share/icons/hicolor/scalable/apps/pretty.svg
```

The window's app ID is `pretty`, matching `pretty.desktop`, so docks and task bars show PreTTY's
name and icon.

## Keyboard shortcuts

| Shortcut                                | Action                               |
| --------------------------------------- | ------------------------------------ |
| `Ctrl+Shift+T`                          | New tab                              |
| `Ctrl+Shift+W`                          | Close tab                            |
| `Ctrl+Tab` / `Ctrl+Shift+Tab`           | Next / previous tab                  |
| `Alt+1` to `Alt+0`                      | Go to tab 1 to 10                    |
| `Ctrl+Shift+Left` / `Ctrl+Shift+Right`  | Move tab left / right                |
| `Ctrl+Shift+C`                          | Copy the selection                   |
| `Ctrl+Shift+V`                          | Paste from the clipboard             |
| `Shift+Insert`                          | Paste the primary selection          |
| `Shift+PageUp` / `Shift+PageDown`       | Scroll one page up / down            |
| `Ctrl+Shift+Up` / `Ctrl+Shift+Down`     | Scroll one line up / down            |
| `Ctrl+Shift+Home` / `Ctrl+Shift+End`    | Scroll to the top / bottom           |
| `Ctrl+,`                                | Settings                             |
| `Ctrl+Shift+H`                          | About window, lists these shortcuts  |

## Configuration

Settings are edited in the app (the sliders button in the tab bar, or `Ctrl+,`) and saved
to `~/.config/pretty.toml`. The About window (`Ctrl+Shift+H`) lists all keyboard shortcuts.

| Setting                  | Default     | Meaning                                            |
| ------------------------ | ----------- | -------------------------------------------------- |
| `font.family`            | empty       | Font name; empty means the theme's monospace font  |
| `font.size`              | `14`        | Font size in pixels                                |
| `cursor.shape`           | `underline` | `underline`, `beam` or `block`                     |
| `cursor.blink`           | `true`      | Blink the cursor                                   |
| `window.columns`         | `100`       | Initial width in columns                           |
| `window.rows`            | `30`        | Initial height in rows                             |
| `window.maximized`       | `true`      | Open maximized                                     |
| `window.titlebar`        | `false`     | Show the window title bar                          |
| `background.opacity`     | `100`       | Background opacity in percent                      |
| `background.zebra`       | `false`     | Shade every other row                              |
| `background.visual_bell` | `true`      | Flash on a bell and mark background tabs that rang |

## Development

### Patched dependencies

`cargo setup` vendors every crate that has a file in `patches/`:

| Patch                          | Change                                                                                        |
| ------------------------------ | --------------------------------------------------------------------------------------------- |
| `patches/gpui-component.diff`  | Clickable controls (buttons, number steppers, select and its items) show the pointer cursor   |
| `patches/gpui-pre-wgpu.diff`   | Rendering starts only Vulkan; starting OpenGL too loaded Mesa's GL driver (about 35 MB of RSS) |

`Cargo.toml` points the dependencies at the patched copies through `[patch.crates-io]`; `vendor/`
is generated and not committed. PreTTY therefore needs a working Vulkan driver.

For each patched crate, `cargo setup` reads its version from `Cargo.lock`, takes the crate from
Cargo's download cache (or downloads it from crates.io), verifies its SHA-256 checksum and applies
the patch. It works offline when the crates are already in the cache.

Without `vendor/`, `cargo build` fails with
`` patch location `.../vendor/<crate>` does not contain packages matching `<crate>` ``;
run `cargo setup` to fix it.

After updating gpui-kit:

1. Bump the version in `Cargo.toml` and run `cargo update -p gpui-kit`.
2. Run `cargo setup` again. If a patch no longer applies, it stops with an error naming the crate
   and file; update that patch for the new version.

### Release binaries

`cargo dist` builds release binaries for `aarch64-unknown-linux-gnu` and
`x86_64-unknown-linux-gnu` and copies them to `target/dist/pretty-<version>-<target>`.
The builds run in a Debian 13 container defined by `Containerfile`, so the binaries need glibc 2.41
or newer. It needs Podman or Docker with its API socket: `DOCKER_HOST` if set, otherwise the
rootless Podman socket, otherwise `/var/run/docker.sock`. With Podman:

```sh
systemctl --user enable --now podman.socket
cargo setup      # the container builds against vendor/ too
cargo dist
```

`cargo rpm`, `cargo deb` and `cargo pacman` run `cargo dist`, then pack each binary with the
desktop entry and icons into packages in `target/dist`. All install the binary as
`/usr/bin/pretty`:

| Command        | Output                                                              | For             |
| -------------- | ------------------------------------------------------------------- | --------------- |
| `cargo rpm`    | `pretty-<version>-1.x86_64.rpm`, `...aarch64.rpm` (v6)              | Fedora          |
| `cargo deb`    | `pretty_<version>-1_amd64.deb`, `..._arm64.deb`                     | Debian / Ubuntu |
| `cargo pacman` | `pretty-<version>-1-x86_64.pkg.tar.zst`, `...aarch64.pkg.tar.zst`   | Arch / CachyOS  |

All packages require glibc 2.41 or newer, so Debian 13, Ubuntu 25.04, Fedora 42, a current Arch
Linux or later.
Attach the packages to a GitHub release, which the [Install](#install) section points to.

### Icon

`assets/pretty.png` and `assets/pretty-about.png` are rendered from `assets/pretty.svg`. The
About window uses the small one because the GPU downscales images without smoothing. After editing
the SVG, regenerate and losslessly optimize them; this needs `rsvg-convert` (from librsvg) and
[oxipng](https://github.com/oxipng/oxipng):

```sh
rsvg-convert -w 512 -h 512 assets/pretty.svg -o assets/pretty.png
rsvg-convert -w 144 -h 144 assets/pretty.svg -o assets/pretty-about.png
oxipng -o max -sz assets/pretty.png assets/pretty-about.png
```

## License

GPL-3.0-or-later, see [LICENSE](LICENSE).
