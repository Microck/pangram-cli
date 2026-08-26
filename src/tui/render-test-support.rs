use ratatui::Terminal;
use ratatui::backend::{Backend, TestBackend};
use ratatui::style::{Color, Modifier};

use super::render;
use crate::tui::model::{AppState, SettingsDraft, StartupState, TerminalSize};

pub(super) struct Screen {
    pub(super) cells: Vec<Vec<String>>,
    pub(super) styles: Vec<Vec<CellStyle>>,
    pub(super) cursor: Option<(u16, u16)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CellStyle {
    pub(super) foreground: Color,
    pub(super) background: Color,
    pub(super) modifier: Modifier,
}

impl Screen {
    pub(super) fn row(&self, y: usize) -> String {
        self.cells[y].concat()
    }

    pub(super) fn text(&self) -> String {
        self.cells
            .iter()
            .map(|row| row.concat())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(super) fn style(&self, x: usize, y: usize) -> CellStyle {
        self.styles[y][x]
    }
}

pub(super) fn ready_state(width: u16, height: u16) -> AppState {
    AppState::new(
        TerminalSize {
            columns: width,
            rows: height,
        },
        StartupState {
            settings: SettingsDraft {
                credential_present: true,
                update_preference: Some(false),
                ..SettingsDraft::default()
            },
            ..StartupState::default()
        },
    )
}

pub(super) fn draw(width: u16, height: u16, state: &AppState) -> Screen {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("create test terminal");
    terminal
        .draw(|frame| render(frame, state))
        .expect("render TUI frame");
    let buffer = terminal.backend().buffer();
    let (cells, styles): (Vec<Vec<String>>, Vec<Vec<CellStyle>>) = (0..height)
        .map(|y| {
            (0..width)
                .map(|x| {
                    let cell = &buffer[(x, y)];
                    (
                        cell.symbol().to_owned(),
                        CellStyle {
                            foreground: cell.fg,
                            background: cell.bg,
                            modifier: cell.modifier,
                        },
                    )
                })
                .unzip()
        })
        .unzip();
    let position = terminal
        .get_cursor_position()
        .expect("read test cursor position");
    let backend = terminal.backend();
    let mut hidden = backend.clone();
    hidden.hide_cursor().expect("hide cloned test cursor");
    let cursor = (*backend != hidden).then_some((position.x, position.y));
    Screen {
        cells,
        styles,
        cursor,
    }
}
