use crate::code::InputCode;
use crate::device::Axis2d;
use crate::system::InputSystem;

#[derive(Clone, Debug)]
pub struct BoundInput {
    pub code: InputCode,
    pub activation_threshold: Option<f32>,
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
    kind: ActionKind,
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

    pub fn down(&self, input: &InputSystem, name: &str) -> bool {
        let Some(action) = self.find(name) else {
            return false;
        };
        if !action.enabled {
            return false;
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
        match &action.kind {
            ActionKind::Axis2d(binding) => eval_2d(input, binding),
            ActionKind::Codes(codes) => {
                let mut out = Axis2d::default();
                for bound in codes {
                    if let Some(value) = code_axis_2d(input, bound.code) {
                        out.x += value.x;
                        out.y += value.y;
                    }
                }
                clamp_axis(out)
            }
        }
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
                    out.push_str("]}");
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
            let enabled = chunk.contains("\"enabled\":true");
            if chunk.contains("\"kind\":\"codes\"") {
                let codes = codes_after(chunk, "\"codes\":")?;
                map.set(name, codes);
            } else if chunk.contains("\"kind\":\"axis_2d\"") {
                map.set_2d(
                    name,
                    Action2dBinding {
                        left: codes_after(chunk, "\"left\":").unwrap_or_default(),
                        right: codes_after(chunk, "\"right\":").unwrap_or_default(),
                        up: codes_after(chunk, "\"up\":").unwrap_or_default(),
                        down: codes_after(chunk, "\"down\":").unwrap_or_default(),
                        vectors: codes_after(chunk, "\"vectors\":").unwrap_or_default(),
                    },
                );
            }
            if let Some(action) = map.actions.iter_mut().find(|action| action.name == name) {
                action.enabled = enabled;
            }
        }
        if map.actions.is_empty() {
            return Err("json contained no actions".into());
        }
        Ok(map)
    }

    fn insert(&mut self, name: &str, kind: ActionKind) {
        if let Some(existing) = self.actions.iter_mut().find(|action| action.name == name) {
            existing.kind = kind;
            existing.enabled = true;
        } else {
            self.actions.push(Action {
                name: name.to_string(),
                enabled: true,
                kind,
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
    BoundInput {
        code,
        activation_threshold: None,
    }
}

fn eval_2d(input: &InputSystem, binding: &Action2dBinding) -> Axis2d {
    let mut out = Axis2d::default();
    if binding.left.iter().any(|item| code_down(input, item.code)) {
        out.x -= 1.0;
    }
    if binding.right.iter().any(|item| code_down(input, item.code)) {
        out.x += 1.0;
    }
    if binding.up.iter().any(|item| code_down(input, item.code)) {
        out.y += 1.0;
    }
    if binding.down.iter().any(|item| code_down(input, item.code)) {
        out.y -= 1.0;
    }
    for item in &binding.vectors {
        if let Some(value) = code_axis_2d(input, item.code) {
            out.x += value.x;
            out.y += value.y;
        }
    }
    clamp_axis(out)
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
        out.push_str(&format!(
            "{{\"code\":\"{}\"}}",
            crate::code::name(code.code)
        ));
    }
}

fn codes_after(chunk: &str, label: &str) -> Result<Vec<BoundInput>, String> {
    let rest = chunk.split(label).nth(1).ok_or("missing code list")?;
    let list = rest.split(']').next().unwrap_or("");
    let mut codes = Vec::new();
    for part in list.split("\"code\":\"") {
        if part.starts_with('[') || part.is_empty() {
            continue;
        }
        let name = part.split('"').next().unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let code = crate::code::parse(name).ok_or_else(|| format!("unknown code {name}"))?;
        codes.push(bound(code));
    }
    Ok(codes)
}
