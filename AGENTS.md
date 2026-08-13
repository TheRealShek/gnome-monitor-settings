# AGENTS.md

> **Project:** **gnome-monitor-settings** — Native GNOME controls for settings exposed by DDC/CI external monitors.
> **Stack:** Rust · GTK 4/libadwaita · zbus · GJS GNOME Shell extension · ddcutil subprocess

---

## Commands

<!-- Keep the commands an agent should run to build, test, lint, or validate changes. -->

| Action | Command | Note |
| ------ | ------- | ---- |
| Full non-hardware verification | `make check` | Does not launch the service or write monitor settings. |
| Rust unit tests | `cargo test --all-targets --all-features` | Uses parser fixtures and a fake backend. |
| Release build | `make build` | Builds both Rust binaries with the lockfile. |
| Native install | `sudo make install` | Installs the app, service, D-Bus metadata, and GNOME 50 extension. |

---

## Working Agreement

- Read nearby code and tests before changing behavior; follow established project conventions.
- Prefer the smallest cohesive change that fully solves the request.
- Preserve public behavior unless the task explicitly requires a breaking change.
- Add or update tests for changed behavior, including meaningful failure cases.
- Use existing abstractions and dependencies before introducing new ones.
- Return errors with enough context to diagnose the failed operation without exposing secrets.
- Run the relevant formatter, static checks, and tests before finishing.
- Update documentation when commands, configuration, or public behavior changes.

## Language Guidelines

- Keep hardware access behind `MonitorBackend`; tests must use a fake backend.
- Do not invoke monitor commands through a shell. Pass validated arguments directly to `Command`.
- Keep GNOME Shell code in GJS; reusable control policy and hardware access belong in Rust.

---

## Gotchas

- DDC terse output prints VCP codes as hexadecimal without a `0x` prefix; values are usually decimal for continuous features.
- Fedora 44 ddcutil 2.2.1 has unreliable built-in write verification; retain the explicit read-back after writes.

<!-- Example format: `path/` has an unusual constraint — explain the failure it prevents. -->

---

## Rules

<!-- Project-specific only. If it applies to every project everywhere, delete it. -->

- Never add power, reset, input-source, or manufacturer-specific VCP codes to `SAFE_FEATURES` without explicit scope and hardware-risk review.
- Do not run hardware-writing tests without the owner's explicit approval.

---

## Verified

Last verified: 2026-08-13 on Fedora 44 with GNOME 50 build dependencies.
