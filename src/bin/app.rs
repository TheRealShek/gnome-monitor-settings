use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use adw::prelude::*;
use anyhow::{Context, Result};
use gio::prelude::DBusProxyExt;
use glib::variant::ToVariant;
use gnome_monitor_settings::{
    DBUS_INTERFACE, DBUS_NAME, DBUS_PATH,
    model::{BRIGHTNESS, Control, ControlKind, Monitor, ServiceState},
};
use gtk::{gio, glib};

const APP_ID: &str = "io.github.avifenesh.GnomeMonitorSettings";
const DBUS_TIMEOUT_MS: i32 = 10_000;

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &adw::Application) {
    let toast_overlay = adw::ToastOverlay::new();
    let page = adw::PreferencesPage::new();
    let groups = Rc::new(RefCell::new(Vec::<adw::PreferencesGroup>::new()));
    toast_overlay.set_child(Some(&page));

    let header = adw::HeaderBar::new();
    let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh.set_tooltip_text(Some("Rescan monitors"));
    header.pack_end(&refresh);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&toast_overlay));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Monitor Settings")
        .default_width(620)
        .default_height(720)
        .content(&toolbar)
        .build();

    let proxy = match gio::DBusProxy::for_bus_sync(
        gio::BusType::Session,
        gio::DBusProxyFlags::NONE,
        None,
        DBUS_NAME,
        DBUS_PATH,
        DBUS_INTERFACE,
        gio::Cancellable::NONE,
    ) {
        Ok(proxy) => proxy,
        Err(error) => {
            show_error_group(
                &page,
                &groups,
                "Monitor service unavailable",
                &format!(
                    "The monitor service could not be started through D-Bus: {error}. Install the service activation file and try again."
                ),
            );
            window.present();
            return;
        }
    };

    let page_for_refresh = page.clone();
    let groups_for_refresh = Rc::clone(&groups);
    let overlay_for_refresh = toast_overlay.clone();
    let proxy_for_refresh = proxy.clone();
    refresh.connect_clicked(move |button| {
        button.set_sensitive(false);
        let button = button.clone();
        let page = page_for_refresh.clone();
        let groups = Rc::clone(&groups_for_refresh);
        let overlay = overlay_for_refresh.clone();
        let proxy = proxy_for_refresh.clone();
        glib::spawn_future_local(async move {
            match call_state_method(&proxy, "Rescan", None).await {
                Ok(state) => render_state(&page, &groups, &overlay, &proxy, state),
                Err(error) => overlay.add_toast(adw::Toast::new(&error.to_string())),
            }
            button.set_sensitive(true);
        });
    });

    let page_for_load = page.clone();
    let groups_for_load = Rc::clone(&groups);
    let overlay_for_load = toast_overlay.clone();
    let proxy_for_load = proxy.clone();
    glib::spawn_future_local(async move {
        match call_state_method(&proxy_for_load, "GetStateJson", None).await {
            Ok(state) if state.ready => render_state(
                &page_for_load,
                &groups_for_load,
                &overlay_for_load,
                &proxy_for_load,
                state,
            ),
            Ok(_) => match call_state_method(&proxy_for_load, "Rescan", None).await {
                Ok(state) => render_state(
                    &page_for_load,
                    &groups_for_load,
                    &overlay_for_load,
                    &proxy_for_load,
                    state,
                ),
                Err(error) => show_error_group(
                    &page_for_load,
                    &groups_for_load,
                    "No monitor controls available",
                    &error.to_string(),
                ),
            },
            Err(error) => show_error_group(
                &page_for_load,
                &groups_for_load,
                "Monitor service error",
                &error.to_string(),
            ),
        }
    });

    window.present();
}

fn render_state(
    page: &adw::PreferencesPage,
    groups: &Rc<RefCell<Vec<adw::PreferencesGroup>>>,
    overlay: &adw::ToastOverlay,
    proxy: &gio::DBusProxy,
    state: ServiceState,
) {
    for group in groups.borrow_mut().drain(..) {
        page.remove(&group);
    }

    if let Some(error) = state.error.as_deref() {
        overlay.add_toast(adw::Toast::new(error));
    }

    if state.monitors.is_empty() {
        show_error_group(
            page,
            groups,
            "No supported external monitors",
            "Connect a DDC/CI-capable monitor, enable DDC/CI in its on-screen menu, then rescan.",
        );
        return;
    }

    if state.monitors.len() > 1 {
        add_all_monitors_group(page, groups, overlay, proxy, &state.monitors);
    }

    for monitor in state.monitors {
        add_monitor_group(page, groups, overlay, proxy, &monitor);
    }
}

fn add_all_monitors_group(
    page: &adw::PreferencesPage,
    groups: &Rc<RefCell<Vec<adw::PreferencesGroup>>>,
    overlay: &adw::ToastOverlay,
    proxy: &gio::DBusProxy,
    monitors: &[Monitor],
) {
    let brightness: Vec<&Control> = monitors
        .iter()
        .filter_map(|monitor| monitor.control(BRIGHTNESS))
        .collect();
    if brightness.len() < 2 {
        return;
    }

    let average = brightness
        .iter()
        .map(|control| normalized_percent(control) as u32)
        .sum::<u32>() as f64
        / brightness.len() as f64;
    let group = adw::PreferencesGroup::builder()
        .title("All monitors")
        .description("Set external-monitor brightness together")
        .build();
    group.add(&continuous_row(
        overlay,
        proxy,
        None,
        "Brightness",
        average,
        100.0,
        BRIGHTNESS,
        true,
    ));
    page.add(&group);
    groups.borrow_mut().push(group);
}

fn add_monitor_group(
    page: &adw::PreferencesPage,
    groups: &Rc<RefCell<Vec<adw::PreferencesGroup>>>,
    overlay: &adw::ToastOverlay,
    proxy: &gio::DBusProxy,
    monitor: &Monitor,
) {
    let description = if monitor.connector.is_empty() {
        monitor.manufacturer.clone()
    } else {
        format!("{} · {}", monitor.manufacturer, monitor.connector)
    };
    let group = adw::PreferencesGroup::builder()
        .title(&monitor.name)
        .description(&description)
        .build();

    for control in &monitor.controls {
        match control.kind {
            ControlKind::Continuous => group.add(&continuous_row(
                overlay,
                proxy,
                Some(&monitor.id),
                &control.title,
                control.current as f64,
                control.maximum.max(1) as f64,
                control.code,
                false,
            )),
            ControlKind::Toggle => group.add(&toggle_row(overlay, proxy, &monitor.id, control)),
            ControlKind::Choice => group.add(&choice_row(overlay, proxy, &monitor.id, control)),
        }
    }
    page.add(&group);
    groups.borrow_mut().push(group);
}

#[allow(clippy::too_many_arguments)]
fn continuous_row(
    overlay: &adw::ToastOverlay,
    proxy: &gio::DBusProxy,
    monitor_id: Option<&str>,
    title: &str,
    current: f64,
    maximum: f64,
    code: u8,
    all_monitors: bool,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).build();
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, maximum, 1.0);
    scale.set_value(current.clamp(0.0, maximum));
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Right);
    scale.set_width_request(300);
    scale.set_hexpand(true);
    row.add_suffix(&scale);
    row.set_activatable_widget(Some(&scale));

    let debounce = Rc::new(RefCell::new(None::<glib::SourceId>));
    let updating = Rc::new(Cell::new(false));
    let confirmed = Rc::new(Cell::new(current.clamp(0.0, maximum)));
    let monitor_id = monitor_id.map(str::to_owned);
    let overlay = overlay.clone();
    let proxy = proxy.clone();
    scale.connect_value_changed(move |scale| {
        if updating.replace(false) {
            return;
        }
        if let Some(source) = debounce.borrow_mut().take() {
            source.remove();
        }
        let scale = scale.clone();
        let monitor_id = monitor_id.clone();
        let overlay = overlay.clone();
        let proxy = proxy.clone();
        let debounce_after_timeout = Rc::clone(&debounce);
        let updating = Rc::clone(&updating);
        let confirmed = Rc::clone(&confirmed);
        *debounce.borrow_mut() = Some(glib::timeout_add_local_once(
            std::time::Duration::from_millis(250),
            move || {
                debounce_after_timeout.borrow_mut().take();
                let value = scale.value().round() as u16;
                scale.set_sensitive(false);
                glib::spawn_future_local(async move {
                    let parameters = if all_monitors {
                        (value,).to_variant()
                    } else {
                        (monitor_id.as_deref().unwrap_or_default(), code, value).to_variant()
                    };
                    let method = if all_monitors {
                        "SetAllBrightness"
                    } else {
                        "SetControl"
                    };
                    match call_state_method(&proxy, method, Some(&parameters)).await {
                        Ok(state) => {
                            let actual = if all_monitors {
                                let controls: Vec<&Control> = state
                                    .monitors
                                    .iter()
                                    .filter_map(|monitor| monitor.control(BRIGHTNESS))
                                    .collect();
                                (!controls.is_empty()).then(|| {
                                    controls
                                        .iter()
                                        .map(|control| {
                                            f64::from(control.current) * 100.0
                                                / f64::from(control.maximum.max(1))
                                        })
                                        .sum::<f64>()
                                        / controls.len() as f64
                                })
                            } else {
                                state.monitors.iter().find_map(|monitor| {
                                    monitor_id
                                        .as_deref()
                                        .filter(|id| *id == monitor.id)
                                        .and_then(|_| monitor.control(code))
                                        .map(|control| f64::from(control.current))
                                })
                            };
                            if let Some(actual) = actual {
                                confirmed.set(actual);
                                if (scale.value() - actual).abs() > f64::EPSILON {
                                    updating.set(true);
                                    scale.set_value(actual);
                                }
                            }
                        }
                        Err(error) => {
                            let previous = confirmed.get();
                            if (scale.value() - previous).abs() > f64::EPSILON {
                                updating.set(true);
                                scale.set_value(previous);
                            }
                            overlay.add_toast(adw::Toast::new(&error.to_string()));
                        }
                    }
                    scale.set_sensitive(true);
                });
            },
        ));
    });
    row
}

fn toggle_row(
    overlay: &adw::ToastOverlay,
    proxy: &gio::DBusProxy,
    monitor_id: &str,
    control: &Control,
) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder()
        .title(&control.title)
        .active(control.current == 1)
        .build();
    let updating = Rc::new(Cell::new(false));
    let monitor_id = monitor_id.to_owned();
    let overlay = overlay.clone();
    let proxy = proxy.clone();
    let code = control.code;
    row.connect_active_notify(move |row| {
        if updating.replace(false) {
            return;
        }
        let row = row.clone();
        let overlay = overlay.clone();
        let proxy = proxy.clone();
        let monitor_id = monitor_id.clone();
        let updating = Rc::clone(&updating);
        let previous = !row.is_active();
        let value = if row.is_active() { 1u16 } else { 2u16 };
        row.set_sensitive(false);
        glib::spawn_future_local(async move {
            let parameters = (monitor_id.as_str(), code, value).to_variant();
            match call_state_method(&proxy, "SetControl", Some(&parameters)).await {
                Ok(state) => {
                    let active = state.monitors.iter().find_map(|monitor| {
                        (monitor.id == monitor_id)
                            .then(|| monitor.control(code))
                            .flatten()
                            .map(|control| control.current == 1)
                    });
                    if active.is_some_and(|active| active != row.is_active()) {
                        updating.set(true);
                        row.set_active(active.unwrap_or(previous));
                    }
                }
                Err(error) => {
                    updating.set(true);
                    row.set_active(previous);
                    overlay.add_toast(adw::Toast::new(&error.to_string()));
                }
            }
            row.set_sensitive(true);
        });
    });
    row
}

fn choice_row(
    overlay: &adw::ToastOverlay,
    proxy: &gio::DBusProxy,
    monitor_id: &str,
    control: &Control,
) -> adw::ComboRow {
    let labels: Vec<&str> = control
        .choices
        .iter()
        .map(|choice| choice.label.as_str())
        .collect();
    let model = gtk::StringList::new(&labels);
    let selected = control
        .choices
        .iter()
        .position(|choice| choice.value == control.current)
        .unwrap_or(gtk::INVALID_LIST_POSITION as usize) as u32;
    let row = adw::ComboRow::builder()
        .title(&control.title)
        .model(&model)
        .selected(selected)
        .build();
    let values = Rc::new(
        control
            .choices
            .iter()
            .map(|choice| choice.value)
            .collect::<Vec<_>>(),
    );
    let confirmed = Rc::new(Cell::new(selected));
    let updating = Rc::new(Cell::new(false));
    let monitor_id = monitor_id.to_owned();
    let overlay = overlay.clone();
    let proxy = proxy.clone();
    let code = control.code;
    row.connect_selected_notify(move |row| {
        if updating.replace(false) || row.selected() == gtk::INVALID_LIST_POSITION {
            return;
        }
        let Some(value) = values.get(row.selected() as usize).copied() else {
            return;
        };
        let previous = confirmed.replace(row.selected());
        let overlay = overlay.clone();
        let proxy = proxy.clone();
        let monitor_id = monitor_id.clone();
        let values = Rc::clone(&values);
        let confirmed = Rc::clone(&confirmed);
        let updating = Rc::clone(&updating);
        row.set_sensitive(false);
        let row = row.clone();
        glib::spawn_future_local(async move {
            let parameters = (monitor_id.as_str(), code, value).to_variant();
            match call_state_method(&proxy, "SetControl", Some(&parameters)).await {
                Ok(state) => {
                    let actual = state.monitors.iter().find_map(|monitor| {
                        (monitor.id == monitor_id)
                            .then(|| monitor.control(code))
                            .flatten()
                            .and_then(|control| {
                                values.iter().position(|value| *value == control.current)
                            })
                            .map(|position| position as u32)
                    });
                    if let Some(actual) = actual {
                        confirmed.set(actual);
                        if actual != row.selected() {
                            updating.set(true);
                            row.set_selected(actual);
                        }
                    }
                }
                Err(error) => {
                    confirmed.set(previous);
                    updating.set(true);
                    row.set_selected(previous);
                    overlay.add_toast(adw::Toast::new(&error.to_string()));
                }
            }
            row.set_sensitive(true);
        });
    });
    row
}

async fn call_state_method(
    proxy: &gio::DBusProxy,
    method: &str,
    parameters: Option<&glib::Variant>,
) -> Result<ServiceState> {
    let reply = proxy
        .call_future(
            method,
            parameters,
            gio::DBusCallFlags::NONE,
            DBUS_TIMEOUT_MS,
        )
        .await
        .with_context(|| format!("{method} failed"))?;
    let (json,): (String,) = reply
        .get()
        .with_context(|| format!("{method} returned an invalid D-Bus response"))?;
    serde_json::from_str(&json).with_context(|| format!("{method} returned invalid state"))
}

fn show_error_group(
    page: &adw::PreferencesPage,
    groups: &Rc<RefCell<Vec<adw::PreferencesGroup>>>,
    title: &str,
    description: &str,
) {
    let group = adw::PreferencesGroup::builder().title(title).build();
    group.add(
        &adw::StatusPage::builder()
            .icon_name("dialog-warning-symbolic")
            .title(title)
            .description(description)
            .build(),
    );
    page.add(&group);
    groups.borrow_mut().push(group);
}

fn normalized_percent(control: &Control) -> u16 {
    if control.maximum == 0 {
        0
    } else {
        ((u32::from(control.current) * 100) / u32::from(control.maximum)) as u16
    }
}
