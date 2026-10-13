//! The one threshold file. `lighting-ref.conf` next to this crate.

use std::collections::BTreeMap;
use std::path::Path;

use crate::metrics::MetricParams;

pub const DEFAULT_TEXT: &str = include_str!("../lighting-ref.conf");

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub params: MetricParams,
    pub human_band: f64,
    pub disagree_delta: f64,
    pub noise_multiple: f64,
    pub noise_tail: f64,
    pub fail: BTreeMap<String, f64>,
    pub regress: BTreeMap<String, f64>,
}

impl Config {
    pub fn builtin() -> Self {
        Self::parse(DEFAULT_TEXT).expect("built-in lighting-ref.conf")
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut values = BTreeMap::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("line {}: expected key = value", n + 1));
            };
            let key = key.trim();
            let value = value.trim();
            let number = value
                .parse::<f64>()
                .map_err(|_| format!("line {}: {key} is not a number", n + 1))?;
            if !number.is_finite() || number < 0.0 {
                return Err(format!(
                    "line {}: {key} must be a finite number >= 0",
                    n + 1
                ));
            }
            values.insert(key.to_string(), number);
        }
        let take = |map: &mut BTreeMap<String, f64>, key: &str| -> Result<f64, String> {
            map.remove(key).ok_or_else(|| format!("missing {key}"))
        };
        let mut values = values;
        let params = MetricParams {
            rel_floor: take(&mut values, "rel_floor")?,
            dark_max: take(&mut values, "dark_max")?,
            unfilled_candidate_max: take(&mut values, "unfilled_candidate_max")?,
            unfilled_reference_min: take(&mut values, "unfilled_reference_min")?,
        };
        let human_band = take(&mut values, "human_band")?;
        let disagree_delta = take(&mut values, "disagree_delta")?;
        let noise_multiple = take(&mut values, "noise_multiple")?;
        let noise_tail = take(&mut values, "noise_tail")?;
        if human_band >= 1.0 {
            return Err("human_band must be under 1".to_string());
        }
        let mut fail = BTreeMap::new();
        let mut regress = BTreeMap::new();
        for (key, value) in values {
            if let Some(name) = key.strip_prefix("fail.") {
                fail.insert(name.to_string(), value);
            } else if let Some(name) = key.strip_prefix("regress.") {
                regress.insert(name.to_string(), value);
            } else {
                return Err(format!("unknown threshold {key}"));
            }
        }
        let config = Self {
            params,
            human_band,
            disagree_delta,
            noise_multiple,
            noise_tail,
            fail,
            regress,
        };
        for id in crate::verdict::MetricId::all() {
            if !config.fail.contains_key(id.key()) || !config.regress.contains_key(id.key()) {
                return Err(format!("missing fail.{} or regress.{}", id.key(), id.key()));
            }
        }
        Ok(config)
    }

    pub fn fail_of(&self, key: &str) -> f64 {
        self.fail[key]
    }

    pub fn regress_of(&self, key: &str) -> f64 {
        self.regress[key]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builtin_file_parses() {
        let config = Config::builtin();
        assert!((config.fail_of("mean_rel") - 0.08).abs() < 1.0e-9);
        assert!((config.human_band - 0.15).abs() < 1.0e-9);
        assert!((config.noise_tail - 4.0).abs() < 1.0e-9);
        assert!((config.fail_of("luma_delta") - 0.10).abs() < 1.0e-9);
        assert!((config.fail_of("blur_rel") - 0.08).abs() < 1.0e-9);
        assert!((config.params.rel_floor - 0.02).abs() < 1.0e-9);
    }

    #[test]
    fn a_typo_is_an_error() {
        let text = "rel_floor = 0.02\nnope = 1\n";
        assert!(Config::parse(text).is_err());
    }
}
