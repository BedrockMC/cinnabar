use json_ui::{DataSource, Scalar};
use serde_json::json;
use ui::mod_panel::{Control, Panel};

use super::widgets::Palette;

pub(super) fn control_data(panel: &Panel) -> DataSource {
    let mut data = DataSource::new();
    let palette = Palette::new(panel.dark);
    for (index, control) in panel.controls.iter().enumerate() {
        let value = match control {
            Control::Toggle { value, .. } => if *value { "On" } else { "Off" }.to_owned(),
            Control::Slider { value, .. } => format!("{value:.2}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_owned(),
            Control::Button { .. } => String::new(),
            Control::Choice { index, options, .. } => options[*index as usize].clone(),
        };
        data.set_global(
            format!("#row_{index}_label"),
            Scalar::Text(control.label().to_owned()),
        );
        data.set_global(format!("#row_{index}_value"), Scalar::Text(value));
        match control {
            Control::Slider {
                value, min, max, ..
            } => data.set_global(
                format!("#row_{index}_fill"),
                Scalar::Num(f64::from((value - min) / (max - min))),
            ),
            Control::Toggle { value, .. } => {
                data.set_global(
                    format!("#row_{index}_toggle_color"),
                    Scalar::Json(json!(if *value {
                        palette.accent
                    } else {
                        palette.raised
                    })),
                );
                data.set_global(
                    format!("#row_{index}_knob_offset"),
                    Scalar::Json(json!([if *value { 12.0 } else { 2.0 }, 2.0])),
                );
            }
            _ => {}
        }
    }
    data
}
