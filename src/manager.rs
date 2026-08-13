use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use thiserror::Error;
use tokio::sync::{Mutex, RwLock};

use crate::{
    backend::{BackendError, MonitorBackend},
    model::{BRIGHTNESS, ServiceState, feature_definition},
};

const MIN_WRITE_INTERVAL: Duration = Duration::from_millis(120);

#[derive(Debug, Error)]
pub enum ManagerError {
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error("monitor {0} is not connected")]
    MissingMonitor(String),
    #[error("monitor {monitor_id} does not expose writable VCP feature 0x{code:02x}")]
    Unsupported { monitor_id: String, code: u8 },
    #[error("value {value} is invalid for VCP feature 0x{code:02x}; maximum is {maximum}")]
    OutOfRange { code: u8, value: u16, maximum: u16 },
    #[error("monitor backend task failed: {0}")]
    Task(String),
}

pub struct MonitorManager {
    backend: Arc<dyn MonitorBackend>,
    state: RwLock<ServiceState>,
    refresh_lock: Mutex<()>,
    bus_locks: Mutex<HashMap<u32, Arc<Mutex<()>>>>,
    last_writes: Mutex<HashMap<(String, u8), Instant>>,
    min_write_interval: Duration,
}

impl MonitorManager {
    pub fn new(backend: Arc<dyn MonitorBackend>) -> Self {
        Self::with_write_interval(backend, MIN_WRITE_INTERVAL)
    }

    pub fn with_write_interval(
        backend: Arc<dyn MonitorBackend>,
        min_write_interval: Duration,
    ) -> Self {
        Self {
            backend,
            state: RwLock::new(ServiceState::default()),
            refresh_lock: Mutex::new(()),
            bus_locks: Mutex::new(HashMap::new()),
            last_writes: Mutex::new(HashMap::new()),
            min_write_interval,
        }
    }

    pub async fn state(&self) -> ServiceState {
        self.state.read().await.clone()
    }

    pub async fn refresh(&self) -> Result<ServiceState, ManagerError> {
        let _refresh_guard = self.refresh_lock.lock().await;
        self.refresh_backend().await
    }

    pub async fn ensure_ready(&self) -> Result<ServiceState, ManagerError> {
        if self.state.read().await.ready {
            return Ok(self.state().await);
        }
        let _refresh_guard = self.refresh_lock.lock().await;
        if self.state.read().await.ready {
            return Ok(self.state().await);
        }
        self.refresh_backend().await
    }

    async fn refresh_backend(&self) -> Result<ServiceState, ManagerError> {
        let backend = Arc::clone(&self.backend);
        let result = tokio::task::spawn_blocking(move || {
            let version = backend.version()?;
            let monitors = backend.discover()?;
            Ok::<_, BackendError>((version, monitors))
        })
        .await
        .map_err(|error| ManagerError::Task(error.to_string()))?;

        match result {
            Ok((version, monitors)) => {
                let next = ServiceState {
                    api_version: crate::model::API_VERSION,
                    ready: true,
                    ddcutil_version: Some(version),
                    error: None,
                    monitors,
                };
                *self.state.write().await = next.clone();
                Ok(next)
            }
            Err(error) => {
                let mut state = self.state.write().await;
                state.ready = false;
                state.error = Some(error.to_string());
                Err(error.into())
            }
        }
    }

    pub async fn set_control(
        &self,
        monitor_id: &str,
        code: u8,
        value: u16,
    ) -> Result<ServiceState, ManagerError> {
        feature_definition(code).ok_or_else(|| ManagerError::Unsupported {
            monitor_id: monitor_id.to_owned(),
            code,
        })?;

        let (bus, maximum, valid_choice) = {
            let state = self.state.read().await;
            let monitor = state
                .monitors
                .iter()
                .find(|monitor| monitor.id == monitor_id)
                .ok_or_else(|| ManagerError::MissingMonitor(monitor_id.to_owned()))?;
            let control = monitor
                .control(code)
                .filter(|control| control.writable)
                .ok_or_else(|| ManagerError::Unsupported {
                    monitor_id: monitor_id.to_owned(),
                    code,
                })?;
            let valid_choice = control.choices.is_empty()
                || control.choices.iter().any(|choice| choice.value == value);
            (monitor.bus, control.maximum, valid_choice)
        };

        if value > maximum || !valid_choice {
            return Err(ManagerError::OutOfRange {
                code,
                value,
                maximum,
            });
        }

        let bus_lock = {
            let mut locks = self.bus_locks.lock().await;
            Arc::clone(locks.entry(bus).or_insert_with(|| Arc::new(Mutex::new(()))))
        };
        let _bus_guard = bus_lock.lock().await;
        self.rate_limit(monitor_id, code).await;

        let backend = Arc::clone(&self.backend);
        let updated = tokio::task::spawn_blocking(move || {
            backend.write_control(bus, code, value)?;
            backend.read_control(bus, code)
        })
        .await
        .map_err(|error| ManagerError::Task(error.to_string()))??;

        let mut state = self.state.write().await;
        let monitor = state
            .monitors
            .iter_mut()
            .find(|monitor| monitor.id == monitor_id)
            .ok_or_else(|| ManagerError::MissingMonitor(monitor_id.to_owned()))?;
        let control = monitor
            .controls
            .iter_mut()
            .find(|control| control.code == code)
            .ok_or_else(|| ManagerError::Unsupported {
                monitor_id: monitor_id.to_owned(),
                code,
            })?;
        *control = updated;
        state.error = None;
        Ok(state.clone())
    }

    pub async fn set_all_brightness(&self, value: u16) -> Result<ServiceState, ManagerError> {
        if value > 100 {
            return Err(ManagerError::OutOfRange {
                code: BRIGHTNESS,
                value,
                maximum: 100,
            });
        }

        let monitors: Vec<(String, u16)> = self
            .state
            .read()
            .await
            .monitors
            .iter()
            .filter_map(|monitor| {
                monitor
                    .control(BRIGHTNESS)
                    .map(|control| (monitor.id.clone(), control.maximum))
            })
            .collect();

        if monitors.is_empty() {
            return Err(ManagerError::Unsupported {
                monitor_id: "all monitors".to_owned(),
                code: BRIGHTNESS,
            });
        }

        let mut failures = Vec::new();
        let mut successes = 0;
        for (monitor_id, maximum) in monitors {
            let monitor_value = ((u32::from(value) * u32::from(maximum) + 50) / 100) as u16;
            match self
                .set_control(&monitor_id, BRIGHTNESS, monitor_value)
                .await
            {
                Ok(_) => successes += 1,
                Err(error) => failures.push(format!("{monitor_id}: {error}")),
            }
        }

        if successes == 0 {
            return Err(ManagerError::Backend(BackendError::Command(
                failures.join("; "),
            )));
        }

        let mut state = self.state.write().await;
        if failures.is_empty() {
            state.error = None;
        } else {
            state.error = Some(format!(
                "Some monitors could not be updated: {}",
                failures.join("; ")
            ));
        }
        Ok(state.clone())
    }

    async fn rate_limit(&self, monitor_id: &str, code: u8) {
        let key = (monitor_id.to_owned(), code);
        let delay = {
            let writes = self.last_writes.lock().await;
            writes
                .get(&key)
                .and_then(|last_write| self.min_write_interval.checked_sub(last_write.elapsed()))
        };
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        self.last_writes.lock().await.insert(key, Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use crate::model::{Control, ControlKind, Monitor};

    use super::*;

    struct FakeBackend {
        monitors: Vec<Monitor>,
        writes: StdMutex<Vec<(u32, u8, u16)>>,
    }

    impl MonitorBackend for FakeBackend {
        fn version(&self) -> Result<String, BackendError> {
            Ok("2.2.1".to_owned())
        }

        fn discover(&self) -> Result<Vec<Monitor>, BackendError> {
            Ok(self.monitors.clone())
        }

        fn read_control(&self, _bus: u32, code: u8) -> Result<Control, BackendError> {
            let value = self
                .writes
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|(_, written_code, _)| *written_code == code)
                .map(|(_, _, value)| *value)
                .unwrap_or(0);
            Ok(Control {
                code,
                key: "brightness".to_owned(),
                title: "Brightness".to_owned(),
                kind: ControlKind::Continuous,
                current: value,
                maximum: 100,
                writable: true,
                choices: Vec::new(),
            })
        }

        fn write_control(&self, bus: u32, code: u8, value: u16) -> Result<(), BackendError> {
            self.writes.lock().unwrap().push((bus, code, value));
            Ok(())
        }
    }

    fn monitor_with_maximum(id: &str, bus: u32, maximum: u16) -> Monitor {
        Monitor {
            id: id.to_owned(),
            name: id.to_owned(),
            manufacturer: "GSM".to_owned(),
            model: id.to_owned(),
            serial: String::new(),
            connector: format!("card0-HDMI-A-{bus}"),
            bus,
            controls: vec![Control {
                code: BRIGHTNESS,
                key: "brightness".to_owned(),
                title: "Brightness".to_owned(),
                kind: ControlKind::Continuous,
                current: 10,
                maximum,
                writable: true,
                choices: Vec::new(),
            }],
        }
    }

    fn monitor(id: &str, bus: u32) -> Monitor {
        monitor_with_maximum(id, bus, 100)
    }

    #[tokio::test]
    async fn validates_and_reads_back_writes() {
        let backend = Arc::new(FakeBackend {
            monitors: vec![monitor("one", 18)],
            writes: StdMutex::new(Vec::new()),
        });
        let manager = MonitorManager::with_write_interval(backend.clone(), Duration::ZERO);
        manager.refresh().await.unwrap();

        let state = manager.set_control("one", BRIGHTNESS, 42).await.unwrap();
        assert_eq!(state.monitors[0].control(BRIGHTNESS).unwrap().current, 42);
        assert_eq!(backend.writes.lock().unwrap().as_slice(), &[(18, 0x10, 42)]);
    }

    #[tokio::test]
    async fn rejects_out_of_range_values_without_writing() {
        let backend = Arc::new(FakeBackend {
            monitors: vec![monitor("one", 18)],
            writes: StdMutex::new(Vec::new()),
        });
        let manager = MonitorManager::with_write_interval(backend.clone(), Duration::ZERO);
        manager.refresh().await.unwrap();

        let error = manager
            .set_control("one", BRIGHTNESS, 101)
            .await
            .unwrap_err();
        assert!(matches!(error, ManagerError::OutOfRange { .. }));
        assert!(backend.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn updates_every_brightness_capable_monitor() {
        let backend = Arc::new(FakeBackend {
            monitors: vec![monitor("one", 18), monitor_with_maximum("two", 19, 200)],
            writes: StdMutex::new(Vec::new()),
        });
        let manager = MonitorManager::with_write_interval(backend.clone(), Duration::ZERO);
        manager.refresh().await.unwrap();

        manager.set_all_brightness(55).await.unwrap();
        assert_eq!(
            backend.writes.lock().unwrap().as_slice(),
            &[(18, BRIGHTNESS, 55), (19, BRIGHTNESS, 110)]
        );
    }

    #[tokio::test]
    async fn rejects_invalid_combined_percentage() {
        let backend = Arc::new(FakeBackend {
            monitors: vec![monitor("one", 18)],
            writes: StdMutex::new(Vec::new()),
        });
        let manager = MonitorManager::with_write_interval(backend.clone(), Duration::ZERO);
        manager.refresh().await.unwrap();

        let error = manager.set_all_brightness(101).await.unwrap_err();
        assert!(matches!(error, ManagerError::OutOfRange { .. }));
        assert!(backend.writes.lock().unwrap().is_empty());
    }
}
