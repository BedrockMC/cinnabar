//! Live state for a form drawn by the JSON-UI engine: the caller-held view state
//! (hover, press, focus, scroll) plus each custom-form element's current value,
//! seeded from the decoded defaults and edited by input until submission.

use std::sync::Arc;

use json_ui::{HitRegion, LayoutReport, ViewState};
use protocol::{CustomFormElement, CustomFormValue, ServerFormModel};
use ui::UiPoint;

use super::ServerFormIdentity;

/// What the last engine-drawn frame exposes to input: regions and scroll extents
/// in virtual pixels, plus the virtual → window-logical mapping.
#[derive(Clone, Debug)]
pub(crate) struct EngineFrame {
    pub(crate) identity: ServerFormIdentity,
    pub(crate) hits: Vec<HitRegion>,
    pub(crate) report: LayoutReport,
    /// Where `button.menu_cancel` (Escape) routes on this screen.
    pub(crate) cancel_target: Option<String>,
    /// Window-logical position of virtual `(0, 0)`.
    pub(crate) origin: [f32; 2],
    /// Window-logical pixels per virtual pixel.
    pub(crate) scale: f32,
}

impl EngineFrame {
    pub(crate) fn to_virtual(&self, point: UiPoint) -> [f64; 2] {
        [
            f64::from((point.x() - self.origin[0]) / self.scale),
            f64::from((point.y() - self.origin[1]) / self.scale),
        ]
    }
}

/// One custom-form element's value; decorations hold `None` so indexes align.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FormValue {
    None,
    Toggle(bool),
    /// The slider's actual value (already snapped to its step).
    Slider(f64),
    Step(usize),
    Dropdown(usize),
    Text(String),
}

/// A pointer drag the engine path is tracking.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FormDrag {
    /// A slider at this element index.
    Slider(usize),
    /// A scrollbar box: the scroll view key and the pointer's offset into the box.
    ScrollBox { view: String, grab: f64 },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct FormEngineState {
    pub(crate) view: ViewState,
    pub(crate) values: Vec<FormValue>,
    pub(crate) open_dropdown: Option<usize>,
    /// The input element receiving typed text.
    pub(crate) editing: Option<usize>,
    pub(crate) drag: Option<FormDrag>,
}

impl FormEngineState {
    pub(crate) fn for_model(model: &ServerFormModel) -> Self {
        let values = match model {
            ServerFormModel::Custom(form) => form.elements.iter().map(initial_value).collect(),
            _ => Vec::new(),
        };
        Self {
            values,
            ..Self::default()
        }
    }

    /// The response array, one entry per element in wire order.
    pub(crate) fn submission(&self) -> Arc<[CustomFormValue]> {
        self.values
            .iter()
            .map(|value| match value {
                FormValue::None => CustomFormValue::Null,
                FormValue::Toggle(on) => CustomFormValue::Toggle(*on),
                FormValue::Slider(value) => CustomFormValue::Slider(*value),
                FormValue::Step(index) => CustomFormValue::Step(*index as u32),
                FormValue::Dropdown(index) => CustomFormValue::Dropdown(*index as u32),
                FormValue::Text(text) => CustomFormValue::Input(text.clone()),
            })
            .collect()
    }
}

fn initial_value(element: &CustomFormElement) -> FormValue {
    match element {
        CustomFormElement::Label { .. }
        | CustomFormElement::Header { .. }
        | CustomFormElement::Divider => FormValue::None,
        CustomFormElement::Toggle { default, .. } => FormValue::Toggle(*default),
        CustomFormElement::Slider { default, .. } => FormValue::Slider(default.get()),
        CustomFormElement::StepSlider { default, .. } => FormValue::Step(*default as usize),
        CustomFormElement::Dropdown { default, .. } => FormValue::Dropdown(*default as usize),
        CustomFormElement::Input { default, .. } => FormValue::Text(default.to_string()),
    }
}

/// Snap a `0..=1` drag position to the slider's step grid within `[min, max]`.
pub(crate) fn slider_value_at(min: f64, max: f64, step: f64, fraction: f64) -> f64 {
    let span = (max - min).max(0.0);
    let raw = min + span * fraction.clamp(0.0, 1.0);
    if step <= 0.0 || span == 0.0 {
        return raw.clamp(min, max);
    }
    (min + ((raw - min) / step).round() * step).clamp(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_drags_snap_to_the_step_grid() {
        assert_eq!(slider_value_at(0.0, 10.0, 2.0, 0.34), 4.0);
        assert_eq!(slider_value_at(0.0, 10.0, 2.0, 1.0), 10.0);
        assert_eq!(slider_value_at(5.0, 5.0, 1.0, 0.7), 5.0);
    }

    #[test]
    fn submission_keeps_decorations_as_nulls_in_order() {
        let state = FormEngineState {
            values: vec![
                FormValue::None,
                FormValue::Toggle(true),
                FormValue::Text("hi".into()),
            ],
            ..FormEngineState::default()
        };
        assert_eq!(
            state.submission().as_ref(),
            [
                CustomFormValue::Null,
                CustomFormValue::Toggle(true),
                CustomFormValue::Input("hi".into())
            ]
        );
    }
}
