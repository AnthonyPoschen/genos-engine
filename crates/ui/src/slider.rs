//! Slider values for a panel row.
//!
//! A numeric slider maps a pointer fraction onto `min`..`max`, snapping to `step`
//! when `step` is greater than zero. A named slider maps that fraction onto
//! evenly spaced steps and reports the step's own value, so the steps need not
//! be evenly spaced in value (1, 3, 5).

use crate::layout::Rect;
use crate::text::{self, Blot};

/// One named position. The panel draws `name` at the tick. A slide reports `value`.
#[derive(Clone, Debug, PartialEq)]
pub struct SliderStep {
    pub name: String,
    pub value: f64,
}

/// Dim tick label. The step under the thumb uses [`LABEL_HOT`].
pub const LABEL_DIM: [f32; 3] = [0.46, 0.48, 0.52];
/// The step the slider is on.
pub const LABEL_HOT: [f32; 3] = [0.96, 0.94, 0.86];

/// A tick label under the track. `x` and `y` are the top-left of the text.
#[derive(Clone, Debug, PartialEq)]
pub struct TickLabel {
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub current: bool,
}

/// How many small-text rows [`place_labels`] needs for this track width.
pub fn label_rows(steps: &[SliderStep], left: f32, right: f32) -> u32 {
    if steps.is_empty() {
        return 0;
    }
    let placed = place_labels(steps, None, left, right, 0.0);
    placed
        .iter()
        .map(|label| (label.y / text::SMALL_LINE).floor() as u32 + 1)
        .max()
        .unwrap_or(1)
}

/// Place a small name under each tick. Neighbours that would overlap drop to the
/// next row. When a name still overlaps, only the selected step is drawn.
pub fn place_labels(
    steps: &[SliderStep],
    current: Option<usize>,
    left: f32,
    right: f32,
    top: f32,
) -> Vec<TickLabel> {
    let count = steps.len();
    if count == 0 || right <= left {
        return Vec::new();
    }
    let span = right - left;
    let packed = steps
        .iter()
        .map(|step| text::measure_small(&step.name).0)
        .sum::<f32>()
        + 4.0 * count.saturating_sub(1) as f32;
    if packed > span {
        return vec![selected_only(steps, current, left, right, span, top)];
    }
    let mut rows: Vec<f32> = Vec::new();
    let mut out = Vec::with_capacity(count);
    for (index, step) in steps.iter().enumerate() {
        let t = if count == 1 {
            0.5
        } else {
            index as f32 / (count - 1) as f32
        };
        let tick = left + span * t;
        let width = text::measure_small(&step.name).0;
        let x = (tick - width * 0.5).clamp(left, (right - width).max(left));
        let mut row = None;
        for (r, edge) in rows.iter().enumerate() {
            if x >= *edge {
                row = Some(r);
                break;
            }
        }
        let row = match row {
            Some(row) => row,
            None if rows.len() < 3 => {
                rows.push(left);
                rows.len() - 1
            }
            None => {
                if current != Some(index) {
                    continue;
                }
                0
            }
        };
        rows[row] = x + width + 4.0;
        out.push(TickLabel {
            index,
            x,
            y: top + row as f32 * text::SMALL_LINE,
            text: step.name.clone(),
            current: current == Some(index),
        });
    }
    if !labels_overlap(&out) {
        return out;
    }
    vec![selected_only(steps, current, left, right, span, top)]
}

fn selected_only(
    steps: &[SliderStep],
    current: Option<usize>,
    left: f32,
    right: f32,
    span: f32,
    top: f32,
) -> TickLabel {
    let index = current.unwrap_or(0).min(steps.len().saturating_sub(1));
    let step = &steps[index];
    let width = text::measure_small(&step.name).0;
    let count = steps.len();
    let t = if count <= 1 {
        0.5
    } else {
        index as f32 / (count - 1) as f32
    };
    TickLabel {
        index,
        x: (left + span * t - width * 0.5).clamp(left, (right - width).max(left)),
        y: top,
        text: step.name.clone(),
        current: current == Some(index),
    }
}

fn labels_overlap(labels: &[TickLabel]) -> bool {
    for (i, a) in labels.iter().enumerate() {
        let a_right = a.x + text::measure_small(&a.text).0;
        for b in labels.iter().skip(i + 1) {
            if (a.y - b.y).abs() > 0.5 {
                continue;
            }
            let b_right = b.x + text::measure_small(&b.text).0;
            if a.x < b_right - 0.5 && b.x < a_right - 0.5 {
                return true;
            }
        }
    }
    false
}

pub fn label_blots(label: &TickLabel) -> Vec<Blot> {
    text::blots_small(&label.text, label.x, label.y)
}

/// Pointer fraction `t` in 0..1 to a snapped value.
pub fn value_at_fraction(min: f64, max: f64, step: f64, steps: &[SliderStep], t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if !steps.is_empty() {
        let last = steps.len() - 1;
        if last == 0 {
            return steps[0].value;
        }
        let index = ((t * last as f64).round() as usize).min(last);
        return steps[index].value;
    }
    let v = min + t * (max - min);
    snap_numeric(min, max, step, v)
}

/// Move `delta` steps from `value`. Named sliders walk the step list. A numeric
/// slider adds `delta * step`, or a tenth of the span when `step` is 0.
pub fn step_by(min: f64, max: f64, step: f64, steps: &[SliderStep], value: f64, delta: i32) -> f64 {
    if !steps.is_empty() {
        let last = steps.len() - 1;
        let index = nearest_index(steps, value).unwrap_or(0);
        let next = (index as i32 + delta).clamp(0, last as i32) as usize;
        return steps[next].value;
    }
    let base = snap_numeric(min, max, step, value);
    if delta == 0 {
        return base;
    }
    let stride = if step > 0.0 {
        step
    } else {
        let span = max - min;
        if span > 0.0 {
            span / 10.0
        } else {
            0.0
        }
    };
    (base + f64::from(delta) * stride).clamp(min, max)
}

pub fn nearest_index(steps: &[SliderStep], value: f64) -> Option<usize> {
    if steps.is_empty() {
        return None;
    }
    let mut best = 0;
    let mut best_dist = f64::MAX;
    for (index, step) in steps.iter().enumerate() {
        let dist = (step.value - value).abs();
        if dist < best_dist {
            best = index;
            best_dist = dist;
        }
    }
    Some(best)
}

/// Share of the track the thumb sits on, 0 at the left and 1 at the right.
pub fn share(min: f64, max: f64, steps: &[SliderStep], value: f64) -> f32 {
    if let Some(index) = nearest_index(steps, value) {
        let last = steps.len().saturating_sub(1);
        if last == 0 {
            return 0.0;
        }
        return index as f32 / last as f32;
    }
    let span = max - min;
    if span <= 0.0 {
        0.0
    } else {
        ((value - min) / span).clamp(0.0, 1.0) as f32
    }
}

pub fn snap_numeric(min: f64, max: f64, step: f64, value: f64) -> f64 {
    let v = value.clamp(min, max);
    if step > 0.0 {
        (min + ((v - min) / step).round() * step).clamp(min, max)
    } else {
        v
    }
}

/// Name and exact value while the pointer is on the slider. A numeric slider
/// with no steps returns the exact value.
pub fn tip(steps: &[SliderStep], value: f64) -> String {
    if let Some(index) = nearest_index(steps, value) {
        let step = &steps[index];
        let exact = format_exact(step.value);
        if step.name == exact {
            exact
        } else {
            format!("{}  {exact}", step.name)
        }
    } else {
        format_exact(value)
    }
}

pub fn format_exact(value: f64) -> String {
    if !value.is_finite() {
        return "0".into();
    }
    let nearest = value.round();
    if (value - nearest).abs() < 1.0e-6 && nearest.abs() < 1.0e12 {
        return format!("{}", nearest as i64);
    }
    let mut text = format!("{value:.4}");
    if let Some(dot) = text.find('.') {
        while text.len() > dot + 1 && text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    text
}

/// Notch centres along a track, inset from the box edges.
pub fn notch_x(track: Rect, index: usize, count: usize, inset: f32) -> f32 {
    let left = track.x + inset;
    let right = track.x + track.w - inset;
    if count <= 1 {
        return (left + right) * 0.5;
    }
    let t = index as f32 / (count - 1) as f32;
    left + (right - left) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grades() -> Vec<SliderStep> {
        ["Off", "Low", "Medium", "High", "Ultra"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| SliderStep {
                name: name.into(),
                value: i as f64,
            })
            .collect()
    }

    fn passes() -> Vec<SliderStep> {
        [(1.0, "1"), (3.0, "3"), (5.0, "5")]
            .into_iter()
            .map(|(value, name)| SliderStep {
                name: name.into(),
                value,
            })
            .collect()
    }

    #[test]
    fn named_fractions_map_onto_step_values() {
        let steps = grades();
        assert_eq!(value_at_fraction(0.0, 4.0, 0.0, &steps, 0.0), 0.0);
        assert_eq!(value_at_fraction(0.0, 4.0, 0.0, &steps, 0.5), 2.0);
        assert_eq!(value_at_fraction(0.0, 4.0, 0.0, &steps, 1.0), 4.0);
        assert_eq!(value_at_fraction(0.0, 4.0, 0.0, &steps, 0.12), 0.0);
        assert_eq!(value_at_fraction(0.0, 4.0, 0.0, &steps, 0.13), 1.0);
        assert_eq!(tip(&steps, 2.0), "Medium  2");
    }

    #[test]
    fn uneven_steps_snap_to_the_listed_values() {
        let steps = passes();
        for i in 0..=20 {
            let v = value_at_fraction(1.0, 5.0, 0.0, &steps, i as f64 / 20.0);
            assert!(
                v == 1.0 || v == 3.0 || v == 5.0,
                "fraction {} mapped to {v}, off the list",
                i as f64 / 20.0
            );
        }
        assert_eq!(value_at_fraction(1.0, 5.0, 0.0, &steps, 0.0), 1.0);
        assert_eq!(value_at_fraction(1.0, 5.0, 0.0, &steps, 0.5), 3.0);
        assert_eq!(value_at_fraction(1.0, 5.0, 0.0, &steps, 1.0), 5.0);
        assert_eq!(share(1.0, 5.0, &steps, 3.0), 0.5);
    }

    #[test]
    fn stepping_walks_names_and_clamps() {
        let steps = grades();
        assert_eq!(step_by(0.0, 4.0, 0.0, &steps, 2.0, 1), 3.0);
        assert_eq!(step_by(0.0, 4.0, 0.0, &steps, 2.0, -1), 1.0);
        assert_eq!(step_by(0.0, 4.0, 0.0, &steps, 4.0, 3), 4.0);
        assert_eq!(step_by(0.0, 4.0, 0.0, &steps, 0.0, -2), 0.0);
        let passes = passes();
        assert_eq!(step_by(1.0, 5.0, 0.0, &passes, 1.0, 1), 3.0);
        assert_eq!(step_by(1.0, 5.0, 0.0, &passes, 3.0, 1), 5.0);
    }

    #[test]
    fn a_continuous_fraction_stays_linear_and_a_step_snaps() {
        assert!((value_at_fraction(0.0, 24.0, 0.0, &[], 0.25) - 6.0).abs() < 1.0e-9);
        assert_eq!(value_at_fraction(0.0, 24.0, 6.0, &[], 0.55), 12.0);
        assert_eq!(step_by(0.0, 24.0, 6.0, &[], 12.0, 1), 18.0);
        assert_eq!(step_by(0.0, 24.0, 6.0, &[], 0.0, -1), 0.0);
        let tenth = step_by(0.0, 10.0, 0.0, &[], 0.0, 1);
        assert!((tenth - 1.0).abs() < 1.0e-9);
    }

    #[test]
    fn long_names_wrap_onto_a_second_row_and_the_current_step_stays() {
        let steps: Vec<SliderStep> = ["potato", "day-cycle", "gi-1bounce", "low", "medium", "high"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| SliderStep {
                name: name.into(),
                value: i as f64,
            })
            .collect();
        let placed = place_labels(&steps, Some(2), 0.0, 280.0, 0.0);
        assert!(
            placed
                .iter()
                .any(|label| label.text == "gi-1bounce" && label.current),
            "the current step was dropped"
        );
        assert!(
            placed.iter().any(|label| label.y > 0.0),
            "names stayed on one row"
        );
        for pair in placed.windows(2) {
            if (pair[0].y - pair[1].y).abs() < 0.5 {
                let a_right = pair[0].x + text::measure_small(&pair[0].text).0;
                assert!(
                    pair[1].x + 0.5 >= a_right || pair[0].current || pair[1].current,
                    "labels overlap: {} and {}",
                    pair[0].text,
                    pair[1].text
                );
            }
        }
    }

    #[test]
    fn a_narrow_track_keeps_only_the_selected_name() {
        let steps: Vec<SliderStep> = [
            "potato",
            "day-cycle",
            "gi-1bounce",
            "low",
            "medium",
            "high",
            "custom",
        ]
        .into_iter()
        .enumerate()
        .map(|(i, name)| SliderStep {
            name: name.into(),
            value: i as f64,
        })
        .collect();
        let placed = place_labels(&steps, Some(2), 12.0, 160.0, 0.0);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].text, "gi-1bounce");
        assert!(placed[0].current);
        assert_eq!(label_rows(&steps, 12.0, 160.0), 1);
    }
}
