//! OreUI icons as our own pixel art (one texel = 0.2rem), matched to the
//! originals' sizes; the local-originals mode draws the install's sprite instead.

use super::super::super::UiPresentationError;
use super::paint::{Bounds, Canvas};
use super::theme::{EDGE, Rgba};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Icon {
    ArrowBack,
    Cross,
    Search,
    Player,
}

impl Icon {
    /// The install's atlas key for the local-originals mode.
    fn original(self) -> &'static str {
        match self {
            Self::ArrowBack => "assets/arrow-left@0.5x.icon-32012c9c1d3e6debaaf8ce01da9815de.png",
            Self::Cross => "assets/cross@0.5x.icon-a30f9556f5895c0d6996af977c6fbb91.png",
            Self::Search => "assets/search@0.5x.icon-57e5a707535fd5959a31271ce38e0b7f.png",
            Self::Player => "assets/player@0.5x.icon-5f2efe885c1189f09a5388b6e6b07c9f.png",
        }
    }

    /// Our pixel art: one row per string, `#` for a lit texel.
    fn pixels(self) -> &'static [&'static str] {
        match self {
            Self::ArrowBack => &[
                "....##......",
                "...##.......",
                "..##........",
                ".###########",
                "############",
                ".###########",
                "..##........",
                "...##.......",
                "....##......",
            ],
            Self::Cross => &[
                "##...##", ".##.##.", "..###..", "...#...", "..###..", ".##.##.", "##...##",
            ],
            Self::Search => &[
                "..####......",
                ".#....#.....",
                "#......#....",
                "#......#....",
                "#......#....",
                "#......#....",
                ".#....#.....",
                "..####.#....",
                ".......##...",
                "........##..",
                ".........##.",
                "..........##",
            ],
            Self::Player => &[
                "..####..", "..####..", "..####..", "..####..", "........", ".######.", "########",
                "########", "########",
            ],
        }
    }

    /// Texel size of our art.
    pub(super) fn texels(self) -> [usize; 2] {
        let rows = self.pixels();
        [rows.first().map_or(0, |row| row.len()), rows.len()]
    }
}

/// Draws `icon` with its top-left at `at`, one texel per 0.2rem (tinted by `color`).
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    icon: Icon,
    at: [f32; 2],
    color: Rgba,
) -> Result<(), UiPresentationError> {
    let texel = canvas.r(EDGE);
    let [width, height] = icon.texels();
    let bounds: Bounds = [
        at[0],
        at[1],
        at[0] + width as f32 * texel,
        at[1] + height as f32 * texel,
    ];
    if canvas.sprite(icon.original(), bounds, [255; 4])? {
        return Ok(());
    }
    for (row, line) in icon.pixels().iter().enumerate() {
        // Runs of lit texels become one fill each.
        let bytes = line.as_bytes();
        let mut column = 0;
        while column < bytes.len() {
            if bytes[column] != b'#' {
                column += 1;
                continue;
            }
            let start = column;
            while column < bytes.len() && bytes[column] == b'#' {
                column += 1;
            }
            let top = at[1] + row as f32 * texel;
            canvas.fill(
                [
                    at[0] + start as f32 * texel,
                    top,
                    at[0] + column as f32 * texel,
                    top + texel,
                ],
                color,
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn art_rows_are_rectangular() {
        for icon in [Icon::ArrowBack, Icon::Cross, Icon::Search, Icon::Player] {
            let [width, _] = icon.texels();
            assert!(
                icon.pixels().iter().all(|row| row.len() == width),
                "{icon:?}"
            );
        }
    }
}
