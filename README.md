<div align="center">

# rayshot

**Lightshot-style screenshots for GNOME on Wayland.**<br>
Press the key, the screen freezes at once, drag a region, mark it up, press Enter.

[![AUR](https://img.shields.io/aur/version/rayshot?label=AUR&color=1793d1)](https://aur.archlinux.org/packages/rayshot)
[![CI](https://github.com/paranoica/rayshot/actions/workflows/ci.yml/badge.svg)](https://github.com/paranoica/rayshot/actions/workflows/ci.yml)
[![GNOME 45–51](https://img.shields.io/badge/GNOME-45%E2%80%9351-4a86cf)](#supported-systems)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

**English** · [Русский](README.ru.md)

</div>

rayshot is a small screenshot and annotation tool written in Rust. A GNOME
Shell extension freezes the screen on the frame you pressed the key on, so
menus, tooltips and video are captured exactly as they were, and the editor
opens on top of that picture in about 40 ms. No portal dialog, no flash.

- **Draw on it:** pen, line, arrow, rectangle, ellipse, numbered markers, highlighter, text, pixelate, blur, colour picker.
- **Change your mind:** pick any shape up again to move, resize, recolour, duplicate or delete it.
- **Copy text:** `Ctrl+Shift+C` reads the selection with Tesseract, on your machine.
- **Several monitors:** select across them; the last selection is remembered.

<div align="center">
<img src="assets/demo.gif" width="720" alt="Selecting a region, then drawing an arrow and a rectangle in the rayshot editor">
</div>

## Install

**Arch Linux:** `paru -S rayshot` (or `rayshot-git`). The package installs the
program and the GNOME extension. GNOME only loads new extensions at login, so
log out and back in, then run `rayshot install-hotkey Print` once.

**Other distributions:** take the archive from the
[releases page](https://github.com/paranoica/rayshot/releases). The x86_64
binary is built on Debian 12 and runs with glibc 2.36 or newer. Unpack it
somewhere permanent, run `gnome-extension/install.sh`, log out and back in,
then `./rayshot install-hotkey Print`.

**From source:** `cargo build --release`, then the same two steps with
`./target/release/rayshot`. A copy of the extension in `~/.local/share` wins
over a packaged one, so run `gnome-extension/uninstall.sh` before switching to
the package.

rayshot needs `wl-clipboard` and a Vulkan driver, plus `xdg-desktop-portal`
for the portal mode on other desktops. Copying text is optional and needs
Tesseract with its language data: `tesseract`, `tesseract-data-eng` and
`tesseract-data-rus` on Arch, `tesseract-ocr` and `tesseract-ocr-rus` on Debian
and Ubuntu, `tesseract` and `tesseract-langpack-rus` on Fedora.

## Usage

| Key | Tool | | Key | Action |
|---|---|---|---|---|
| `S` | select, move, resize | | `Enter` / `Ctrl+C` | copy |
| `P` | pen | | `Ctrl+Shift+C` | copy the text |
| `L` | line | | `Ctrl+S` | save to your pictures folder |
| `A` | arrow | | `Ctrl+Shift+S` | save as… |
| `R` | rectangle | | `Ctrl+Z` / `Ctrl+Y` | undo / redo |
| `E` | ellipse | | `Esc` | cancel |
| `N` | numbered markers | | mouse wheel | line width |
| `M` | highlighter | | `1`–`9` | colours |
| `T` | text | | | |
| `X` / `B` | pixelate / blur | | | |
| `I` | colour picker, copies `#RRGGBB` | | | |

Hold **Shift** while drawing for straight lines at 45° steps, squares and circles.

**Editing what you drew.** With `S`, click a shape: drag it from anywhere in
its dashed frame, or drag a round handle to resize it. While it is selected,
colours and the wheel change that shape, `Delete` removes it, the arrow keys
nudge it, `Ctrl+D` duplicates it and `Esc` lets go. Double-click text to edit it.

**Copying text** closes the editor at once and reads the selection in the
background; a notice shows the first line when it is on the clipboard. Your
drawings are left out, but blur and pixelate are applied first, so hidden text
stays hidden.

The gear on the toolbar picks which tools and buttons are shown; hidden ones
keep their keys. Colour, width and that choice are remembered.

**Where files go.** Copying keeps a temporary copy in `$XDG_RUNTIME_DIR/rayshot/`
(in RAM, pruned after a day). Saving writes `rayshot-DD-MM-YYYY-HH-MM-SS.png`
to your pictures folder (`XDG_PICTURES_DIR`).

<details>
<summary>Commands and environment variables</summary>

| Command | |
|---|---|
| `rayshot install-hotkey [key]` | set the hotkey (`Print`); until then the extension listens on `Ctrl+Print` |
| `rayshot uninstall-hotkey` | turn the extension off and give `Print` back to GNOME |
| `rayshot warm [on\|off\|status]` | keep the warm editor process, or not |
| `rayshot ocr --print <png>` | print the text Tesseract reads in a PNG |
| `rayshot` | portal mode, for desktops without the extension |

`RAYSHOT_NO_REMEMBER_SELECTION` starts every capture without the previous
selection; `RAYSHOT_BIN` points the extension at a rayshot binary.

</details>

## Speed

On a GTX 1060, GNOME 51 and two 1080p monitors, from the keypress:

| | median |
|---|---|
| Screen frozen | 1.7 ms |
| Editor on screen | 38 ms (144 ms with `rayshot warm off`) |
| Text copied (`Ctrl+Shift+C`, a paragraph) | usually under a second |

While idle the warm editor costs about 0.2 % of one core, 24 MB of its own
memory and 196 MB of video memory; `rayshot warm off` frees all of it.

## How it works

The extension asks the compositor for the frame it is showing (the same call
GNOME's own screenshot UI uses) and lays it over everything, so keys and early
clicks land on the freeze instead of the apps below. A warm `rayshot serve`
process waits with its GPU ready; its transparent windows are revealed through
the frozen picture after their first frame, so nothing flashes. Each process
serves one capture and the next one starts right away. If it is missing or
crashes, the extension starts a fresh one for that capture.

## Supported systems

GNOME Shell 45 to 51 on Wayland. Every one of those versions runs the tests in
CI: Fedora 39, Ubuntu 24.04, Fedora 41, Debian 13, Fedora 43, Ubuntu 26.04
and Arch. GNOME 44 and older cannot load the extension. Other desktops can
bind a key to `rayshot` and capture through the screenshot portal, without the
freeze; that path is not tested. Scaling at 100 %, 150 % and 200 %, mixed
scaling and rotated monitors are tested on virtual monitors; real hardware
testing is on NVIDIA at 100 %.

## Compared with others

GNOME's own screenshot UI freezes the screen the same way but cannot draw.
Flameshot and Ksnip can draw, but on GNOME Wayland they capture through the
portal; Flameshot users report permission prompts and delays, and Ksnip's
global hotkeys do not work there. Lightshot, ShareX and CleanShot X are the closest in spirit, on
Windows and macOS. rayshot never uploads anything. The full tables, with
sources, are in [docs/comparison.md](docs/comparison.md).

## Testing

146 unit tests in Rust and five suites for the extension. A soak test runs a
headless GNOME Shell with a real rayshot through 100 warm and 20 cold captures
and 16 scenarios (crashes, monitors unplugged, sleep, lock, copy failures,
OCR) and fails on any leak. A second suite drags real selections with a
virtual pointer at 100 %, 150 % and 200 %, on mixed scaling, rotated, stacked
and side-by-side monitors of different sizes, an ultrawide and a fullscreen
app, and checks the size of the copied PNG and the edges of coloured squares
in it to a few pixels. CI runs both on every GNOME
version above.

```sh
gnome-extension/tests/soak.sh 100 20
gnome-extension/tests/displays.sh
```

## Troubleshooting

| Symptom | Fix |
|---|---|
| The hotkey does nothing after an install or update | log out and back in; GNOME on Wayland cannot reload extensions. |
| `Print` still opens GNOME's screenshot UI | run `rayshot install-hotkey Print` once. |
| "rayshot keeps failing to prepare its overlay" | see `journalctl --user -b \| grep rayshot`, then `rayshot warm off` and `rayshot warm on`. |
| The screen freezes but no editor appears | the GPU may be out of video memory; a screen recorder with a replay buffer is a common cause. |
| The clipboard is empty after Enter | `wl-copy` did not get the focus in time; a notice says where the temporary copy is. |
| "Install tesseract to copy text" | install Tesseract and its language data, see [Install](#install). |
| `Print` does nothing after removing the package | `gsettings reset org.gnome.shell.keybindings show-screenshot-ui` |
| "rayshot was updated. Log out and back in…" | the package was upgraded while the old extension is still loaded: log out and back in. |
| "rayshot was updated. Run rayshot install-hotkey Print…" | you came from the 0.1 daemon: run `rayshot install-hotkey Print` once. |
| "…do not match" or "…is older than its GNOME extension" | update rayshot, then log out and back in, so both come from the same release. |
| "The rayshot program was not found" | install rayshot, or point the extension at it with `RAYSHOT_BIN`. |

## License

MIT. See [LICENSE](LICENSE).
