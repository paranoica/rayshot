[English](comparison.md) · [Русский](comparison.ru.md)

# rayshot compared

rayshot is about one thing: the Lightshot flow (key, frozen screen, drag,
draw, Enter) at native speed on GNOME Wayland. Other tools do more in other
directions, and the tables say where.

## On Linux

| | rayshot | GNOME screenshot UI | Flameshot | Ksnip | Spectacle | Gradia | Satty / Swappy |
|---|---|---|---|---|---|---|---|
| GNOME Wayland | yes | yes | experimental | yes, with limits | not documented | yes, through the portal | no, wlroots only |
| Still picture to select on | at the keypress, ≈2 ms | at the keypress | after a portal call | after a portal call | not documented on GNOME | GNOME's own UI | n/a |
| Arrows, text, shapes | yes | no | yes | yes | yes | yes | yes |
| Blur / pixelate | both | no | pixelate | yes | yes | censor tool | Satty both, Swappy blur |
| Upload, OCR | OCR on your machine, never uploads | no | Imgur | Imgur, OCR plugin | OCR | upload, OCR | no |
| Key to selection | ≈40 ms, measured | freezes at once, not measured | long delays reported on GNOME | not found | 1.5–2 s reported | not found | n/a |
| Memory | 24 MB own, 146 MB resident | part of the shell | 36 MB on a Raspberry Pi | not found | 67 MB on a Raspberry Pi | not found | not found |

<details>
<summary>Notes and sources</summary>

- **GNOME screenshot UI** freezes the screen with the same compositor call rayshot uses (GNOME Shell's `screenshot.js`), but it only crops; there is nothing to draw with.
- **Flameshot:** GNOME Wayland support is labelled experimental; it captures through the portal, and its capture window has to stay open for the clipboard ([README](https://github.com/flameshot-org/flameshot), [v14 notes](https://github.com/flameshot-org/flameshot/releases)). Users report a permission dialog and long delays on GNOME ([NixOS forum](https://discourse.nixos.org/t/looking-for-a-gnome-compatible-screenshot-annotation-tool/75964)); the Arch Wiki notes slow area selection in some versions ([wiki](https://wiki.archlinux.org/title/Flameshot)).
- **Ksnip:** global hotkeys do not work on Wayland, and the clipboard may need `QT_QPA_PLATFORM=xcb` on GNOME; one maintainer ([README](https://github.com/ksnip/ksnip)).
- **Spectacle:** KDE's tool; users report 1.5–2 s from PrtSc to the window ([KDE forum](https://discuss.kde.org/t/make-spectacle-launch-faster/38030)).
- **Shutter:** in maintenance mode with rudimentary Wayland support ([FAQ](https://shutter-project.org/faq-help/)).
- **Gradia:** a GTK4 app to beautify and redact screenshots, with OCR and upload ([repo](https://github.com/AlexanderVanhee/Gradia)).
- **Satty / Swappy:** annotators for `grim` output on wlroots compositors ([Satty](https://github.com/Satty-org/Satty), [Swappy](https://github.com/jtheoof/swappy)).
- The Flameshot and Spectacle memory figures come from a Raspberry Pi 4 test ([LinuxLinks](https://www.linuxlinks.com/raspberry-pi-4-chronicling-desktop-experience-screen-capture/)). No controlled benchmark of these tools has been published; "not found" means exactly that.

</details>

## Against the Windows and macOS classics

| | rayshot | Lightshot | ShareX | Snipping Tool | macOS ⇧⌘4 | CleanShot X | Shottr |
|---|---|---|---|---|---|---|---|
| Platform | Linux (GNOME) | Windows, macOS | Windows | Windows | macOS | macOS | macOS |
| Price | free, MIT | free | free, GPL | built in | built in | $35 | pay what you want |
| Freezes at the keypress | yes | not documented | not documented | dims the screen | not documented | not documented | optional |
| Draw before copying | yes | yes | yes | afterwards, in a window | in a thumbnail editor | yes | yes |
| Blur / pixelate | both | no | both | redact | no | redact | not found |
| Upload | never | public prnt.sc links | many services | no | no | cloud | S3 |
| OCR, scrolling capture | OCR | no | yes | OCR | no | yes | yes |
| Published speed | ≈40 ms, measured | not found | not found | not found | not found | not found | 17 ms to grab, 165 ms to show (vendor) |

<details>
<summary>Notes and sources</summary>

- **Lightshot** is where this flow comes from. Its uploads are public: anyone who guesses a `prnt.sc` link can open the image ([privacy policy](https://lightshot.app/en/privacy.html)). rayshot never uploads anything.
- **ShareX** is the most complete free tool on Windows: annotations, many upload targets, OCR, scrolling capture ([site](https://getsharex.com/), [region capture](https://getsharex.com/docs/region-capture)).
- **Snipping Tool** greys the screen, then opens the shot in a separate editor; OCR runs on the device ([Microsoft](https://support.microsoft.com/en-us/windows/use-snipping-tool-to-capture-screenshots-00246869-1843-655f-f220-97299b865f6b)).
- **macOS** edits through the thumbnail in the corner; holding Control copies to the clipboard ([Apple](https://support.apple.com/en-us/102646)).
- **CleanShot X:** $35 with a year of updates, on-device OCR, scrolling capture, cloud sharing ([pricing](https://cleanshot.com/pricing), [features](https://cleanshot.com/)).
- **Shottr** can freeze the screen for area capture since 1.9 and states 17 ms to grab and about 165 ms to show ([site](https://shottr.cc/), [freezing capture](https://shottr.cc/kb/customareacapture)).
- **Greenshot** (Windows free, macOS $1.99) and **Snagit** (subscription) are popular too; their freeze behaviour and speed are not documented ([Greenshot](https://getgreenshot.org/downloads/), [Snagit](https://www.techsmith.com/store/snagit)).

</details>

Back to the [README](../README.md).
