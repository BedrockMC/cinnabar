//! Alpha and offset animations. An `@ns.anim` reference in `alpha` or `anims` resolves once,
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
/// Property holding a control's resolved `offset` animation (a [`Slide`]).
pub(crate) const SLIDE_KEY: &str = "anim_offset";
/// Resolved `size` animation: a [`Slide`] of size vectors, relaid out each tick.
pub(crate) const RESIZE_KEY: &str = "anim_size";
/// Property holding a control's resolved `uv` flip-book (a [`FlipBook`]).
pub(crate) const FLIP_BOOK_KEY: &str = "anim_flip_book";
/// Longest `next` chain followed; a longer or cyclic chain loops from its start.
const MAX_STEPS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepKind {
    Alpha,
    Wait,
    /// Any other anim type: holds the current value for its duration.
    Other,
}

/// One step of a chain: `from`/`to` only matter for interpolating (alpha) steps.
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
    /// The `play_event` that starts it (a screen transition); it plays only
    /// from the caller clock of that name.
    #[serde(default)]
    pub event: Option<String>,
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

/// The product of every fade's multiplier at `now`; an event-started fade
/// holds until its event fires.
pub fn fade_factor(fades: &[Fade], now: f64) -> f32 {
    fades
        .iter()
        .filter(|fade| fade.chain.event.is_none())
        .map(|fade| fade.factor(now))
        .product()
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
            let clock = fade.chain.event.as_ref().or(fade.clock.as_ref());
            let born = match clock.and_then(|clock| clocks.get(clock)) {
                Some(born) => *born,
                None if fade.chain.event.is_some() => return 1.0,
                None => fade.born,
            };
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
/// A `uv` flip-book: frames stepping right along the texture, played by the
/// caller's clock at paint time so layout never depends on it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlipBook {
    pub initial_uv: [f64; 2],
    pub frame_count: u32,
    /// Texture pixels between frames; the draw converts it to normalised uv.
    pub frame_step: f64,
    pub fps: f64,
    pub reversible: bool,
    pub looping: bool,
    /// Normalised uv between frames, set once the texture size is known.
    #[serde(default)]
    pub step_u: f32,
}

impl FlipBook {
    /// The frame shown `seconds` into the animation.
    pub fn frame(&self, seconds: f64) -> u32 {
        let count = u64::from(self.frame_count);
        if count <= 1 || self.fps <= 0.0 || !seconds.is_finite() || seconds < 0.0 {
            return 0;
        }
        let tick = (seconds * self.fps) as u64;
        let frame = if self.reversible {
            let period = 2 * (count - 1);
            let at = if self.looping {
                tick % period
            } else {
                tick.min(period)
            };
            if at < count { at } else { period - at }
        } else if self.looping {
            tick % count
        } else {
            tick.min(count - 1)
        };
        frame as u32
    }
}

/// The flip-book `reference` names; an event-started one holds its first frame.
pub(crate) fn resolve_flip_book(catalog: &Catalog, reference: &str, env: &Env) -> Option<FlipBook> {
    let target = ControlRef::parse(reference, "");
    let def = catalog.lookup(&target.namespace, &target.name)?;
    let props = substitute(&Value::Object(def.props.clone()), env, &mut Vec::new());
    if props.get("anim_type").and_then(Value::as_str) != Some("flip_book") {
        return None;
    }
    let number =
        |key: &str, fallback: f64| props.get(key).and_then(Value::as_f64).unwrap_or(fallback);
    let initial = props.get("initial_uv").and_then(Value::as_array);
    let coordinate = |index: usize| {
        initial
            .and_then(|pair| pair.get(index))
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
    };
    let event_started = props.get("play_event").is_some_and(|event| event != "");
    let flag =
        |key: &str, fallback: bool| props.get(key).and_then(Value::as_bool).unwrap_or(fallback);
    Some(FlipBook {
        initial_uv: [coordinate(0), coordinate(1)],
        frame_count: if event_started {
            1
        } else {
            number("frame_count", 1.0).clamp(1.0, 4096.0) as u32
        },
        frame_step: number("frame_step", 0.0),
        fps: number("fps", 0.0),
        reversible: flag("reversible", false),
        looping: flag("looping", true),
        step_u: 0.0,
    })
}

pub(crate) fn resolve_chain(catalog: &Catalog, reference: &str, env: &Env) -> Option<Chain> {
    let (links, looping, event) = chain_links(catalog, reference, env)?;
    let steps: Vec<Step> = links
        .iter()
        .map(|props| {
            let number = |key: &str| number_or(props, key, 1.0);
            let kind = match props.get("anim_type").and_then(Value::as_str) {
                Some("alpha") => StepKind::Alpha,
                Some("wait") => StepKind::Wait,
                _ => StepKind::Other,
            };
            Step {
                kind,
                duration: number_or(props, "duration", 0.0),
                from: number("from"),
                to: number("to"),
                easing: easing(props),
                destroys: props
                    .get("destroy_at_end")
                    .and_then(Value::as_str)
                    .is_some_and(|name| !name.is_empty()),
            }
        })
        .collect();
    steps
        .iter()
        .any(|step| step.kind == StepKind::Alpha)
        .then_some(Chain {
            steps,
            looping,
            event,
        })
}

/// Resolve `@ns.name` (following `next`) to its offset steps, their ends still
/// length expressions; `None` without an offset step.
pub(crate) fn resolve_slide(catalog: &Catalog, reference: &str, env: &Env) -> Option<Slide> {
    resolve_vector(catalog, reference, env, "offset")
}

/// Resolve `@ns.name` to its `size` steps; `None` without one or when an event
/// starts it.
pub(crate) fn resolve_resize(catalog: &Catalog, reference: &str, env: &Env) -> Option<Slide> {
    resolve_vector(catalog, reference, env, "size")
}

/// A `size` animation's resting vector: its first step's `from`, what the
/// control holds until the animation plays.
pub(crate) fn resting_size(catalog: &Catalog, reference: &str, env: &Env) -> Option<Value> {
    let target = ControlRef::parse(reference, "");
    let def = catalog.lookup(&target.namespace, &target.name)?;
    let Value::Object(props) = substitute(&Value::Object(def.props.clone()), env, &mut Vec::new())
    else {
        return None;
    };
    (props.get("anim_type").and_then(Value::as_str) == Some("size"))
        .then(|| props.get("from").cloned())
        .flatten()
}

fn resolve_vector(catalog: &Catalog, reference: &str, env: &Env, kind: &str) -> Option<Slide> {
    let (links, looping, event) = chain_links(catalog, reference, env)?;
    // Event-started slides (screen transitions) are not played.
    if event.is_some() {
        return None;
    }
    let steps: Vec<SlideStep> = links
        .iter()
        .map(|props| {
            let moves = props.get("anim_type").and_then(Value::as_str) == Some(kind);
            let end = |key: &str| {
                props
                    .get(key)
                    .filter(|value| moves && value.is_array())
                    .cloned()
                    .unwrap_or(Value::Null)
            };
            SlideStep {
                moves,
                duration: number_or(props, "duration", 0.0),
                from: end("from"),
                to: end("to"),
                easing: easing(props),
            }
        })
        .collect();
    steps
        .iter()
        .any(|step| step.moves)
        .then_some(Slide { steps, looping })
}

/// A chain's substituted link definitions, whether it loops, and its `play_event`.
type ChainLinks = (Vec<serde_json::Map<String, Value>>, bool, Option<String>);

/// The substituted definitions of `reference` and its `next` links, whether
/// they loop, and the `play_event` that starts them; `None` for an unknown reference.
fn chain_links(catalog: &Catalog, reference: &str, env: &Env) -> Option<ChainLinks> {
    let mut links = Vec::new();
    let mut seen: Vec<ControlRef> = Vec::new();
    let mut owner = String::new();
    let mut next = Some(reference.to_owned());
    let mut looping = false;
    let mut event = None;
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
        let Value::Object(props) = substitute(&props, env, &mut Vec::new()) else {
            return None;
        };
        if links.is_empty() {
            event = props
                .get("play_event")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .map(str::to_owned);
        }
        next = props
            .get("next")
            .and_then(Value::as_str)
            .filter(|text| text.starts_with('@'))
            .map(str::to_owned);
        owner = target.namespace.clone();
        seen.push(target);
        links.push(props);
    }
    Some((links, looping, event))
}

fn number_or(props: &serde_json::Map<String, Value>, key: &str, fallback: f64) -> f64 {
    match props.get(key) {
        Some(Value::Number(value)) => value.as_f64().unwrap_or(fallback),
        Some(Value::String(text)) => text.parse().unwrap_or(fallback),
        _ => fallback,
    }
}

fn easing(props: &serde_json::Map<String, Value>) -> String {
    props
        .get("easing")
        .and_then(Value::as_str)
        .unwrap_or("linear")
        .to_owned()
}

/// One step of an unresolved offset chain: `moves` for an `offset` step, whose
/// `from`/`to` are `[x, y]` offset expressions; any other step holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlideStep {
    pub moves: bool,
    pub duration: f64,
    pub from: Value,
    pub to: Value,
    pub easing: String,
}

/// An `offset` animation as resolved, before layout gives its ends pixels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Slide {
    pub steps: Vec<SlideStep>,
    pub looping: bool,
}

impl Slide {
    /// One axis's value `age` seconds in, each end measured by `pixels`
    /// (a vector element to pixels); `rest` holds before the first step.
    pub(crate) fn axis_at(&self, age: f64, rest: f64, pixels: impl Fn(&Value) -> f64) -> f64 {
        Chain {
            steps: self
                .steps
                .iter()
                .map(|step| Step {
                    kind: if step.moves {
                        StepKind::Alpha
                    } else {
                        StepKind::Wait
                    },
                    duration: step.duration,
                    from: pixels(&step.from),
                    to: pixels(&step.to),
                    easing: step.easing.clone(),
                    destroys: false,
                })
                .collect(),
            looping: self.looping,
            event: None,
        }
        .value_at(age, rest)
    }

    /// Whether the slide is still moving `age` seconds in.
    pub(crate) fn running(&self, age: f64) -> bool {
        self.looping
            || age
                < self
                    .steps
                    .iter()
                    .map(|step| step.duration.max(0.0))
                    .sum::<f64>()
    }

    /// The slide in pixels, its ends measured by `pixels` (an offset pair to
    /// pixels); `rest` is the static offset it replaces.
    pub(crate) fn motion(
        &self,
        rest: [f64; 2],
        born: f64,
        clock: Option<String>,
        pixels: impl Fn(&Value) -> [f64; 2],
    ) -> Motion {
        let axis = |index: usize| Chain {
            steps: self
                .steps
                .iter()
                .map(|step| Step {
                    kind: if step.moves {
                        StepKind::Alpha
                    } else {
                        StepKind::Wait
                    },
                    duration: step.duration,
                    from: pixels(&step.from)[index],
                    to: pixels(&step.to)[index],
                    easing: step.easing.clone(),
                    destroys: false,
                })
                .collect(),
            looping: self.looping,
            event: None,
        };
        Motion {
            axes: [axis(0), axis(1)],
            rest,
            born,
            clock,
        }
    }
}

/// An `offset` animation in pixels: the control and everything under it draw
/// displaced by its value less the static offset it replaces, at paint time.
/// Each axis is a [`Chain`] whose interpolating steps are the offset steps.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    pub axes: [Chain; 2],
    pub rest: [f64; 2],
    pub born: f64,
    #[serde(default)]
    pub clock: Option<String>,
}

/// The summed displacement of `motions` at `now`, each motion's creation time
/// read from `clocks` when it names one.
pub fn motion_offset(
    motions: &[Motion],
    now: f64,
    clocks: Option<&std::collections::BTreeMap<String, f64>>,
) -> [f64; 2] {
    motions.iter().fold([0.0; 2], |sum, motion| {
        let born = motion
            .clock
            .as_ref()
            .and_then(|clock| clocks?.get(clock))
            .copied()
            .unwrap_or(motion.born);
        let age = now - born;
        std::array::from_fn(|index| {
            let rest = motion.rest[index];
            sum[index] + motion.axes[index].value_at(age, rest) - rest
        })
    })
}

/// What a control takes from its ancestors: the creation time of the nearest
/// factory instance, and a `propagate_alpha` parent's alpha and fades.
#[derive(Clone, Default)]
pub(crate) struct Inherited {
    alpha: Option<f32>,
    fades: Vec<Fade>,
    born: f64,
    clock: Option<String>,
    /// Offset animations moving this control, and those moving its clip.
    pub(crate) motions: Motions,
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
            motions: self.motions.clone(),
        };
        (own, fades, children)
    }

    /// The creation time and clock of the control this was applied for.
    pub(crate) fn timing(&self) -> (f64, Option<String>) {
        (self.born, self.clock.clone())
    }
}

/// The offset animations displacing a draw at paint time: `own` moves its
/// rect, `clip` the rect of the ancestor that clips it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Motions {
    pub own: Vec<Motion>,
    pub clip: Vec<Motion>,
}

impl Motions {
    /// `(dest, clip)` displacements at `now`.
    pub fn at(
        &self,
        now: f64,
        clocks: Option<&std::collections::BTreeMap<String, f64>>,
    ) -> ([f64; 2], [f64; 2]) {
        (
            motion_offset(&self.own, now, clocks),
            motion_offset(&self.clip, now, clocks),
        )
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
mod flip_book_tests {
    use super::FlipBook;

    fn book(reversible: bool, looping: bool) -> FlipBook {
        FlipBook {
            initial_uv: [0.0, 0.0],
            frame_count: 4,
            frame_step: 8.0,
            fps: 10.0,
            reversible,
            looping,
            step_u: 0.25,
        }
    }

    // Frames advance with the clock, loop, ping-pong when reversible, or hold.
    #[test]
    fn frames_follow_the_clock() {
        let frames = |book: FlipBook| {
            (0..8)
                .map(|tick| book.frame(f64::from(tick) * 0.1 + 0.01))
                .collect::<Vec<_>>()
        };
        assert_eq!(frames(book(false, true)), [0, 1, 2, 3, 0, 1, 2, 3]);
        assert_eq!(frames(book(true, true)), [0, 1, 2, 3, 2, 1, 0, 1]);
        assert_eq!(frames(book(false, false)), [0, 1, 2, 3, 3, 3, 3, 3]);
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
            event: None,
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
            event: None,
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

    // A screen-transition fade holds until the caller's clock names its event.
    #[test]
    fn event_fades_wait_for_their_clock() {
        let fade = Fade {
            chain: Chain {
                steps: vec![step(StepKind::Alpha, 1.0, 0.0, 1.0)],
                looping: false,
                event: Some("screen.entrance_push".to_owned()),
            },
            rest: 1.0,
            born: 0.0,
            clock: None,
        };
        let fades = [fade];
        assert_eq!(fade_factor(&fades, 5.0), 1.0);
        let mut clocks = std::collections::BTreeMap::new();
        assert_eq!(fade_factor_at(&fades, 5.0, &clocks), 1.0);
        clocks.insert("screen.entrance_push".to_owned(), 4.5);
        assert_eq!(fade_factor_at(&fades, 5.0, &clocks), 0.5);
    }
}
