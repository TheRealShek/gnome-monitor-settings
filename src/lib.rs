pub mod backend;
pub mod manager;
pub mod model;

pub const APP_ID: &str = "io.github.avifenesh.GnomeMonitorSettings";
pub const DBUS_NAME: &str = "io.github.avifenesh.GnomeMonitorSettings.Service";
pub const DBUS_PATH: &str = "/io/github/avifenesh/GnomeMonitorSettings";
pub const DBUS_INTERFACE: &str = "io.github.avifenesh.GnomeMonitorSettings1";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_and_service_have_distinct_bus_names() {
        assert_ne!(APP_ID, DBUS_NAME);
    }
}
