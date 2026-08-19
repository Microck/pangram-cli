//! Semantic terminal styles and transition colors shared by the TUI renderers.

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};

use crate::tui::model::ColorMode;

const OPACITY_SCALE: u16 = 10_000;

pub(in crate::tui) const fn canvas_color(color_mode: ColorMode) -> Color {
    match color_mode {
        ColorMode::None => Color::Reset,
        ColorMode::Ansi => Color::Indexed(233),
        ColorMode::TrueColor => Color::Rgb(17, 17, 17),
    }
}

pub(in crate::tui) fn base_style(color_mode: ColorMode) -> Style {
    match color_mode {
        ColorMode::None => Style::default(),
        ColorMode::Ansi | ColorMode::TrueColor => Style::default()
            .fg(Color::White)
            .bg(canvas_color(color_mode)),
    }
}

pub(in crate::tui) fn panel_style(color_mode: ColorMode) -> Style {
    match color_mode {
        ColorMode::None => Style::default(),
        ColorMode::Ansi => Style::default().bg(Color::Indexed(235)),
        ColorMode::TrueColor => Style::default().bg(Color::Rgb(28, 28, 28)),
    }
}

pub(in crate::tui) fn element_style(color_mode: ColorMode) -> Style {
    match color_mode {
        ColorMode::None => Style::default(),
        ColorMode::Ansi => Style::default().fg(Color::White).bg(Color::Indexed(236)),
        ColorMode::TrueColor => Style::default().fg(Color::White).bg(Color::Rgb(42, 42, 42)),
    }
}

pub(in crate::tui) fn control_style(
    color_mode: ColorMode,
    focused: bool,
    selected: bool,
    available: bool,
) -> Style {
    if color_mode == ColorMode::None {
        return if focused || selected {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
    }
    if !available {
        return muted_style(color_mode);
    }
    if selected {
        return action_style(color_mode);
    }
    if focused {
        return element_style(color_mode)
            .fg(match color_mode {
                ColorMode::Ansi => Color::Indexed(202),
                ColorMode::TrueColor => Color::Rgb(255, 97, 6),
                ColorMode::None => unreachable!(),
            })
            .add_modifier(Modifier::BOLD);
    }
    element_style(color_mode)
}

pub(in crate::tui) fn action_style(color_mode: ColorMode) -> Style {
    match color_mode {
        ColorMode::None => Style::default(),
        ColorMode::Ansi => Style::default()
            .fg(canvas_color(color_mode))
            .bg(Color::Indexed(202)),
        ColorMode::TrueColor => Style::default()
            .fg(canvas_color(color_mode))
            .bg(Color::Rgb(255, 97, 6)),
    }
    .add_modifier(Modifier::BOLD)
}

pub(in crate::tui) fn body_style(color_mode: ColorMode) -> Style {
    match color_mode {
        ColorMode::None => Style::default(),
        ColorMode::Ansi | ColorMode::TrueColor => Style::default().fg(Color::White),
    }
}

pub(in crate::tui) fn primary_style(color_mode: ColorMode) -> Style {
    let style = match color_mode {
        ColorMode::None => Style::default(),
        ColorMode::Ansi => Style::default().fg(Color::Indexed(202)),
        ColorMode::TrueColor => Style::default().fg(Color::Rgb(255, 97, 6)),
    };
    style.add_modifier(Modifier::BOLD)
}

pub(in crate::tui) fn route_style(color_mode: ColorMode, selected: bool) -> Style {
    if selected {
        return action_style(color_mode);
    }
    element_style(color_mode)
}

pub(in crate::tui) fn muted_style(color_mode: ColorMode) -> Style {
    match color_mode {
        ColorMode::None => Style::default(),
        ColorMode::Ansi => Style::default().fg(Color::Indexed(245)),
        ColorMode::TrueColor => Style::default().fg(Color::Rgb(138, 138, 138)),
    }
}

pub(in crate::tui) fn separator_style(color_mode: ColorMode) -> Style {
    match color_mode {
        ColorMode::None => Style::default(),
        ColorMode::Ansi | ColorMode::TrueColor => Style::default().fg(Color::DarkGray),
    }
}

/// Fades a completed TUI buffer in from its base canvas without changing
/// layout, symbols, or semantic style ownership.
pub(in crate::tui) fn fade_buffer(buffer: &mut Buffer, color_mode: ColorMode, opacity: u16) {
    if color_mode == ColorMode::None || opacity >= OPACITY_SCALE {
        return;
    }
    let area = buffer.area;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buffer[(x, y)];
            cell.fg = fade_from_canvas(cell.fg, color_mode, opacity);
            cell.bg = fade_from_canvas(cell.bg, color_mode, opacity);
        }
    }
}

pub(in crate::tui) fn fade_from_black(
    target: Color,
    color_mode: ColorMode,
    level: usize,
    levels: usize,
) -> Color {
    if color_mode == ColorMode::None || level == 0 {
        return Color::Reset;
    }
    if level >= levels {
        return target;
    }
    let target_rgb = color_rgb(target).unwrap_or((0, 0, 0));
    let opacity = u16::try_from(level.saturating_mul(usize::from(OPACITY_SCALE)) / levels)
        .unwrap_or(OPACITY_SCALE);
    mode_color(blend_rgb((0, 0, 0), target_rgb, opacity), color_mode)
}

fn fade_from_canvas(target: Color, color_mode: ColorMode, opacity: u16) -> Color {
    let canvas = color_rgb(canvas_color(color_mode)).expect("colored canvas has a fixed RGB value");
    let target = color_rgb(target).unwrap_or(canvas);
    mode_color(blend_rgb(canvas, target, opacity), color_mode)
}

fn blend_rgb(from: (u8, u8, u8), to: (u8, u8, u8), opacity: u16) -> (u8, u8, u8) {
    let blend = |from: u8, to: u8| {
        let from = i32::from(from);
        let delta = i32::from(to) - from;
        let value = from
            + (delta * i32::from(opacity) + i32::from(OPACITY_SCALE) / 2)
                / i32::from(OPACITY_SCALE);
        u8::try_from(value).expect("blending two bytes remains a byte")
    };
    (
        blend(from.0, to.0),
        blend(from.1, to.1),
        blend(from.2, to.2),
    )
}

fn mode_color(rgb: (u8, u8, u8), color_mode: ColorMode) -> Color {
    match color_mode {
        ColorMode::None => Color::Reset,
        ColorMode::Ansi => Color::Indexed(nearest_ansi(rgb)),
        ColorMode::TrueColor => Color::Rgb(rgb.0, rgb.1, rgb.2),
    }
}

fn color_rgb(color: Color) -> Option<(u8, u8, u8)> {
    match color {
        Color::Rgb(red, green, blue) => Some((red, green, blue)),
        Color::Indexed(index) => Some(indexed_rgb(index)),
        Color::White => Some((255, 255, 255)),
        Color::DarkGray => Some((128, 128, 128)),
        Color::Reset => None,
        _ => None,
    }
}

fn indexed_rgb(index: u8) -> (u8, u8, u8) {
    if index >= 232 {
        let gray = 8 + 10 * (index - 232);
        return (gray, gray, gray);
    }
    if index >= 16 {
        const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
        let cube = index - 16;
        return (
            CUBE[usize::from(cube / 36)],
            CUBE[usize::from((cube % 36) / 6)],
            CUBE[usize::from(cube % 6)],
        );
    }
    const SYSTEM: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (128, 0, 0),
        (0, 128, 0),
        (128, 128, 0),
        (0, 0, 128),
        (128, 0, 128),
        (0, 128, 128),
        (192, 192, 192),
        (128, 128, 128),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (0, 0, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    SYSTEM[usize::from(index)]
}

fn nearest_ansi(rgb: (u8, u8, u8)) -> u8 {
    const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let component = |value: u8| {
        CUBE.iter()
            .enumerate()
            .min_by_key(|(_, candidate)| i16::from(value).abs_diff(i16::from(**candidate)))
            .map(|(index, _)| index)
            .expect("the ANSI color cube is non-empty")
    };
    let red = component(rgb.0);
    let green = component(rgb.1);
    let blue = component(rgb.2);
    let cube_index = 16 + 36 * red + 6 * green + blue;
    let cube_rgb = (CUBE[red], CUBE[green], CUBE[blue]);

    let average = (u16::from(rgb.0) + u16::from(rgb.1) + u16::from(rgb.2)) / 3;
    let gray_step = average.saturating_sub(8).saturating_add(5) / 10;
    let gray_step = gray_step.min(23);
    let gray = u8::try_from(8 + 10 * gray_step).expect("ANSI grayscale remains a byte");
    let gray_index = 232 + usize::from(gray_step);

    if distance(rgb, (gray, gray, gray)) < distance(rgb, cube_rgb) {
        u8::try_from(gray_index).expect("ANSI grayscale index remains a byte")
    } else {
        u8::try_from(cube_index).expect("ANSI cube index remains a byte")
    }
}

fn distance(left: (u8, u8, u8), right: (u8, u8, u8)) -> u32 {
    let square = |left: u8, right: u8| u32::from(left.abs_diff(right)).pow(2);
    square(left.0, right.0) + square(left.1, right.1) + square(left.2, right.2)
}
