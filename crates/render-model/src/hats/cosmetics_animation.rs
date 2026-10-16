//! Optional data-driven motion applied to authored attachment joints.
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Default, Deserialize)]
pub(super) struct Animation {
    pub wind: Option<Wind>,
}
#[derive(Deserialize)]
pub(super) struct Wind {
    pub axis: String,
    pub max_degrees: f32,
    pub speed: f32,
    pub rise: f32,
    pub fall: f32,
    pub joints: BTreeMap<String, f32>,
}
