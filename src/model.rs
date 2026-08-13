use serde::{Deserialize, Serialize};

pub const API_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeatureDefinition {
    pub code: u8,
    pub key: &'static str,
    pub title: &'static str,
    pub kind: ControlKind,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    Continuous,
    Toggle,
    Choice,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Control {
    pub code: u8,
    pub key: String,
    pub title: String,
    pub kind: ControlKind,
    pub current: u16,
    pub maximum: u16,
    pub writable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Choice>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Choice {
    pub value: u16,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Monitor {
    pub id: String,
    pub name: String,
    pub manufacturer: String,
    pub model: String,
    pub serial: String,
    pub connector: String,
    pub bus: u32,
    #[serde(default)]
    pub controls: Vec<Control>,
}

impl Monitor {
    pub fn control(&self, code: u8) -> Option<&Control> {
        self.controls.iter().find(|control| control.code == code)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceState {
    pub api_version: u32,
    pub ready: bool,
    pub ddcutil_version: Option<String>,
    pub error: Option<String>,
    pub monitors: Vec<Monitor>,
}

impl Default for ServiceState {
    fn default() -> Self {
        Self {
            api_version: API_VERSION,
            ready: false,
            ddcutil_version: None,
            error: None,
            monitors: Vec::new(),
        }
    }
}

pub const BRIGHTNESS: u8 = 0x10;

pub const SAFE_FEATURES: &[FeatureDefinition] = &[
    FeatureDefinition {
        code: BRIGHTNESS,
        key: "brightness",
        title: "Brightness",
        kind: ControlKind::Continuous,
    },
    FeatureDefinition {
        code: 0x12,
        key: "contrast",
        title: "Contrast",
        kind: ControlKind::Continuous,
    },
    FeatureDefinition {
        code: 0x62,
        key: "volume",
        title: "Monitor volume",
        kind: ControlKind::Continuous,
    },
    FeatureDefinition {
        code: 0x8d,
        key: "mute",
        title: "Monitor mute",
        kind: ControlKind::Toggle,
    },
    FeatureDefinition {
        code: 0x14,
        key: "color_preset",
        title: "Colour preset",
        kind: ControlKind::Choice,
    },
    FeatureDefinition {
        code: 0x16,
        key: "red_gain",
        title: "Red gain",
        kind: ControlKind::Continuous,
    },
    FeatureDefinition {
        code: 0x18,
        key: "green_gain",
        title: "Green gain",
        kind: ControlKind::Continuous,
    },
    FeatureDefinition {
        code: 0x1a,
        key: "blue_gain",
        title: "Blue gain",
        kind: ControlKind::Continuous,
    },
];

pub fn feature_definition(code: u8) -> Option<&'static FeatureDefinition> {
    SAFE_FEATURES.iter().find(|feature| feature.code == code)
}

pub fn choices_for(code: u8) -> Vec<Choice> {
    let values: &[(u16, &str)] = match code {
        0x8d => &[(1, "Muted"), (2, "Unmuted")],
        0x14 => &[
            (1, "sRGB"),
            (4, "5000 K"),
            (5, "6500 K"),
            (6, "7500 K"),
            (7, "8200 K"),
            (8, "9300 K"),
            (10, "11500 K"),
            (11, "User 1"),
        ],
        _ => &[],
    };

    values
        .iter()
        .map(|(value, label)| Choice {
            value: *value,
            label: (*label).to_owned(),
        })
        .collect()
}
