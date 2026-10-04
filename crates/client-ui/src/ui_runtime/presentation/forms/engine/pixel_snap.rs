
/// Float noise from layout arithmetic that must not move a position that is whole in physical
/// pixels (`2.9999998` physical pixels is 3).
const EPSILON: f32 = 1.0 / 1024.0;

/// `rect` (`x`, `y`, `w`, `h` in GUI units) as logical `[x0, y0, x1, y1]` whose physical edges
/// are whole pixels, as `drawImage` places an image: `pixels` physical pixels and `logical`
/// logical pixels per GUI unit (their ratio is the platform DPI).
pub(super) fn snapped(rect: [f64; 4], pixels: f32, logical: f32) -> [f32; 4] {
    let [x, y, w, h] = rect.map(|value| value as f32 * pixels);
    let size = |value: f32| (value - EPSILON).ceil().max(0.0);
    let (x0, y0) = (position(x), position(y));
    let to_logical = logical / pixels;
    [x0, y0, x0 + size(w), y0 + size(h)].map(|edge| edge * to_logical)
}

/// `rect` moved to a whole physical pixel with its size kept, as `flushText` places text.
pub(super) fn positioned(rect: [f64; 4], pixels: f32, logical: f32) -> [f32; 4] {
    let [x, y, w, h] = rect.map(|value| value as f32 * pixels);
    let to_logical = logical / pixels;
    let (x0, y0) = (position(x) * to_logical, position(y) * to_logical);
    [x0, y0, x0 + w * to_logical, y0 + h * to_logical]
}

/// A physical position truncated toward zero, as the `(int)` cast does.
fn position(value: f32) -> f32 {
    (value + EPSILON.copysign(value)).trunc()
}

#[cfg(test)]
mod tests;
