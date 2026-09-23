//! Renders generated intro cells without decoding or loading runtime assets.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;

use super::intro::IntroArtwork;
use super::model::ColorMode;
use super::render::{canvas_color, fade_from_black};

mod fox {
    include!("intro-frames.rs");
}

mod cat_pufferfish {
    include!("intro-pufferfish-frames.rs");
}

pub(crate) const FOX_SEQUENCE_LEN: usize = fox::FRAME_SEQUENCE.len();
pub(crate) const PUFFERFISH_SEQUENCE_LEN: usize = cat_pufferfish::FRAME_SEQUENCE.len();

const FOX_TRUECOLOR: [Color; 4] = [
    Color::Rgb(111, 41, 0),
    Color::Rgb(255, 97, 6),
    Color::Rgb(254, 202, 185),
    Color::Rgb(255, 244, 239),
];
const FOX_ANSI: [Color; 4] = [
    Color::Indexed(94),
    Color::Indexed(202),
    Color::Indexed(217),
    Color::Indexed(230),
];
const UNICODE_DENSITY: [&str; 3] = ["░", "▒", "█"];
const ASCII_DENSITY: [&str; 3] = [".", "+", "#"];
const BACKDROP_FADE_FRAME_COUNT: usize = 18;

pub(crate) fn render(
    frame: &mut ratatui::Frame<'_>,
    artwork: IntroArtwork,
    frame_index: usize,
    colors: ColorMode,
) {
    let area = frame.area();
    frame.render_widget(
        IntroArt {
            artwork,
            frame_index,
            colors,
        },
        area,
    );
}

struct IntroArt {
    artwork: IntroArtwork,
    frame_index: usize,
    colors: ColorMode,
}

impl Widget for IntroArt {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let background = fade_from_black(
            canvas_color(self.colors),
            self.colors,
            self.frame_index.min(BACKDROP_FADE_FRAME_COUNT),
            BACKDROP_FADE_FRAME_COUNT,
        );
        if background != Color::Reset {
            buffer.set_style(area, Style::default().bg(background));
        }

        match self.artwork {
            IntroArtwork::Fox => {
                let Some(rows) = fox::FRAME_SEQUENCE
                    .get(self.frame_index)
                    .and_then(|unique| fox::ART_FRAMES.get(*unique))
                else {
                    return;
                };
                draw_rows(area, buffer, rows, fox::ART_WIDTH, |row, column| {
                    decode_fox_cell(*row.as_bytes().get(column)?, self.colors)
                });
            }
            IntroArtwork::CatPufferfish => {
                let Some(rows) = cat_pufferfish::FRAME_SEQUENCE
                    .get(self.frame_index)
                    .and_then(|unique| cat_pufferfish::ART_FRAMES.get(*unique))
                else {
                    return;
                };
                draw_rows(
                    area,
                    buffer,
                    rows,
                    cat_pufferfish::ART_WIDTH,
                    |row, column| {
                        let pair = row.as_bytes().get(column * 2..column * 2 + 2)?;
                        decode_pufferfish_cell(pair[0], pair[1], self.colors)
                    },
                );
            }
        }
    }
}

/// One decoded terminal cell: symbol, foreground, and optional background.
type Cell = Option<(&'static str, Color, Option<Color>)>;

/// Centers `rows` in `area` and writes each decoded cell; skips the art when
/// the area is too small to hold it.
fn draw_rows(
    area: Rect,
    buffer: &mut Buffer,
    rows: &[&str],
    width: usize,
    decode: impl Fn(&str, usize) -> Cell,
) {
    let (Ok(art_width), Ok(art_height)) = (u16::try_from(width), u16::try_from(rows.len())) else {
        return;
    };
    if area.width < art_width || area.height < art_height {
        return;
    }
    let origin_x = area.x + (area.width - art_width) / 2;
    let origin_y = area.y + (area.height - art_height) / 2;
    for (row_index, row) in rows.iter().enumerate() {
        let y = origin_y + u16::try_from(row_index).expect("art height fits u16");
        for column_index in 0..width {
            let Some((symbol, foreground, background)) = decode(row, column_index) else {
                continue;
            };
            let x = origin_x + u16::try_from(column_index).expect("art width fits u16");
            let mut style = Style::default().fg(foreground);
            if let Some(background) = background {
                style = style.bg(background);
            }
            buffer[(x, y)].set_symbol(symbol).set_style(style);
        }
    }
}

fn decode_fox_cell(encoded: u8, colors: ColorMode) -> Cell {
    let code = encoded.checked_sub(b'a')?;
    if code >= 12 {
        return None;
    }
    let palette_index = usize::from(code / 3);
    let density_index = usize::from(code % 3);
    let (density, color) = match colors {
        ColorMode::None => (ASCII_DENSITY, Color::Reset),
        ColorMode::Ansi => (UNICODE_DENSITY, FOX_ANSI[palette_index]),
        ColorMode::TrueColor => (UNICODE_DENSITY, FOX_TRUECOLOR[palette_index]),
    };
    Some((density[density_index], color, None))
}

/// Decodes an upper and a lower pixel into one half-block cell.
fn decode_pufferfish_cell(upper: u8, lower: u8, colors: ColorMode) -> Cell {
    let color = |code: u8| -> Option<Color> {
        let index = cat_pufferfish::PIXEL_CODES
            .iter()
            .position(|candidate| *candidate == code)?;
        match colors {
            ColorMode::None => Some(Color::Reset),
            ColorMode::Ansi => cat_pufferfish::ANSI_PALETTE
                .get(index)
                .map(|index| Color::Indexed(*index)),
            ColorMode::TrueColor => cat_pufferfish::PALETTE
                .get(index)
                .map(|(red, green, blue)| Color::Rgb(*red, *green, *blue)),
        }
    };
    let (upper, lower) = (color(upper), color(lower));
    if matches!(colors, ColorMode::None) {
        return match (upper, lower) {
            (Some(_), Some(_)) => Some(("#", Color::Reset, None)),
            (Some(_), None) | (None, Some(_)) => Some(("+", Color::Reset, None)),
            (None, None) => None,
        };
    }
    match (upper, lower) {
        (Some(upper), Some(lower)) if upper == lower => Some(("█", upper, None)),
        (Some(upper), Some(lower)) => Some(("▀", upper, Some(lower))),
        (Some(upper), None) => Some(("▀", upper, None)),
        (None, Some(lower)) => Some(("▄", lower, None)),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tables_have_the_locked_shape() {
        assert_eq!((fox::ART_WIDTH, fox::ART_HEIGHT), (72, 16));
        assert_eq!(fox::ART_FRAMES.len(), 22);
        assert!(
            fox::ART_FRAMES
                .iter()
                .flatten()
                .all(|row| row.len() == fox::ART_WIDTH)
        );
        assert_eq!(
            (cat_pufferfish::ART_WIDTH, cat_pufferfish::ART_HEIGHT),
            (72, 24)
        );
        // Two pixel codes per cell, each transparent or a palette entry.
        let palette_codes = &cat_pufferfish::PIXEL_CODES[..cat_pufferfish::PALETTE.len()];
        for row in cat_pufferfish::ART_FRAMES.iter().flatten() {
            assert_eq!(row.len(), cat_pufferfish::ART_WIDTH * 2);
            assert!(
                row.bytes()
                    .all(|code| code == b' ' || palette_codes.contains(&code))
            );
        }
        assert_eq!(
            cat_pufferfish::ANSI_PALETTE.len(),
            cat_pufferfish::PALETTE.len()
        );
        assert_eq!(FOX_SEQUENCE_LEN, 56);
        assert_eq!(PUFFERFISH_SEQUENCE_LEN, 56);
        assert!(
            fox::FRAME_SEQUENCE
                .iter()
                .all(|index| *index < fox::ART_FRAMES.len())
        );
        assert!(
            cat_pufferfish::FRAME_SEQUENCE
                .iter()
                .all(|index| *index < cat_pufferfish::ART_FRAMES.len())
        );
    }

    #[test]
    fn half_blocks_carry_both_pixels() {
        let [first, second] = [
            cat_pufferfish::PIXEL_CODES[0],
            cat_pufferfish::PIXEL_CODES[1],
        ];
        let rgb = |index: usize| {
            let (red, green, blue) = cat_pufferfish::PALETTE[index];
            Color::Rgb(red, green, blue)
        };
        let truecolor = ColorMode::TrueColor;
        assert_eq!(
            decode_pufferfish_cell(first, second, truecolor),
            Some(("▀", rgb(0), Some(rgb(1))))
        );
        assert_eq!(
            decode_pufferfish_cell(first, first, truecolor),
            Some(("█", rgb(0), None))
        );
        assert_eq!(
            decode_pufferfish_cell(b' ', second, truecolor),
            Some(("▄", rgb(1), None))
        );
        assert_eq!(decode_pufferfish_cell(b' ', b' ', truecolor), None);
        assert_eq!(
            decode_pufferfish_cell(first, b' ', ColorMode::None),
            Some(("+", Color::Reset, None))
        );
    }
}
