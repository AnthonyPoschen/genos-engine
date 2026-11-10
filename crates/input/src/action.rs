#![expect(
    clippy::format_push_string,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::missing_errors_doc,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::must_use_candidate,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]

use crate::code::InputCode;
use crate::device::Axis2d;
use crate::system::InputSystem;
use crate::touch::{Gesture, GestureKind};

#[derive(Clone, Debug)]
pub struct BoundInput {
    pub code: InputCode,
    pub activation_threshold: Option<f32>,
    /// Scale applied after the stick deadzone. `1` leaves the value alone.
    /// Written only through [`Self::set_sensitivity`], which clamps to `-8..=8`.
    sensitivity: f32,
}

impl BoundInput {
    #[must_use]
    pub fn new(code: InputCode) -> Self {
        Self {
            code,
            activation_threshold: None,
            sensitivity: 1.0,
        }
    }

    #[must_use]
    pub fn sensitivity(&self) -> f32 {
        self.sensitivity
    }

    /// # Errors
    ///
    /// Returns an error when `sensitivity` is not finite. A finite value is
    /// clamped to `-8..=8`.
    pub fn set_sensitivity(&mut self, sensitivity: f32) -> Result<(), String> {
        self.sensitivity = finite_sensitivity(sensitivity)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Action2dBinding {
    pub left: Vec<BoundInput>,
    pub right: Vec<BoundInput>,
    pub up: Vec<BoundInput>,
    pub down: Vec<BoundInput>,
    pub vectors: Vec<BoundInput>,
}

#[derive(Clone, Debug)]
enum ActionKind {
    Codes(Vec<BoundInput>),
    Axis2d(Action2dBinding),
}

#[derive(Clone, Debug)]
struct Action {
    name: String,
    enabled: bool,
    /// Scale remembered so a sensitivity line can precede the binding line.
    sensitivity: f32,
    kind: ActionKind,
    gesture: Option<GestureKind>,
}

/// Named bindings evaluated against an [`InputSystem`].
#[derive(Clone, Debug, Default)]
pub struct ActionMap {
    actions: Vec<Action>,
}

impl ActionMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, name: &str, codes: Vec<BoundInput>) {
        self.insert(name, ActionKind::Codes(codes));
    }

    pub fn set_2d(&mut self, name: &str, binding: Action2dBinding) {
        self.insert(name, ActionKind::Axis2d(binding));
    }

    /// Replace the button bindings for `name`, or add the action.
    pub fn rebind(&mut self, name: &str, codes: Vec<BoundInput>) {
        self.set(name, codes);
    }

    /// Replace the axis bindings for `name`, or add the action.
    pub fn rebind_2d(&mut self, name: &str, binding: Action2dBinding) {
        self.set_2d(name, binding);
    }

    /// Replace one side of a 2D action. A missing action becomes a 2D action.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is a button action or `side` is unknown.
    pub fn rebind_side(
        &mut self,
        name: &str,
        side: &str,
        codes: Vec<BoundInput>,
    ) -> Result<(), String> {
        if !self.actions.iter().any(|action| action.name == name) {
            self.set_2d(name, Action2dBinding::default());
        }
        let action = self
            .actions
            .iter_mut()
            .find(|action| action.name == name)
            .ok_or_else(|| format!("unknown action {name}"))?;
        let scale = action.sensitivity;
        let ActionKind::Axis2d(binding) = &mut action.kind else {
            return Err(format!("{name} is a button action"));
        };
        let slot = match side {
            "left" => &mut binding.left,
            "right" => &mut binding.right,
            "up" => &mut binding.up,
            "down" => &mut binding.down,
            "vectors" => &mut binding.vectors,
            _ => return Err(format!("unknown axis side {side}")),
        };
        *slot = codes;
        stamp_unset(slot, scale);
        Ok(())
    }

    /// Set every binding on `name` to the same sensitivity.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is not finite. A missing action becomes
    /// an axis, so a side line can follow the sensitivity line.
    pub fn set_sensitivity(&mut self, name: &str, sensitivity: f32) -> Result<(), String> {
        let sensitivity = finite_sensitivity(sensitivity)?;
        if !self.actions.iter().any(|action| action.name == name) {
            self.set_2d(name, Action2dBinding::default());
        }
        let action = self
            .actions
            .iter_mut()
            .find(|action| action.name == name)
            .ok_or_else(|| format!("unknown action {name}"))?;
        action.sensitivity = sensitivity;
        match &mut action.kind {
            ActionKind::Codes(codes) => {
                for code in codes {
                    code.sensitivity = sensitivity;
                }
            }
            ActionKind::Axis2d(binding) => {
                for codes in [
                    &mut binding.left,
                    &mut binding.right,
                    &mut binding.up,
                    &mut binding.down,
                    &mut binding.vectors,
                ] {
                    for code in codes {
                        code.sensitivity = sensitivity;
                    }
                }
            }
        }
        Ok(())
    }

    /// Enable or disable one action. A disabled action reads as off.
    ///
    /// # Errors
    ///
    /// Returns an error when the action is missing.
    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> Result<(), String> {
        if !self.actions.iter().any(|action| action.name == name) {
            self.set(name, Vec::new());
        }
        let action = self
            .actions
            .iter_mut()
            .find(|action| action.name == name)
            .ok_or_else(|| format!("unknown action {name}"))?;
        action.enabled = enabled;
        Ok(())
    }

    /// Overlay one settings key. A bare name replaces button bindings.
    /// `move.left` replaces one axis side. `look.sensitivity` scales that action.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown field, an unknown code, or a button action
    /// used with an axis side.
    pub fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        let key = key.trim();
        let value = value.trim();
        if let Some((name, field)) = key.split_once('.') {
            match field {
                "sensitivity" => {
                    let sensitivity = value
                        .parse::<f32>()
                        .map_err(|_| format!("sensitivity is a number, got {value}"))?;
                    self.set_sensitivity(name, sensitivity)?;
                }
                "enabled" => self.set_enabled(name, parse_enabled(value)?)?,
                "left" | "right" | "up" | "down" | "vectors" => {
                    self.rebind_side(name, field, parse_code_list(value)?)?;
                }
                "gesture" => self.set_gesture(name, parse_gesture(value)?)?,
                _ => return Err(format!("unknown action field {key}")),
            }
            return Ok(());
        }
        if value.is_empty() {
            return Err(format!("{key} has no bindings"));
        }
        self.rebind(key, parse_code_list(value)?);
        Ok(())
    }

    /// Settings lines for the actions. Scalars such as the deadzone live on
    /// [`crate::InputSettings`].
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for action in &self.actions {
            match &action.kind {
                ActionKind::Codes(codes) => {
                    if !codes.is_empty() {
                        out.push_str(&format!("{}={}\n", action.name, format_code_list(codes)));
                    }
                }
                ActionKind::Axis2d(binding) => {
                    for (side, codes) in [
                        ("left", &binding.left),
                        ("right", &binding.right),
                        ("up", &binding.up),
                        ("down", &binding.down),
                        ("vectors", &binding.vectors),
                    ] {
                        out.push_str(&format!(
                            "{}.{}={}\n",
                            action.name,
                            side,
                            format_code_list(codes)
                        ));
                    }
                }
            }
            if !action.enabled {
                out.push_str(&format!("{}.enabled=0\n", action.name));
            }
            if let Some(gesture) = action.gesture {
                out.push_str(&format!("{}.gesture={}\n", action.name, gesture.name()));
            }
        }
        out
    }

    /// Action names in binding order.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.actions
            .iter()
            .map(|action| action.name.clone())
            .collect()
    }

    /// Bind a tap, a swipe, or a pinch to `name`. `None` clears it.
    /// A missing action becomes a button action with no keys.
    ///
    /// # Errors
    ///
    /// Returns an error only when the action cannot be created.
    pub fn set_gesture(&mut self, name: &str, gesture: Option<GestureKind>) -> Result<(), String> {
        if !self.actions.iter().any(|action| action.name == name) {
            self.set(name, Vec::new());
        }
        let action = self
            .actions
            .iter_mut()
            .find(|action| action.name == name)
            .ok_or_else(|| format!("unknown action {name}"))?;
        action.gesture = gesture;
        Ok(())
    }

    /// Build a map from settings lines. An empty text is an empty map.
    ///
    /// # Errors
    ///
    /// Returns the first `set_key` error.
    pub fn parse_text(text: &str) -> Result<Self, String> {
        let mut map = Self::new();
        for raw in text.split(['\n', ';']) {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("use key=value, got {line}"));
            };
            map.set_key(key, value)?;
        }
        Ok(map)
    }

    /// The settings lines. This is the probe for the bindings in force.
    #[must_use]
    pub fn dump(&self) -> String {
        self.to_text()
    }

    pub fn down(&self, input: &InputSystem, name: &str) -> bool {
        let Some(action) = self.find(name) else {
            return false;
        };
        if !action.enabled {
            return false;
        }
        if action.gesture == Some(GestureKind::Tap)
            && input
                .gestures()
                .iter()
                .any(|gesture| matches!(gesture, Gesture::Tap { .. }))
        {
            return true;
        }
        let ActionKind::Codes(codes) = &action.kind else {
            return false;
        };
        codes.iter().any(|bound| code_down(input, bound.code))
    }

    pub fn axis_2d(&self, input: &InputSystem, name: &str) -> Axis2d {
        let Some(action) = self.find(name) else {
            return Axis2d::default();
        };
        if !action.enabled {
            return Axis2d::default();
        }
        let mut out = match &action.kind {
            ActionKind::Axis2d(binding) => eval_2d(input, binding),
            ActionKind::Codes(codes) => {
                let mut out = Axis2d::default();
                for bound in codes {
                    if let Some(value) = code_axis_2d(input, bound.code) {
                        out.x += value.x * bound.sensitivity;
                        out.y += value.y * bound.sensitivity;
                    }
                }
                clamp_axis(out)
            }
        };
        add_gesture(&mut out, input, action.gesture, action.sensitivity);
        clamp_axis(out)
    }

    pub fn to_json(&self) -> String {
        let mut out = String::from("{\"actions\":[");
        for (index, action) in self.actions.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(&format!(
                "{{\"name\":\"{}\",\"enabled\":{},",
                action.name, action.enabled
            ));
            match &action.kind {
                ActionKind::Codes(codes) => {
                    out.push_str("\"kind\":\"codes\",\"codes\":[");
                    write_codes(&mut out, codes);
                    out.push(']');
                    write_gesture_field(&mut out, action.gesture);
                    out.push('}');
                }
                ActionKind::Axis2d(binding) => {
                    out.push_str("\"kind\":\"axis_2d\"");
                    for (label, codes) in [
                        ("left", &binding.left),
                        ("right", &binding.right),
                        ("up", &binding.up),
                        ("down", &binding.down),
                        ("vectors", &binding.vectors),
                    ] {
                        out.push_str(&format!(",\"{label}\":["));
                        write_codes(&mut out, codes);
                        out.push(']');
                    }
                    write_gesture_field(&mut out, action.gesture);
                    out.push('}');
                }
            }
        }
        out.push_str("]}");
        out
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        // A small reader for the object this map writes. It is not a general JSON parser.
        let mut map = Self::new();
        for chunk in text.split("{\"name\":\"").skip(1) {
            let name = chunk.split('"').next().ok_or("missing name")?;
            let enabled = json_bool(chunk, "enabled");
            if chunk.contains("\"kind\":\"codes\"") {
                let codes = codes_after(chunk, "\"codes\":")?;
                map.set(name, codes);
            } else if chunk.contains("\"kind\":\"axis_2d\"") {
                map.set_2d(
                    name,
                    Action2dBinding {
                        left: codes_after(chunk, "\"left\":")?,
                        right: codes_after(chunk, "\"right\":")?,
                        up: codes_after(chunk, "\"up\":")?,
                        down: codes_after(chunk, "\"down\":")?,
                        vectors: codes_after(chunk, "\"vectors\":")?,
                    },
                );
            }
            let body = chunk.split("{\"name\":\"").next().unwrap_or(chunk);
            if let Some(action) = map.actions.iter_mut().find(|action| action.name == name) {
                if let Some(enabled) = enabled {
                    action.enabled = enabled;
                }
                if let Some(text) = json_string(body, "gesture") {
                    action.gesture = parse_gesture(text)?;
                }
            }
        }
        if map.actions.is_empty() {
            return Err("json contained no actions".into());
        }
        Ok(map)
    }

    fn insert(&mut self, name: &str, kind: ActionKind) {
        if let Some(existing) = self.actions.iter_mut().find(|action| action.name == name) {
            let scale = existing.sensitivity;
            existing.kind = kind;
            if (scale - 1.0).abs() > 1.0e-4 {
                match &mut existing.kind {
                    ActionKind::Codes(codes) => stamp_unset(codes, scale),
                    ActionKind::Axis2d(binding) => {
                        for codes in [
                            &mut binding.left,
                            &mut binding.right,
                            &mut binding.up,
                            &mut binding.down,
                            &mut binding.vectors,
                        ] {
                            stamp_unset(codes, scale);
                        }
                    }
                }
            }
        } else {
            self.actions.push(Action {
                name: name.to_string(),
                enabled: true,
                sensitivity: 1.0,
                kind,
                gesture: None,
            });
        }
    }

    fn find(&self, name: &str) -> Option<&Action> {
        self.actions.iter().find(|action| action.name == name)
    }
}

/// The character-controller map. `move` is WASD, IJKL, and the left stick. `look` is the right stick.
pub fn character_controller() -> ActionMap {
    let mut map = ActionMap::new();
    map.set_2d(
        "move",
        Action2dBinding {
            left: vec![bound(InputCode::key_a), bound(InputCode::key_j)],
            right: vec![bound(InputCode::key_d), bound(InputCode::key_l)],
            up: vec![bound(InputCode::key_w), bound(InputCode::key_i)],
            down: vec![bound(InputCode::key_s), bound(InputCode::key_k)],
            vectors: vec![bound(InputCode::gamepad_left_stick)],
        },
    );
    map.set_2d(
        "look",
        Action2dBinding {
            vectors: vec![bound(InputCode::gamepad_right_stick)],
            ..Action2dBinding::default()
        },
    );
    map.set("capture", vec![bound(InputCode::mouse_left)]);
    map.set("release", vec![bound(InputCode::key_escape)]);
    map
}

fn bound(code: InputCode) -> BoundInput {
    BoundInput::new(code)
}

fn eval_2d(input: &InputSystem, binding: &Action2dBinding) -> Axis2d {
    let mut out = Axis2d::default();
    out.x -= direction_scale(&binding.left, input);
    out.x += direction_scale(&binding.right, input);
    out.y += direction_scale(&binding.up, input);
    out.y -= direction_scale(&binding.down, input);
    for item in &binding.vectors {
        if let Some(value) = code_axis_2d(input, item.code) {
            out.x += value.x * item.sensitivity;
            out.y += value.y * item.sensitivity;
        }
    }
    clamp_axis(out)
}

/// The strongest pressed binding on this side. Nothing pressed is zero.
fn direction_scale(items: &[BoundInput], input: &InputSystem) -> f32 {
    items
        .iter()
        .filter(|item| code_down(input, item.code))
        .map(|item| item.sensitivity)
        .max_by(f32::total_cmp)
        .unwrap_or(0.0)
}

fn code_down(input: &InputSystem, code: InputCode) -> bool {
    if input.keyboard.down(code) || input.mouse.down(code) {
        return true;
    }
    input
        .gamepads
        .iter()
        .any(|pad| pad.view.connected && pad.down(code))
}

fn code_axis_2d(input: &InputSystem, code: InputCode) -> Option<Axis2d> {
    let mut sum = Axis2d::default();
    let mut found = false;
    for pad in input.gamepads.iter().filter(|pad| pad.view.connected) {
        if let Some(value) = pad.axis2d(code) {
            sum.x += value.x;
            sum.y += value.y;
            found = true;
        }
    }
    found.then_some(sum)
}

fn write_gesture_field(out: &mut String, gesture: Option<GestureKind>) {
    if let Some(gesture) = gesture {
        out.push_str(",\"gesture\":\"");
        out.push_str(gesture.name());
        out.push('"');
    }
}

fn add_gesture(
    out: &mut Axis2d,
    input: &InputSystem,
    gesture: Option<GestureKind>,
    sensitivity: f32,
) {
    match gesture {
        Some(GestureKind::Swipe) => {
            for item in input.gestures() {
                if let Gesture::Swipe { x, y, .. } = *item {
                    out.x += x * sensitivity;
                    out.y += y * sensitivity;
                }
            }
        }
        Some(GestureKind::Pinch) => {
            for item in input.gestures() {
                if let Gesture::Pinch { scale } = *item {
                    out.y += (scale - 1.0).clamp(-1.0, 1.0) * sensitivity;
                }
            }
        }
        Some(GestureKind::Tap) | None => {}
    }
}

fn parse_gesture(value: &str) -> Result<Option<GestureKind>, String> {
    if value.is_empty() || matches!(value.to_ascii_lowercase().as_str(), "off" | "none") {
        return Ok(None);
    }
    GestureKind::parse(value)
        .map(Some)
        .ok_or_else(|| format!("gesture is tap, swipe, pinch, or off, got {value}"))
}

fn clamp_axis(mut value: Axis2d) -> Axis2d {
    value.x = value.x.clamp(-1.0, 1.0);
    value.y = value.y.clamp(-1.0, 1.0);
    value
}

fn write_codes(out: &mut String, codes: &[BoundInput]) {
    for (index, code) in codes.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"code\":\"");
        out.push_str(crate::code::name(code.code));
        out.push('"');
        if (code.sensitivity - 1.0).abs() > 1.0e-4 {
            out.push_str(",\"sensitivity\":");
            out.push_str(&format_number(code.sensitivity));
        }
        out.push('}');
    }
}

fn stamp_unset(codes: &mut [BoundInput], sensitivity: f32) {
    for code in codes {
        if (code.sensitivity - 1.0).abs() <= 1.0e-4 {
            code.sensitivity = sensitivity;
        }
    }
}

fn codes_after(chunk: &str, label: &str) -> Result<Vec<BoundInput>, String> {
    let Some(rest) = chunk.split(label).nth(1) else {
        return Ok(Vec::new());
    };
    let list = rest.split(']').next().unwrap_or("");
    let mut codes = Vec::new();
    for object in list.split('{').skip(1) {
        let body = object.split('}').next().unwrap_or("");
        codes.push(binding_from_object(body)?);
    }
    Ok(codes)
}

fn binding_from_object(body: &str) -> Result<BoundInput, String> {
    let name = json_string(body, "code").ok_or("missing code")?;
    let code = crate::code::parse(name).ok_or_else(|| format!("unknown code {name}"))?;
    let mut item = bound(code);
    if let Some(number) = json_number(body, "sensitivity") {
        item.sensitivity = finite_sensitivity(number)?;
    }
    Ok(item)
}

fn value_after<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let marker = format!("\"{key}\"");
    let index = text.find(&marker)?;
    let after = text[index + marker.len()..].trim_start();
    Some(after.strip_prefix(':')?.trim_start())
}

fn json_string<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let after = value_after(text, key)?.strip_prefix('"')?;
    let end = after.find('"')?;
    Some(&after[..end])
}

fn json_number(text: &str, key: &str) -> Option<f32> {
    let after = value_after(text, key)?;
    let end = after
        .find([',', '}', ']', ' ', '\n', '\t'])
        .unwrap_or(after.len());
    after[..end].trim().parse().ok()
}

fn json_bool(text: &str, key: &str) -> Option<bool> {
    let after = value_after(text, key)?;
    if after.starts_with("true") {
        Some(true)
    } else if after.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn format_code_list(codes: &[BoundInput]) -> String {
    let mut out = String::new();
    for (index, code) in codes.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        out.push_str(crate::code::name(code.code));
        if (code.sensitivity - 1.0).abs() > 1.0e-4 {
            out.push('*');
            out.push_str(&format_number(code.sensitivity));
        }
    }
    out
}

fn parse_code_list(value: &str) -> Result<Vec<BoundInput>, String> {
    let mut codes = Vec::new();
    for token in value.split_whitespace() {
        let (code_name, scale) = token
            .split_once('*')
            .map_or((token, None), |(name, scale)| (name, Some(scale)));
        let code =
            crate::code::parse(code_name).ok_or_else(|| format!("unknown code {code_name}"))?;
        let mut item = bound(code);
        if let Some(scale) = scale {
            let sensitivity = scale
                .parse::<f32>()
                .map_err(|_| format!("bad sensitivity {scale}"))?;
            item.sensitivity = finite_sensitivity(sensitivity)?;
        }
        codes.push(item);
    }
    Ok(codes)
}

fn parse_enabled(value: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Ok(true),
        "0" | "false" | "off" | "no" => Ok(false),
        _ => Err(format!("enabled is on or off, got {value}")),
    }
}

fn finite_sensitivity(value: f32) -> Result<f32, String> {
    if !value.is_finite() {
        return Err("sensitivity must be finite".into());
    }
    Ok(value.clamp(-8.0, 8.0))
}

pub(crate) fn format_number(value: f32) -> String {
    let rounded = (value * 10_000.0).round() / 10_000.0;
    format!("{rounded}")
}
