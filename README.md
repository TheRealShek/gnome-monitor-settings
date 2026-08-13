# Monitor Settings for GNOME

Monitor Settings is a native GNOME application and Quick Settings extension for controls exposed by DDC/CI external monitors. A Rust user service owns monitor access, the Rust GTK/libadwaita application exposes every deliberately supported control, and a small GJS extension provides one brightness slider per monitor plus a combined slider when multiple monitors are connected.

The project currently targets Fedora 44 and GNOME Shell 50. It does not replace GNOME's built-in laptop-panel brightness control.

## Safety boundary

Only these VCP features are accepted by the service:

- brightness (`0x10`)
- contrast (`0x12`)
- monitor speaker volume (`0x62`)
- monitor mute (`0x8d`)
- colour preset (`0x14`)
- red, green, and blue gain (`0x16`, `0x18`, `0x1a`)

The service rejects every other feature even if a client calls D-Bus directly. In particular, it does not expose monitor power, factory reset, input switching, or manufacturer-specific features. Values are checked against discovered limits, operations are serialized per I²C bus, rapid writes are rate-limited, and successful writes are read back.

No process needs root privileges. Fedora's `ddcutil` package supplies a udev rule that grants the active desktop user access to applicable I²C devices.

## Architecture

```text
GNOME Quick Settings extension (GJS) ─┐
                                      ├─ D-Bus ─ Rust user service ─ ddcutil ─ monitor
GTK/libadwaita application (Rust) ────┘
```

Using `ddcutil` as a subprocess is intentional for the first release. It isolates monitor and native-library failures from GNOME Shell, uses Fedora's maintained monitor compatibility layer, and avoids tying the service to an unstable Rust wrapper around `libddcutil`. The service calls the executable directly without a shell and addresses a cached I²C bus for responsive reads.

## Supported versions

| Component | Minimum/target |
| --- | --- |
| Fedora | 44 |
| GNOME Shell | 50.x |
| GTK | 4.22 |
| libadwaita | 1.9 |
| Rust | 1.92 (MSRV) |
| ddcutil | 2.2.1 |

Fedora 44 packages ddcutil 2.2.1. Because verification in ddcutil 2.2.1 is known to be unreliable, the service performs an explicit read after each write instead of trusting `setvcp --verify`.

## Build and verify

Install the Fedora build dependencies:

```sh
sudo dnf install cargo rust ddcutil gtk4-devel libadwaita-devel glib2-devel appstream
```

Run the non-hardware checks:

```sh
make check
```

These checks use parser fixtures and an in-memory monitor backend. They neither launch the service nor write monitor settings.

Build the release binaries:

```sh
make build
```

Create an installable GNOME extension bundle:

```sh
make pack
```

## Install

The native install is required because a Flatpak cannot normally access host I²C devices or the host `ddcutil` executable.

```sh
sudo make install
sudo update-desktop-database
sudo gtk-update-icon-cache -f /usr/share/icons/hicolor
```

Log out and back in so GNOME discovers the system extension, then enable it:

```sh
gnome-extensions enable monitor-settings@avifenesh.github.io
```

Opening the application or Quick Settings activates the user service through D-Bus. Clicking a Quick Settings slider icon opens the full application.

After upgrading an existing installation, reload and restart the user service so it acquires its dedicated D-Bus name:

```sh
systemctl --user daemon-reload
systemctl --user restart gnome-monitor-settings-service.service
```

## Hardware validation

Hardware writes are intentionally a separate validation stage. Before enabling the extension, verify discovery and read-only communication manually:

```sh
ddcutil detect --brief
ddcutil getvcp 10 --display 1
```

The first approved write should move brightness by only one unit, confirm the physical result, and read it back. Do not start with input, power, reset, or manufacturer-specific controls.

## Known limitations

- Internal laptop panels use the kernel backlight interface, not DDC/CI, and remain under GNOME's built-in brightness control.
- Monitor capability strings and maximum values can be inaccurate. A displayed control means the monitor answered a probe, not that its firmware is perfectly compliant.
- Docks, KVM switches, DisplayLink devices, and some DisplayPort paths may block DDC/CI.
- A monitor may temporarily stop accepting brightness while dynamic contrast, HDR, or a vendor picture mode is enabled.
- GNOME Shell extensions are version-sensitive. Shell 51 support will be declared only after testing it.
