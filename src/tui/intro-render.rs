//! Renders generated fox cells without decoding or loading runtime assets.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;

use super::model::ColorMode;
use super::render::{canvas_color, fade_from_black};

include!("intro-frames.rs");

const TRUECOLOR_PALETTE: [Color; 4] = [
    Color::Rgb(111, 41, 0),
    Color::Rgb(255, 97, 6),
    Color::Rgb(254, 202, 185),
    Color::Rgb(255, 244, 239),
];
const ANSI_PALETTE: [Color; 4] = [
    Color::Indexed(94),
    Color::Indexed(202),
    Color::Indexed(217),
    Color::Indexed(230),
];
const UNICODE_DENSITY: [&str; 3] = ["░", "▒", "█"];
const ASCII_DENSITY: [&str; 3] = [".", "+", "#"];
const BACKDROP_FADE_FRAME_COUNT: usize = 18;

pub(crate) fn render(frame: &mut ratatui::Frame<'_>, frame_index: usize, colors: ColorMode) {
    let area = frame.area();
    frame.render_widget(
        IntroArt {
            frame_index,
            colors,
        },
        area,
    );
}

struct IntroArt {
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

        let Some(&unique_index) = FRAME_SEQUENCE.get(self.frame_index) else {
            return;
        };
        let Some(rows) = ART_FRAMES.get(unique_index) else {
            return;
        };
        let Ok(art_width) = u16::try_from(ART_WIDTH) else {
            return;
        };
        let Ok(art_height) = u16::try_from(ART_HEIGHT) else {
            return;
        };
        if area.width < art_width || area.height < art_height {
            return;
        }

        let origin_x = area.x + (area.width - art_width) / 2;
        let origin_y = area.y + (area.height - art_height) / 2;
        for (row_index, row) in rows.iter().enumerate() {
            for (column_index, encoded) in row.bytes().enumerate() {
                let Some((symbol, color)) = decode_cell(encoded, self.colors) else {
                    continue;
                };
                let x = origin_x + u16::try_from(column_index).expect("art width fits u16");
                let y = origin_y + u16::try_from(row_index).expect("art height fits u16");
                buffer[(x, y)]
                    .set_symbol(symbol)
                    .set_style(Style::default().fg(color));
            }
        }
    }
}

fn decode_cell(encoded: u8, colors: ColorMode) -> Option<(&'static str, Color)> {
    let code = encoded.checked_sub(b'a')?;
    if code >= 12 {
        return None;
    }
    let palette_index = usize::from(code / 3);
    let density_index = usize::from(code % 3);
    let (density, color) = match colors {
        ColorMode::None => (ASCII_DENSITY, Color::Reset),
        ColorMode::Ansi => (UNICODE_DENSITY, ANSI_PALETTE[palette_index]),
        ColorMode::TrueColor => (UNICODE_DENSITY, TRUECOLOR_PALETTE[palette_index]),
    };
    Some((density[density_index], color))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tables_have_the_locked_shape() {
        assert_eq!(ART_WIDTH, 72);
        assert_eq!(ART_HEIGHT, 16);
        assert_eq!(ART_FRAMES.len(), 22);
        assert_eq!(FRAME_SEQUENCE.len(), 56);
        assert!(
            ART_FRAMES
                .iter()
                .flatten()
                .all(|row| row.len() == ART_WIDTH)
        );
        assert!(FRAME_SEQUENCE.iter().all(|index| *index < ART_FRAMES.len()));
    }
}
