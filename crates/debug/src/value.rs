//! Reading command arguments and building replies.

use genos_mcp::json::{self, Value};
use genos_scene::Vec3;

pub fn obj(pairs: Vec<(&str, Value)>) -> Value {
    Value::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

pub fn num(v: f64) -> Value {
    // Short decimals keep replies readable.
    json::float((v * 1.0e5).round() / 1.0e5)
}

pub fn vec3(v: [f32; 3]) -> Value {
    json::array(v.iter().map(|c| num(*c as f64)).collect())
}

pub fn list(items: Vec<Value>) -> Value {
    json::array(items)
}

pub fn text(t: impl Into<String>) -> Value {
    json::string(t)
}

/// The arguments of one command, with errors that name the field.
pub struct Args<'a>(pub &'a Value);

impl Args<'_> {
    pub fn f64(&self, key: &str) -> Result<Option<f64>, String> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_f64()
                .map(Some)
                .ok_or_else(|| format!("{key} is a number")),
        }
    }

    pub fn f32(&self, key: &str) -> Result<Option<f32>, String> {
        Ok(self.f64(key)?.map(|v| v as f32))
    }

    pub fn u32(&self, key: &str) -> Result<Option<u32>, String> {
        Ok(self.f64(key)?.map(|v| v.max(0.0).round() as u32))
    }

    pub fn bool(&self, key: &str) -> Result<Option<bool>, String> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_bool()
                .or_else(|| v.as_f64().map(|n| n != 0.0))
                .map(Some)
                .ok_or_else(|| format!("{key} is true or false")),
        }
    }

    pub fn str(&self, key: &str) -> Result<Option<String>, String> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_str()
                .map(|s| Some(s.to_string()))
                .or_else(|| v.as_f64().map(|n| Some(n.to_string())))
                .ok_or_else(|| format!("{key} is a string")),
        }
    }

    pub fn floats(&self, key: &str, n: usize) -> Result<Option<Vec<f32>>, String> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => {
                let items = v
                    .as_array()
                    .ok_or_else(|| format!("{key} is a list of {n} numbers"))?;
                if items.len() != n {
                    return Err(format!("{key} is a list of {n} numbers"));
                }
                items
                    .iter()
                    .map(|i| i.as_f64().map(|f| f as f32))
                    .collect::<Option<Vec<f32>>>()
                    .map(Some)
                    .ok_or_else(|| format!("{key} is a list of {n} numbers"))
            }
        }
    }

    pub fn vec3(&self, key: &str) -> Result<Option<Vec3>, String> {
        Ok(self.floats(key, 3)?.map(|v| Vec3::new(v[0], v[1], v[2])))
    }

    pub fn rgb(&self, key: &str) -> Result<Option<[f32; 3]>, String> {
        Ok(self.floats(key, 3)?.map(|v| [v[0], v[1], v[2]]))
    }

    /// A list of points `[[x, y, z], ...]`.
    pub fn points(&self, key: &str) -> Result<Option<Vec<Vec3>>, String> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => {
                let bad = || format!("{key} is a list of [x, y, z] points");
                let items = v.as_array().ok_or_else(bad)?;
                let mut out = Vec::new();
                for item in items {
                    let p = item.as_array().filter(|p| p.len() == 3).ok_or_else(bad)?;
                    let c: Vec<f32> = p
                        .iter()
                        .map(|c| c.as_f64().map(|f| f as f32))
                        .collect::<Option<_>>()
                        .ok_or_else(bad)?;
                    out.push(Vec3::new(c[0], c[1], c[2]));
                }
                if out.is_empty() {
                    return Err(bad());
                }
                Ok(Some(out))
            }
        }
    }
}
