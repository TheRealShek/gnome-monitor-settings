use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    time::Duration,
};

use thiserror::Error;

use crate::model::{Control, Monitor, SAFE_FEATURES, choices_for, feature_definition};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(8);
const COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("ddcutil is not installed at {0}")]
    MissingProgram(PathBuf),
    #[error("ddcutil command timed out after {0:?}")]
    Timeout(Duration),
    #[error("ddcutil failed: {0}")]
    Command(String),
    #[error("ddcutil returned malformed output: {0}")]
    Parse(String),
    #[error("ddcutil {found} is unsupported; version 2.2.1 or newer is required")]
    UnsupportedVersion { found: String },
    #[error("monitor {0} is no longer connected")]
    MissingMonitor(String),
    #[error("monitor {monitor_id} does not expose writable VCP feature 0x{code:02x}")]
    Unsupported { monitor_id: String, code: u8 },
    #[error("value {value} exceeds maximum {maximum} for VCP feature 0x{code:02x}")]
    OutOfRange { code: u8, value: u16, maximum: u16 },
}

pub trait MonitorBackend: Send + Sync {
    fn version(&self) -> Result<String, BackendError>;
    fn discover(&self) -> Result<Vec<Monitor>, BackendError>;
    fn read_control(&self, bus: u32, code: u8) -> Result<Control, BackendError>;
    fn write_control(&self, bus: u32, code: u8, value: u16) -> Result<(), BackendError>;
}

#[derive(Clone, Debug)]
pub struct DdcutilBackend {
    program: PathBuf,
    timeout: Duration,
}

impl Default for DdcutilBackend {
    fn default() -> Self {
        Self::new("/usr/bin/ddcutil")
    }
}

impl DdcutilBackend {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            timeout: COMMAND_TIMEOUT,
        }
    }

    fn run(&self, args: &[String], accept_partial: bool) -> Result<Output, BackendError> {
        if !Path::new(&self.program).is_file() {
            return Err(BackendError::MissingProgram(self.program.clone()));
        }

        let mut child = Command::new(&self.program)
            .args(args)
            .env("LC_ALL", "C")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|error| BackendError::Command(error.to_string()))?;

        let started = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if started.elapsed() < self.timeout => {
                    std::thread::sleep(COMMAND_POLL_INTERVAL);
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(BackendError::Timeout(self.timeout));
                }
                Err(error) => return Err(BackendError::Command(error.to_string())),
            }
        }

        let output = child
            .wait_with_output()
            .map_err(|error| BackendError::Command(error.to_string()))?;
        if output.status.success() || accept_partial {
            Ok(output)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            Err(BackendError::Command(if stderr.is_empty() {
                format!("exited with {}", output.status)
            } else {
                stderr
            }))
        }
    }

    fn read_controls(&self, bus: u32) -> Result<Vec<Control>, BackendError> {
        let mut args = vec!["getvcp".to_owned()];
        args.extend(
            SAFE_FEATURES
                .iter()
                .map(|feature| format!("{:02x}", feature.code)),
        );
        args.extend(["--terse".to_owned(), "--bus".to_owned(), bus.to_string()]);

        let output = self.run(&args, true)?;
        let controls = parse_vcp_output(&String::from_utf8_lossy(&output.stdout));
        if controls.is_empty() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(BackendError::Command(if stderr.is_empty() {
                "no supported safe controls were returned".to_owned()
            } else {
                stderr
            }));
        }
        Ok(controls)
    }
}

impl MonitorBackend for DdcutilBackend {
    fn version(&self) -> Result<String, BackendError> {
        let output = self.run(&["--version".to_owned()], false)?;
        let version = parse_version(&String::from_utf8_lossy(&output.stdout))?;
        require_supported_version(&version)?;
        Ok(version)
    }

    fn discover(&self) -> Result<Vec<Monitor>, BackendError> {
        let output = self.run(&["detect".to_owned(), "--brief".to_owned()], true)?;
        let mut monitors = parse_detect_output(&String::from_utf8_lossy(&output.stdout));
        for monitor in &mut monitors {
            match self.read_controls(monitor.bus) {
                Ok(controls) => monitor.controls = controls,
                Err(error) => {
                    tracing::warn!(monitor = %monitor.id, %error, "control discovery failed")
                }
            }
        }
        Ok(monitors)
    }

    fn read_control(&self, bus: u32, code: u8) -> Result<Control, BackendError> {
        feature_definition(code).ok_or_else(|| BackendError::Unsupported {
            monitor_id: format!("bus-{bus}"),
            code,
        })?;
        let args = [
            "getvcp".to_owned(),
            format!("{code:02x}"),
            "--terse".to_owned(),
            "--bus".to_owned(),
            bus.to_string(),
        ];
        let output = self.run(&args, false)?;
        parse_vcp_output(&String::from_utf8_lossy(&output.stdout))
            .into_iter()
            .find(|control| control.code == code)
            .ok_or_else(|| BackendError::Parse(String::from_utf8_lossy(&output.stdout).into()))
    }

    fn write_control(&self, bus: u32, code: u8, value: u16) -> Result<(), BackendError> {
        feature_definition(code).ok_or_else(|| BackendError::Unsupported {
            monitor_id: format!("bus-{bus}"),
            code,
        })?;
        let args = [
            "setvcp".to_owned(),
            format!("{code:02x}"),
            value.to_string(),
            "--noverify".to_owned(),
            "--bus".to_owned(),
            bus.to_string(),
        ];
        self.run(&args, false).map(|_| ())
    }
}

pub fn parse_version(output: &str) -> Result<String, BackendError> {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("ddcutil version: "))
        .or_else(|| {
            output
                .lines()
                .find_map(|line| line.trim().strip_prefix("ddcutil "))
        })
        .map(str::to_owned)
        .ok_or_else(|| BackendError::Parse(output.trim().to_owned()))
}

fn require_supported_version(version: &str) -> Result<(), BackendError> {
    let mut parts = version.split('.').map(|part| {
        part.chars()
            .take_while(|character| character.is_ascii_digit())
            .collect::<String>()
            .parse::<u32>()
            .unwrap_or(0)
    });
    let parsed = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    if parsed < (2, 2, 1) {
        Err(BackendError::UnsupportedVersion {
            found: version.to_owned(),
        })
    } else {
        Ok(())
    }
}

pub fn parse_detect_output(output: &str) -> Vec<Monitor> {
    #[derive(Default)]
    struct Pending {
        valid: bool,
        bus: Option<u32>,
        connector: String,
        identity: String,
    }

    fn finish(pending: Pending, monitors: &mut Vec<Monitor>) {
        let Some(bus) = pending.bus else { return };
        if !pending.valid || pending.identity.is_empty() {
            return;
        }

        let mut parts = pending.identity.splitn(3, ':');
        let manufacturer = parts.next().unwrap_or_default().trim().to_owned();
        let model = parts.next().unwrap_or_default().trim().to_owned();
        let serial = parts.next().unwrap_or_default().trim().to_owned();
        let name = if model.is_empty() {
            manufacturer.clone()
        } else {
            model.clone()
        };
        let identity = if serial.is_empty() {
            format!("{manufacturer}-{model}-{}", pending.connector)
        } else {
            format!("{manufacturer}-{model}-{serial}")
        };
        let id = identity
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .split('-')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("-");

        monitors.push(Monitor {
            id,
            name,
            manufacturer,
            model,
            serial,
            connector: pending.connector,
            bus,
            controls: Vec::new(),
        });
    }

    let mut monitors = Vec::new();
    let mut pending = Pending::default();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Display ") || trimmed == "Invalid display" {
            finish(pending, &mut monitors);
            pending = Pending {
                valid: trimmed.starts_with("Display "),
                ..Pending::default()
            };
        } else if let Some(value) = trimmed.strip_prefix("I2C bus:") {
            pending.bus = value
                .trim()
                .strip_prefix("/dev/i2c-")
                .and_then(|value| value.parse().ok());
        } else if let Some(value) = trimmed.strip_prefix("DRM connector:") {
            pending.connector = value.trim().to_owned();
        } else if let Some(value) = trimmed.strip_prefix("Monitor:") {
            pending.identity = value.trim().to_owned();
        }
    }
    finish(pending, &mut monitors);
    monitors
}

pub fn parse_vcp_output(output: &str) -> Vec<Control> {
    output.lines().filter_map(parse_vcp_line).collect()
}

fn parse_vcp_line(line: &str) -> Option<Control> {
    let fields: Vec<_> = line.split_whitespace().collect();
    if fields.first().copied() != Some("VCP") {
        return None;
    }

    let code = u8::from_str_radix(fields.get(1)?.trim_start_matches("0x"), 16).ok()?;
    let definition = feature_definition(code)?;
    let kind = fields.get(2).copied()?;
    let choices = choices_for(code);
    let numbers: Vec<u16> = fields[3..]
        .iter()
        .filter_map(|field| parse_number(field))
        .collect();

    let (current, maximum) = match kind {
        "C" if numbers.len() >= 2 && numbers[1] > 0 => (numbers[0], numbers[1]),
        "NC" | "SNC" | "XNC" if !numbers.is_empty() => (
            *numbers.last()?,
            choices
                .iter()
                .map(|choice| choice.value)
                .max()
                .unwrap_or(*numbers.last()?),
        ),
        _ => return None,
    };

    Some(Control {
        code,
        key: definition.key.to_owned(),
        title: definition.title.to_owned(),
        kind: definition.kind,
        current,
        maximum,
        writable: true,
        choices,
    })
}

fn parse_number(value: &str) -> Option<u16> {
    let value =
        value.trim_matches(|character: char| !character.is_ascii_hexdigit() && character != 'x');
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix('x'))
        .and_then(|hex| u16::from_str_radix(hex, 16).ok())
        .or_else(|| value.parse().ok())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn parses_only_valid_external_displays() {
        let output = r#"
Invalid display
   I2C bus:          /dev/i2c-11
   DRM connector:    card1-eDP-1
   Monitor:          TMX:TL156VDXP0101:

Display 1
   I2C bus:          /dev/i2c-7
   DRM connector:    card0-HDMI-A-1
   Monitor:          GSM:LG ULTRAGEAR:TESTSERIAL
"#;
        let monitors = parse_detect_output(output);
        assert_eq!(monitors.len(), 1);
        assert_eq!(monitors[0].id, "gsm-lg-ultragear-testserial");
        assert_eq!(monitors[0].name, "LG ULTRAGEAR");
        assert_eq!(monitors[0].bus, 7);
    }

    #[test]
    fn parses_continuous_and_non_continuous_features() {
        let output = "VCP 10 C 15 100\nVCP 14 SNC x00 x0b\nVCP 8D NC 0x02\n";
        let controls = parse_vcp_output(output);
        assert_eq!(controls.len(), 3);
        assert_eq!(controls[0].current, 15);
        assert_eq!(controls[0].maximum, 100);
        assert_eq!(controls[1].current, 11);
        assert_eq!(controls[1].maximum, 11);
        assert_eq!(controls[2].current, 2);
        assert_eq!(controls[2].maximum, 2);
    }

    #[test]
    fn ignores_unknown_and_diagnostic_lines() {
        let output = "VCP 60 SNC x00 x01\nInvalid value\nVCP 12 C 0 0\nVCP 10 C 20 100\n";
        let controls = parse_vcp_output(output);
        assert_eq!(controls.len(), 1);
        assert_eq!(controls[0].code, 0x10);
    }

    #[test]
    fn parses_supported_version_formats() {
        assert_eq!(parse_version("ddcutil version: 2.2.1\n").unwrap(), "2.2.1");
        assert_eq!(parse_version("ddcutil 2.2.7\n").unwrap(), "2.2.7");
        assert!(require_supported_version("2.2.1").is_ok());
        assert!(require_supported_version("2.2.7-dev").is_ok());
        assert!(matches!(
            require_supported_version("2.1.4"),
            Err(BackendError::UnsupportedVersion { .. })
        ));
    }
}
