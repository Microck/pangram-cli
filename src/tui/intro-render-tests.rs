use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

use super::intro_render;
use super::model::{AppState, ColorMode, TerminalSize};
use super::render;

fn draw(frame_index: usize, color_mode: ColorMode) -> TestBackend {
    let backend = TestBackend::new(100, 28);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| intro_render::render(frame, frame_index, color_mode))
        .unwrap();
    terminal.backend().clone()
}

#[test]
fn fox_is_centered_while_the_backdrop_starts_at_terminal_default() {
    let backend = draw(0, ColorMode::TrueColor);
    let buffer = backend.buffer();
    let occupied: Vec<_> = (0..28)
        .flat_map(|y| (0..100).map(move |x| (x, y)))
        .filter(|&(x, y)| buffer[(x, y)].symbol() != " ")
        .collect();

    assert!(occupied.len() > 200, "the source frame must remain legible");
    let left = occupied.iter().map(|(x, _)| *x).min().unwrap();
    let right = occupied.iter().map(|(x, _)| *x).max().unwrap();
    let top = occupied.iter().map(|(_, y)| *y).min().unwrap();
    let bottom = occupied.iter().map(|(_, y)| *y).max().unwrap();
    assert!((i32::from(left) - i32::from(99 - right)).abs() <= 1);
    assert!((i32::from(top) - i32::from(27 - bottom)).abs() <= 1);
    assert!(buffer.content().iter().all(|cell| cell.bg == Color::Reset));
}

#[test]
fn fox_backdrop_reaches_the_exact_tui_canvas_after_900ms() {
    let midway = draw(9, ColorMode::TrueColor);
    assert!(
        midway
            .buffer()
            .content()
            .iter()
            .all(|cell| cell.bg == Color::Rgb(9, 9, 9))
    );

    for frame_index in [18, intro_render::FRAME_SEQUENCE.len() - 1] {
        let backend = draw(frame_index, ColorMode::TrueColor);
        assert!(
            backend
                .buffer()
                .content()
                .iter()
                .all(|cell| cell.bg == Color::Rgb(17, 17, 17))
        );
    }
}

#[test]
fn truecolor_is_orange_dominant_with_three_detail_colors() {
    let backend = draw(0, ColorMode::TrueColor);
    let buffer = backend.buffer();
    let count = |color| {
        buffer
            .content()
            .iter()
            .filter(|cell| cell.fg == color)
            .count()
    };

    let orange = count(Color::Rgb(255, 97, 6));
    assert!(orange > count(Color::Rgb(254, 202, 185)));
    assert!(count(Color::Rgb(255, 244, 239)) > 0);
    assert!(count(Color::Rgb(111, 41, 0)) > 0);
}

#[test]
fn no_color_uses_only_ascii_density_glyphs_and_reset_colors() {
    let backend = draw(0, ColorMode::None);
    let buffer = backend.buffer();
    for cell in buffer.content() {
        assert!(matches!(cell.symbol(), " " | "." | "+" | "#"));
        assert_eq!(cell.fg, Color::Reset);
        assert_eq!(cell.bg, Color::Reset);
    }
}

#[test]
fn last_playback_frame_is_fully_dissolved() {
    let backend = draw(intro_render::FRAME_SEQUENCE.len() - 1, ColorMode::Ansi);
    assert!(
        backend
            .buffer()
            .content()
            .iter()
            .all(|cell| cell.symbol() == " ")
    );
}

fn draw_tui(opacity: Option<u16>) -> TestBackend {
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = AppState::default();
    state.color_mode = ColorMode::TrueColor;
    state.terminal = TerminalSize {
        columns: 120,
        rows: 40,
    };
    terminal
        .draw(|frame| match opacity {
            Some(opacity) => render::render_faded(frame, &state, opacity),
            None => render::render(frame, &state),
        })
        .unwrap();
    terminal.backend().clone()
}

#[test]
fn analyze_fade_starts_invisible_and_ends_at_the_exact_normal_frame() {
    let hidden = draw_tui(Some(0));
    assert!(
        hidden
            .buffer()
            .content()
            .iter()
            .all(|cell| { cell.fg == Color::Rgb(17, 17, 17) && cell.bg == Color::Rgb(17, 17, 17) })
    );

    let complete = draw_tui(Some(10_000));
    let normal = draw_tui(None);
    assert_eq!(complete.buffer(), normal.buffer());
}
