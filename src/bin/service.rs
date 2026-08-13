use std::{fs, path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use gnome_monitor_settings::{
    DBUS_INTERFACE, DBUS_NAME, DBUS_PATH, backend::DdcutilBackend, manager::MonitorManager,
};
use zbus::object_server::SignalEmitter;

struct MonitorSettingsService {
    manager: Arc<MonitorManager>,
}

const CONNECTOR_POLL_INTERVAL: Duration = Duration::from_secs(10);

impl MonitorSettingsService {
    fn json<T: serde::Serialize>(value: &T) -> zbus::fdo::Result<String> {
        serde_json::to_string(value).map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    fn dbus_error(error: impl std::fmt::Display) -> zbus::fdo::Error {
        zbus::fdo::Error::Failed(error.to_string())
    }
}

#[zbus::interface(name = "io.github.avifenesh.GnomeMonitorSettings1")]
impl MonitorSettingsService {
    async fn get_state_json(&self) -> zbus::fdo::Result<String> {
        Self::json(&self.manager.state().await)
    }

    async fn rescan(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<String> {
        let state = if self.manager.state().await.ready {
            self.manager.refresh().await
        } else {
            self.manager.ensure_ready().await
        }
        .map_err(Self::dbus_error)?;
        let json = Self::json(&state)?;
        Self::state_changed(&emitter, &json)
            .await
            .map_err(Self::dbus_error)?;
        Ok(json)
    }

    async fn set_control(
        &self,
        monitor_id: &str,
        code: u8,
        value: u16,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<String> {
        let state = self
            .manager
            .set_control(monitor_id, code, value)
            .await
            .map_err(Self::dbus_error)?;
        let json = Self::json(&state)?;
        Self::state_changed(&emitter, &json)
            .await
            .map_err(Self::dbus_error)?;
        Ok(json)
    }

    async fn set_all_brightness(
        &self,
        value: u16,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<String> {
        let state = self
            .manager
            .set_all_brightness(value)
            .await
            .map_err(Self::dbus_error)?;
        let json = Self::json(&state)?;
        Self::state_changed(&emitter, &json)
            .await
            .map_err(Self::dbus_error)?;
        Ok(json)
    }

    #[zbus(property)]
    fn api_version(&self) -> u32 {
        gnome_monitor_settings::model::API_VERSION
    }

    #[zbus(signal)]
    async fn state_changed(emitter: &SignalEmitter<'_>, state_json: &str) -> zbus::Result<()>;
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "gnome_monitor_settings=info".into()),
        )
        .init();

    debug_assert_eq!(DBUS_INTERFACE, "io.github.avifenesh.GnomeMonitorSettings1");
    let manager = Arc::new(MonitorManager::new(Arc::new(DdcutilBackend::default())));

    let connection = zbus::connection::Builder::session()
        .context("failed to connect to the user D-Bus")?
        .name(DBUS_NAME)
        .context("invalid D-Bus service name")?
        .serve_at(
            DBUS_PATH,
            MonitorSettingsService {
                manager: Arc::clone(&manager),
            },
        )
        .context("invalid D-Bus object path")?
        .build()
        .await
        .context("failed to publish monitor settings service")?;

    watch_connectors(connection.clone(), Arc::clone(&manager));
    initialize_state(connection.clone(), Arc::clone(&manager));

    tracing::info!(service = DBUS_NAME, "monitor settings service is ready");
    std::future::pending().await
}

fn initialize_state(connection: zbus::Connection, manager: Arc<MonitorManager>) {
    tokio::spawn(async move {
        let state = match manager.ensure_ready().await {
            Ok(state) => state,
            Err(error) => {
                tracing::warn!(%error, "initial monitor discovery failed; service will remain available");
                manager.state().await
            }
        };
        publish_state(&connection, &state).await;
    });
}

fn watch_connectors(connection: zbus::Connection, manager: Arc<MonitorManager>) {
    tokio::spawn(async move {
        let mut signature = connector_signature();
        loop {
            tokio::time::sleep(CONNECTOR_POLL_INTERVAL).await;
            let next_signature = connector_signature();
            if next_signature == signature {
                continue;
            }
            signature = next_signature;

            let state = match manager.refresh().await {
                Ok(state) => state,
                Err(error) => {
                    tracing::warn!(%error, "monitor refresh after connector change failed");
                    manager.state().await
                }
            };
            publish_state(&connection, &state).await;
        }
    });
}

async fn publish_state(
    connection: &zbus::Connection,
    state: &gnome_monitor_settings::model::ServiceState,
) {
    let Ok(json) = serde_json::to_string(state) else {
        tracing::error!("failed to serialize refreshed monitor state");
        return;
    };
    let interface = match connection
        .object_server()
        .interface::<_, MonitorSettingsService>(DBUS_PATH)
        .await
    {
        Ok(interface) => interface,
        Err(error) => {
            tracing::error!(%error, "failed to obtain monitor service interface");
            return;
        }
    };
    if let Err(error) =
        MonitorSettingsService::state_changed(interface.signal_emitter(), &json).await
    {
        tracing::warn!(%error, "failed to publish refreshed monitor state");
    }
}

fn connector_signature() -> Vec<(String, String)> {
    let mut signature = Vec::new();
    let Ok(entries) = fs::read_dir(Path::new("/sys/class/drm")) else {
        return signature;
    };
    for entry in entries.flatten() {
        let status_path = entry.path().join("status");
        let Ok(status) = fs::read_to_string(status_path) else {
            continue;
        };
        signature.push((
            entry.file_name().to_string_lossy().into_owned(),
            status.trim().to_owned(),
        ));
    }
    signature.sort_unstable();
    signature
}
