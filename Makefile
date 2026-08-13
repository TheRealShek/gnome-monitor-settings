PREFIX ?= /usr
DESTDIR ?=
UUID := monitor-settings@avifenesh.github.io

.PHONY: all build check install pack

all: build

build:
	cargo build --release --locked

pack:
	gnome-extensions pack extension --extra-source=state.js --force

check:
	cargo fmt --all -- --check
	cargo clippy --all-targets --all-features -- -D warnings
	cargo test --all-targets --all-features
	gjs -m tests/extension-state.test.js
	node --check extension/extension.js
	node --check extension/state.js
	desktop-file-validate data/io.github.avifenesh.GnomeMonitorSettings.desktop
	appstreamcli validate --no-net data/io.github.avifenesh.GnomeMonitorSettings.metainfo.xml
	xmllint --noout data/io.github.avifenesh.GnomeMonitorSettings1.xml data/io.github.avifenesh.GnomeMonitorSettings.metainfo.xml
	python -m json.tool extension/metadata.json >/dev/null

install: build
	install -Dm755 target/release/gnome-monitor-settings $(DESTDIR)$(PREFIX)/bin/gnome-monitor-settings
	install -Dm755 target/release/gnome-monitor-settings-service $(DESTDIR)$(PREFIX)/bin/gnome-monitor-settings-service
	install -Dm644 data/io.github.avifenesh.GnomeMonitorSettings.desktop $(DESTDIR)$(PREFIX)/share/applications/io.github.avifenesh.GnomeMonitorSettings.desktop
	install -Dm644 data/io.github.avifenesh.GnomeMonitorSettings.metainfo.xml $(DESTDIR)$(PREFIX)/share/metainfo/io.github.avifenesh.GnomeMonitorSettings.metainfo.xml
	install -Dm644 data/io.github.avifenesh.GnomeMonitorSettings.svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/io.github.avifenesh.GnomeMonitorSettings.svg
	install -Dm644 data/io.github.avifenesh.GnomeMonitorSettings.service $(DESTDIR)$(PREFIX)/share/dbus-1/services/io.github.avifenesh.GnomeMonitorSettings.service
	install -Dm644 data/gnome-monitor-settings-service.service $(DESTDIR)$(PREFIX)/lib/systemd/user/gnome-monitor-settings-service.service
	install -Dm644 data/io.github.avifenesh.GnomeMonitorSettings1.xml $(DESTDIR)$(PREFIX)/share/dbus-1/interfaces/io.github.avifenesh.GnomeMonitorSettings1.xml
	install -d $(DESTDIR)$(PREFIX)/share/gnome-shell/extensions/$(UUID)
	install -m644 extension/extension.js extension/state.js extension/metadata.json $(DESTDIR)$(PREFIX)/share/gnome-shell/extensions/$(UUID)/
