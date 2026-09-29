//! Alpha animations. An `@ns.anim` reference in `alpha` or `anims` resolves once,
//! in the referencing control's variable scope (so a factory's `$title_fade_in_time`
//! applies), into a [`Chain`]; a draw evaluates it against the time since its
//! control was created, which the caller supplies at paint time so layout never
//! depends on the clock.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::Catalog;
use crate::env::{Env, substitute};
use crate::tree::{ControlRef, ResolvedControl};

/// Property holding the resolved chains of a control (a JSON array of chains).
pub(crate) const CHAINS_KEY: &str = "anim_alpha";
/// Property holding a factory instance's creation time in seconds.
pub(crate) const BORN_KEY: &str = "anim_born";
/// Property naming the caller clock that holds an instance's creation time, so
/// a re-sent title restarts its fade without re-binding the screen.
pub(crate) const CLOCK_KEY: &str = "anim_clock";
/// Longest `next` chain followed; a longer or cyclic chain loops from its start.
const MAX_STEPS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepKind {
    Alpha,
    Wait,
    /// Any other anim type: holds the current value for its duration.
    Other,
}

/// One step of a chain: `from`/`to` only matter for alpha steps.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub kind: StepKind,
    pub duration: f64,
    pub from: f64,
    pub to: f64,
    pub easing: String,
    /// `destroy_at_end` names a control: after this step the draw vanishes.
    pub destroys: bool,
}

/// A resolved `next` chain; `looping` when it refers back to itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chain {
    pub steps: Vec<Step>,
    pub looping: bool,
}

/// A chain applied to a draw: its value divided by `rest` (the control's static
/// alpha) scales the draw, so wait steps hold the static alpha.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fade {
    pub chain: Chain,
    pub rest: f32,
    /// Seconds on the caller's clock when the animated control was created.
    pub born: f64,
    /// The caller clock that overrides `born`, when the control has one.
    #[serde(default)]
    pub clock: Option<String>,
}

impl Fade {
    /// The multiplier this fade applies at `now` (seconds, the `born` clock).
    pub fn factor(&self, now: f64) -> f32 {
        let value = self.chain.value_at(now - self.born, f64::from(self.rest));
        if self.rest > 0.0 {
            (value / f64::from(self.rest)) as f32
        } else {
            value as f32
        }
    }
}

/// The product of every fade's multiplier at `now`.
pub fn fade_factor(fades: &[Fade], now: f64) -> f32 {
    fades.iter().map(|fade| fade.factor(now)).product()
}

/// [`fade_factor`] with each fade's creation time read from `clocks` when it names one.
pub fn fade_factor_at(
    fades: &[Fade],
    now: f64,
    clocks: &std::collections::BTreeMap<String, f64>,
) -> f32 {
    fades
        .iter()
        .map(|fade| {
            let born = fade
                .clock
                .as_ref()
                .and_then(|clock| clocks.get(clock))
                .copied()
                .unwrap_or(fade.born);
            fade.factor(now - born + fade.born)
        })
        .product()
}

impl Chain {
    /// The alpha `age` seconds after creation, holding `rest` until the first
    /// alpha step and after a wait.
    pub fn value_at(&self, age: f64, rest: f64) -> f64 {
        let total: f64 = self.steps.iter().map(|step| step.duration.max(0.0)).sum();
        let mut age = age.max(0.0);
        if self.looping && total > 0.0 {
            age %= total;
        }
        let mut current = rest;
        for step in &self.steps {
            let duration = step.duration.max(0.0);
            if age < duration {
                return match step.kind {
                    StepKind::Alpha => {
                        let t = ease(&step.easing, age / duration);
                        step.from + (step.to - step.from) * t
                    }
                    _ => current,
                };
            }
            age -= duration;
            if step.kind == StepKind::Alpha {
                current = step.to;
            }
            if step.destroys {
                return 0.0;
            }
        }
        current
    }
}

/// Resolve `@ns.name` (following `next`) against `catalog` in `env`; `None` for
/// an unknown reference, an event-started chain, or one with no alpha step.
pub(crate) fn resolve_chain(catalog: &Catalog, reference: &str, env: &Env) -> Option<Chain> {
    let mut steps = Vec::new();
    let mut seen: Vec<ControlRef> = Vec::new();
    let mut owner = String::new();
    let mut next = Some(reference.to_owned());
    let mut looping = false;
    while let Some(text) = next.take() {
        let target = ControlRef::parse(&text, &owner);
        if seen.contains(&target) {
            looping = true;
            break;
        }
        if seen.len() >= MAX_STEPS {
            break;
        }
        let def = catalog.lookup(&target.namespace, &target.name)?;
        let props = Value::Object(def.props.clone());
        let props = substitute(&props, env, &mut Vec::new());
        // An event-started animation (screen transitions) never plays by itself.
        if steps.is_empty() && props.get("play_event").is_some_and(|event| event != "") {
            return None;
        }
        let number = |key: &str, fallback: f64| match props.get(key) {
            Some(Value::Number(value)) => value.as_f64().unwrap_or(fallback),
            Some(Value::String(text)) => text.parse().unwrap_or(fallback),
            _ => fallback,
        };
        let kind = match props.get("anim_type").and_then(Value::as_str) {
            Some("alpha") => StepKind::Alpha,
            Some("wait") => StepKind::Wait,
            _ => StepKind::Other,
        };
        steps.push(Step {
            kind,
            duration: number("duration", 0.0),
            from: number("from", 1.0),
            to: number("to", 1.0),
            easing: props
                .get("easing")
                .and_then(Value::as_str)
                .unwrap_or("linear")
                .to_owned(),
            destroys: props
                .get("destroy_at_end")
                .and_then(Value::as_str)
                .is_some_and(|name| !name.is_empty()),
        });
        owner = target.namespace.clone();
        seen.push(target);
        next = props
            .get("next")
            .and_then(Value::as_str)
            .filter(|text| text.starts_with('@'))
            .map(str::to_owned);
    }
    steps
        .iter()
        .any(|step| step.kind == StepKind::Alpha)
        .then_some(Chain { steps, looping })
}

/// What a control takes from its ancestors: the creation time of the nearest
/// factory instance, and a `propagate_alpha` parent's alpha and fades.
#[derive(Clone, Default)]
pub(crate) struct Inherited {
    alpha: Option<f32>,
    fades: Vec<Fade>,
    born: f64,
    clock: Option<String>,
}

impl Inherited {
    /// This control's alpha and fades, and what its children inherit.
    pub(crate) fn apply(
        &self,
        control: &ResolvedControl,
        rest: f32,
    ) -> (f32, Vec<Fade>, Inherited) {
        let born = crate::widgets::bound_number(control, BORN_KEY).unwrap_or(self.born);
        let clock = control
            .properties
            .get(CLOCK_KEY)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| self.clock.clone());
        let mut fades = self.fades.clone();
        if let Some(Value::Array(chains)) = control.properties.get(CHAINS_KEY) {
            fades.extend(chains.iter().filter_map(|chain| {
                serde_json::from_value::<Chain>(chain.clone())
                    .ok()
                    .map(|chain| Fade {
                        chain,
                        rest,
                        born,
                        clock: clock.clone(),
                    })
            }));
        }
        let own = rest * self.alpha.unwrap_or(1.0);
        let propagate = matches!(
            control.properties.get("propagate_alpha"),
            Some(Value::Bool(true))
        );
        let children = Inherited {
            alpha: if propagate { Some(own) } else { self.alpha },
            fades: if propagate {
                fades.clone()
            } else {
                self.fades.clone()
            },
            born,
            clock,
        };
        (own, fades, children)
    }
}

/// The standard easing curves by their JSON-UI names; unknown names are linear.
fn ease(name: &str, t: f64) -> f64 {
    use std::f64::consts::PI;
    let t = t.clamp(0.0, 1.0);
    let out = |f: &dyn Fn(f64) -> f64| 1.0 - f(1.0 - t);
    let in_out = |f: &dyn Fn(f64) -> f64| {
        if t < 0.5 {
            f(t * 2.0) / 2.0
        } else {
            1.0 - f((1.0 - t) * 2.0) / 2.0
        }
    };
    let power = |p: i32| move |x: f64| x.powi(p);
    let sine = |x: f64| 1.0 - (x * PI / 2.0).cos();
    let expo = |x: f64| {
        if x <= 0.0 {
            0.0
        } else {
            2f64.powf(10.0 * x - 10.0)
        }
    };
    let circ = |x: f64| 1.0 - (1.0 - x * x).max(0.0).sqrt();
    let back = |x: f64| 2.70158 * x * x * x - 1.70158 * x * x;
    match name {
        "step" => {
            if t < 1.0 {
                0.0
            } else {
                1.0
            }
        }
        "in_quad" => power(2)(t),
        "out_quad" => out(&power(2)),
        "in_out_quad" => in_out(&power(2)),
        "in_cubic" => power(3)(t),
        "out_cubic" => out(&power(3)),
        "in_out_cubic" => in_out(&power(3)),
        "in_quart" => power(4)(t),
        "out_quart" => out(&power(4)),
        "in_out_quart" => in_out(&power(4)),
        "in_quint" => power(5)(t),
        "out_quint" => out(&power(5)),
        "in_out_quint" => in_out(&power(5)),
        "in_sine" => sine(t),
        "out_sine" => out(&sine),
        "in_out_sine" => in_out(&sine),
        "in_expo" => expo(t),
        "out_expo" => out(&expo),
        "in_out_expo" => in_out(&expo),
        "in_circ" => circ(t),
        "out_circ" => out(&circ),
        "in_out_circ" => in_out(&circ),
        "in_back" => back(t),
        "out_back" => out(&back),
        "in_out_back" => in_out(&back),
        _ => t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(kind: StepKind, duration: f64, from: f64, to: f64) -> Step {
        Step {
            kind,
            duration,
            from,
            to,
            easing: "linear".to_owned(),
            destroys: false,
        }
    }

    // A title fades in, holds, fades out, then stays gone.
    #[test]
    fn chain_holds_between_steps_and_ends_at_the_last_value() {
        let chain = Chain {
            steps: vec![
                step(StepKind::Alpha, 1.0, 0.0, 1.0),
                step(StepKind::Wait, 2.0, 0.0, 0.0),
                step(StepKind::Alpha, 1.0, 1.0, 0.0),
            ],
            looping: false,
        };
        assert_eq!(chain.value_at(0.5, 1.0), 0.5);
        assert_eq!(chain.value_at(2.0, 1.0), 1.0);
        assert_eq!(chain.value_at(3.5, 1.0), 0.5);
        assert_eq!(chain.value_at(9.0, 1.0), 0.0);
    }

    // A wait before the first alpha step keeps the control's static alpha.
    #[test]
    fn a_leading_wait_holds_the_static_alpha() {
        let chain = Chain {
            steps: vec![
                step(StepKind::Wait, 10.0, 0.0, 0.0),
                step(StepKind::Alpha, 1.0, 0.5, 0.0),
            ],
            looping: false,
        };
        let fade = Fade {
            chain,
            rest: 0.5,
            born: 100.0,
            clock: None,
        };
        assert_eq!(fade.factor(105.0), 1.0);
        assert_eq!(fade.factor(110.5), 0.5);
        assert_eq!(fade.factor(200.0), 0.0);
    }
}
