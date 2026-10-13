//! Keyboard and mouse, expressed as the same Xbox pad gameplay already reads.
//! The right stick eases toward key targets so a down-then-up tap is a flick
//! the gesture recognizer can see, instead of a one-sample jump.
use super::platform::{DevicePacket, BUTTON_NAMES};
use crate::graphics_menu::gameplay_active;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use serde::{Deserialize, Serialize};
use skate_core::input::xbox::XboxState;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const STICK_STEP: f32 = 0.5;
const SENSITIVITY_MIN: f32 = 0.002;
const SENSITIVITY_MAX: f32 = 0.08;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Key(KeyCode),
    Mouse(MouseButton),
}

#[derive(Clone, Debug)]
struct Binding {
    action: &'static str,
    sources: Vec<Source>,
}

fn defaults() -> Vec<Binding> {
    use KeyCode::*;
    use Source::*;
    let key = |action, keys: &[KeyCode]| Binding {
        action,
        sources: keys.iter().copied().map(Key).collect(),
    };
    vec![
        key("steer_forward", &[KeyW]),
        key("steer_back", &[KeyS]),
        key("steer_left", &[KeyA]),
        key("steer_right", &[KeyD]),
        key("trick_up", &[ArrowUp, KeyI]),
        key("trick_down", &[ArrowDown, KeyK]),
        key("trick_left", &[ArrowLeft, KeyJ]),
        key("trick_right", &[ArrowRight, KeyL]),
        key("push", &[Space]),
        key("push_mongo", &[ControlLeft]),
        key("brake", &[KeyQ]),
        key("offboard", &[KeyF]),
        Binding { action: "grab_left", sources: vec![Mouse(MouseButton::Left), Key(ShiftLeft)] },
        Binding { action: "grab_right", sources: vec![Mouse(MouseButton::Right), Key(KeyE)] },
        key("shoulder_left", &[KeyZ]),
        key("shoulder_right", &[KeyC]),
        key("bail", &[KeyR]),
        key("dpad_up", &[Digit1]),
        key("dpad_down", &[Digit2]),
        key("dpad_left", &[Digit3]),
        key("dpad_right", &[Digit4]),
    ]
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct KeyboardFile {
    mouse: bool,
    invert_y: bool,
    sensitivity: f32,
    bindings: BTreeMap<String, Vec<String>>,
}

impl Default for KeyboardFile {
    fn default() -> Self {
        Self {
            mouse: true,
            invert_y: false,
            sensitivity: 0.015,
            bindings: defaults().into_iter().map(|binding| {
                (binding.action.to_string(), binding.sources.iter().map(source_name).collect())
            }).collect(),
        }
    }
}

fn source_name(source: &Source) -> String {
    match source {
        Source::Key(key) => key_name(*key).to_string(),
        Source::Mouse(MouseButton::Left) => "MouseLeft".into(),
        Source::Mouse(MouseButton::Right) => "MouseRight".into(),
        Source::Mouse(MouseButton::Middle) => "MouseMiddle".into(),
        Source::Mouse(_) => "MouseOther".into(),
    }
}

fn key_name(key: KeyCode) -> &'static str {
    use KeyCode::*;
    match key {
        KeyW => "W", KeyA => "A", KeyS => "S", KeyD => "D", KeyQ => "Q", KeyE => "E",
        KeyF => "F", KeyZ => "Z", KeyC => "C", KeyR => "R", KeyI => "I", KeyJ => "J",
        KeyK => "K", KeyL => "L", Space => "Space", ControlLeft => "LeftCtrl",
        ShiftLeft => "LeftShift", ArrowUp => "ArrowUp", ArrowDown => "ArrowDown",
        ArrowLeft => "ArrowLeft", ArrowRight => "ArrowRight", Digit1 => "1",
        Digit2 => "2", Digit3 => "3", Digit4 => "4", _ => "Unknown",
    }
}

fn parse_source(name: &str) -> Option<Source> {
    use KeyCode::*;
    Some(match name {
        "W" => Source::Key(KeyW), "A" => Source::Key(KeyA), "S" => Source::Key(KeyS),
        "D" => Source::Key(KeyD), "Q" => Source::Key(KeyQ), "E" => Source::Key(KeyE),
        "F" => Source::Key(KeyF), "Z" => Source::Key(KeyZ), "C" => Source::Key(KeyC),
        "R" => Source::Key(KeyR), "I" => Source::Key(KeyI), "J" => Source::Key(KeyJ),
        "K" => Source::Key(KeyK), "L" => Source::Key(KeyL), "Space" => Source::Key(Space),
        "LeftCtrl" => Source::Key(ControlLeft), "LeftShift" => Source::Key(ShiftLeft),
        "ArrowUp" => Source::Key(ArrowUp), "ArrowDown" => Source::Key(ArrowDown),
        "ArrowLeft" => Source::Key(ArrowLeft), "ArrowRight" => Source::Key(ArrowRight),
        "1" => Source::Key(Digit1), "2" => Source::Key(Digit2), "3" => Source::Key(Digit3),
        "4" => Source::Key(Digit4), "MouseLeft" => Source::Mouse(MouseButton::Left),
        "MouseRight" => Source::Mouse(MouseButton::Right),
        "MouseMiddle" => Source::Mouse(MouseButton::Middle),
        _ => return None,
    })
}

#[derive(Resource, Clone, Debug)]
pub(crate) struct VirtualPad {
    pub mouse: bool,
    pub invert_y: bool,
    pub sensitivity: f32,
    bindings: Vec<Binding>,
    path: PathBuf,
    right: [f32; 2],
    mouse_pending: Vec2,
}

impl Default for VirtualPad {
    fn default() -> Self {
        Self::from_file(KeyboardFile::default(), PathBuf::new())
    }
}

impl VirtualPad {
    fn from_file(file: KeyboardFile, path: PathBuf) -> Self {
        let mut bindings = defaults();
        for binding in &mut bindings {
            if let Some(names) = file.bindings.get(binding.action) {
                let sources: Vec<Source> = names.iter().filter_map(|name| parse_source(name)).collect();
                if sources.len() == names.len() && !sources.is_empty() {
                    binding.sources = sources;
                } else {
                    warn!("keyboard.json: keeping the default keys for {}", binding.action);
                }
            }
        }
        Self {
            mouse: file.mouse,
            invert_y: file.invert_y,
            sensitivity: file.sensitivity.clamp(SENSITIVITY_MIN, SENSITIVITY_MAX),
            bindings,
            path,
            right: [0.0, 0.0],
            mouse_pending: Vec2::ZERO,
        }
    }

    pub(crate) fn load(asset_root: &Path) -> Self {
        let path = asset_root.parent().unwrap_or(asset_root).join("settings/keyboard.json");
        let file = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<KeyboardFile>(&bytes) {
                Ok(file) => file,
                Err(error) => {
                    warn!("Invalid keyboard settings {}: {error}", path.display());
                    KeyboardFile::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => KeyboardFile::default(),
            Err(error) => {
                warn!("Cannot read keyboard settings {}: {error}", path.display());
                KeyboardFile::default()
            }
        };
        Self::from_file(file, path)
    }

    pub(crate) fn save(&self) -> Result<(), String> {
        if self.path.as_os_str().is_empty() {
            return Err("Keyboard settings have no save path".into());
        }
        let file = KeyboardFile {
            mouse: self.mouse,
            invert_y: self.invert_y,
            sensitivity: self.sensitivity,
            bindings: self.bindings.iter().map(|binding| {
                (binding.action.to_string(), binding.sources.iter().map(source_name).collect())
            }).collect(),
        };
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&self.path, serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }

    pub(crate) fn adjust(&mut self, row: usize, direction: i32, coarse: bool) -> String {
        match row {
            4 => {
                let step = if coarse { 0.01 } else { 0.001 };
                self.sensitivity = (self.sensitivity + direction as f32 * step).clamp(SENSITIVITY_MIN, SENSITIVITY_MAX);
                format!("Mouse sensitivity {:.3}", self.sensitivity)
            }
            9 => {
                self.invert_y = !self.invert_y;
                format!("Invert vertical {}", if self.invert_y { "on" } else { "off" })
            }
            15 => {
                self.mouse = !self.mouse;
                format!("Mouse tricks {}", if self.mouse { "on" } else { "off" })
            }
            _ => String::new(),
        }
    }

    pub(crate) fn label(&self, row: usize) -> String {
        match row {
            4 => format!("Mouse sensitivity    {:.3}", self.sensitivity),
            9 => format!("Invert vertical       {}", if self.invert_y { "On" } else { "Off" }),
            15 => format!("Mouse tricks          {}", if self.mouse { "On" } else { "Off" }),
            _ => String::new(),
        }
    }

    fn down(&self, action: &str, keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>) -> bool {
        self.bindings.iter().find(|binding| binding.action == action).is_some_and(|binding| {
            binding.sources.iter().any(|source| match source {
                Source::Key(key) => keys.pressed(*key),
                Source::Mouse(button) => mouse.pressed(*button),
            })
        })
    }

    /// Move the right stick one gameplay tick and return it in -1..1.
    pub(crate) fn step_right(&mut self, target: [f32; 2], mouse: Vec2) -> [f32; 2] {
        for axis in 0..2 {
            let delta = target[axis] - self.right[axis];
            self.right[axis] += delta.clamp(-STICK_STEP, STICK_STEP);
        }
        let y_sign = if self.invert_y { -1.0 } else { 1.0 };
        self.right[0] = (self.right[0] + mouse.x * self.sensitivity).clamp(-1.0, 1.0);
        self.right[1] = (self.right[1] + mouse.y * self.sensitivity * y_sign).clamp(-1.0, 1.0);
        self.right
    }

    fn stick_target(&self, keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>) -> [f32; 2] {
        let axis = |negative, positive| match (negative, positive) {
            (true, false) => -1.0,
            (false, true) => 1.0,
            _ => 0.0,
        };
        [
            axis(self.down("trick_left", keys, mouse), self.down("trick_right", keys, mouse)),
            axis(self.down("trick_down", keys, mouse), self.down("trick_up", keys, mouse)),
        ]
    }

    pub(crate) fn xbox_state(&self, keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>) -> XboxState {
        let mut buttons = 0u16;
        let mut set = |action: &str, name: &str| {
            if self.down(action, keys, mouse) {
                if let Some((_, bit)) = BUTTON_NAMES.iter().find(|(n, _)| *n == name) {
                    buttons |= bit;
                }
            }
        };
        set("push", "a");
        set("push_mongo", "x");
        set("brake", "b");
        set("offboard", "y");
        set("shoulder_left", "left_shoulder");
        set("shoulder_right", "right_shoulder");
        set("dpad_up", "dpad_up");
        set("dpad_down", "dpad_down");
        set("dpad_left", "dpad_left");
        set("dpad_right", "dpad_right");
        if self.down("bail", keys, mouse) {
            buttons |= button_bit("left_stick") | button_bit("right_stick");
        }
        let axis = |negative, positive| -> i16 {
            match (self.down(negative, keys, mouse), self.down(positive, keys, mouse)) {
                (true, false) => -32767,
                (false, true) => 32767,
                _ => 0,
            }
        };
        let to_stick = |value: f32| (value.clamp(-1.0, 1.0) * 32767.0) as i16;
        XboxState {
            buttons,
            triggers: [
                if self.down("grab_left", keys, mouse) { 255 } else { 0 },
                if self.down("grab_right", keys, mouse) { 255 } else { 0 },
            ],
            left: [
                axis("steer_left", "steer_right"),
                axis("steer_back", "steer_forward"),
            ],
            right: [to_stick(self.right[0]), to_stick(self.right[1])],
        }
    }
}

fn button_bit(name: &str) -> u16 {
    BUTTON_NAMES.iter().find(|(n, _)| *n == name).map(|(_, bit)| *bit).unwrap_or(0)
}

/// Buttons combine. Each stick axis keeps whichever input is pushed further.
pub(crate) fn merge_state(into: &mut XboxState, from: &XboxState) {
    into.buttons |= from.buttons;
    for (slot, value) in into.triggers.iter_mut().zip(from.triggers) {
        *slot = (*slot).max(value);
    }
    for (slot, value) in into.left.iter_mut().zip(from.left) {
        if value.unsigned_abs() > slot.unsigned_abs() {
            *slot = value;
        }
    }
    for (slot, value) in into.right.iter_mut().zip(from.right) {
        if value.unsigned_abs() > slot.unsigned_abs() {
            *slot = value;
        }
    }
}

pub(crate) fn keyboard_packet(state: XboxState) -> DevicePacket {
    DevicePacket { number: 1, state, subtype: 1, kind: None }
}

pub(crate) fn accumulate_mouse(
    motion: Res<AccumulatedMouseMotion>,
    mut pad: ResMut<VirtualPad>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    if !pad.mouse || !gameplay_active(menu) || !windows.iter().any(|window| window.focused) {
        return;
    }
    pad.mouse_pending += motion.delta;
}

pub(crate) fn step_stick(
    mut pad: ResMut<VirtualPad>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let playing = gameplay_active(menu) && windows.iter().any(|window| window.focused);
    if !playing {
        pad.right = [0.0, 0.0];
        pad.mouse_pending = Vec2::ZERO;
        return;
    }
    let target = pad.stick_target(&keys, &mouse);
    let pending = std::mem::take(&mut pad.mouse_pending);
    pad.step_right(target, pending);
}

pub(crate) fn capture_cursor(
    pad: Res<VirtualPad>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
    mut windows: Query<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
) {
    let Ok((window, mut cursor)) = windows.single_mut() else { return };
    let capture = pad.mouse && gameplay_active(menu) && window.focused;
    cursor.grab_mode = if capture { CursorGrabMode::Locked } else { CursorGrabMode::None };
    cursor.visible = !capture;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn down_then_up_eases_through_the_middle() {
        let mut pad = VirtualPad::default();
        let mut samples = Vec::new();
        for _ in 0..6 {
            samples.push(pad.step_right([0.0, -1.0], Vec2::ZERO));
        }
        for _ in 0..8 {
            samples.push(pad.step_right([0.0, 1.0], Vec2::ZERO));
        }
        assert!(samples.iter().any(|sample| sample[1] < -0.9), "{samples:?}");
        assert!(samples.windows(2).any(|pair| pair[0][1] < -0.4 && pair[1][1] > -0.1 && pair[1][1] < 0.6));
        assert!(samples.last().unwrap()[1] > 0.9);
    }

    #[test]
    fn down_then_up_registers_an_ollie() {
        let Some(assets) = installed_assets() else {
            eprintln!("skipping ollie gesture test; converted assets were not found");
            return;
        };
        let mut gestures = crate::input::gesture_input::GestureInput::load(&assets).expect("gesture patterns");
        let mut intents = skate_core::graph::intents::IntentMap::new();
        let mut pad = VirtualPad::default();
        let mut names = Vec::new();
        for target in [[0.0, -1.0]; 8].into_iter().chain([[0.0, 1.0]; 10]) {
            let sample = pad.step_right(target, Vec2::ZERO);
            for (name, _) in gestures.publish([[0.0, 0.0], sample], 0, 0, 0, &mut intents) {
                names.push(name);
            }
        }
        assert!(
            names.iter().any(|name| name == "Ollie"),
            "expected Ollie, got {names:?}"
        );
    }

    fn installed_assets() -> Option<std::path::PathBuf> {
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3)?;
        let installations = workspace.join("skate3rust-windows-x64/data/installations");
        for entry in std::fs::read_dir(installations).ok()?.flatten() {
            let assets = entry.path().join("assets");
            if assets.join("private/stock/data/joystick/skater.pat").is_file() {
                return Some(assets);
            }
        }
        None
    }
}
