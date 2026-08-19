//! Converts raw terminal input into reducer-owned application events.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::model::{AppEvent, AppState, KeyInput, TerminalSize};

pub(super) fn terminal_event(event: Event, state: &AppState) -> Option<AppEvent> {
    match event {
        Event::Resize(columns, rows) => Some(AppEvent::Resize(TerminalSize { columns, rows })),
        Event::Paste(text) => Some(AppEvent::Paste(text)),
        Event::Key(key) if key.kind != KeyEventKind::Release => key_input(key).map(AppEvent::Key),
        Event::Mouse(mouse) => super::mouse::pointer_intent(mouse, state).map(AppEvent::Pointer),
        _ => None,
    }
}

pub(super) fn key_input(key: KeyEvent) -> Option<KeyInput> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c' | 'C') => Some(KeyInput::CtrlC),
            KeyCode::Char('u' | 'U') => Some(KeyInput::CtrlU),
            KeyCode::Char('d' | 'D') => Some(KeyInput::CtrlD),
            _ => None,
        };
    }
    if key.modifiers.intersects(
        KeyModifiers::ALT | KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META,
    ) {
        return None;
    }
    match key.code {
        KeyCode::Char(character) => Some(KeyInput::Character(character)),
        KeyCode::Up => Some(KeyInput::Up),
        KeyCode::Down => Some(KeyInput::Down),
        KeyCode::Left => Some(KeyInput::Left),
        KeyCode::Right => Some(KeyInput::Right),
        KeyCode::Tab => Some(KeyInput::Tab),
        KeyCode::BackTab => Some(KeyInput::BackTab),
        KeyCode::Enter => Some(KeyInput::Enter),
        KeyCode::Esc => Some(KeyInput::Escape),
        KeyCode::Home => Some(KeyInput::Home),
        KeyCode::End => Some(KeyInput::End),
        KeyCode::PageUp => Some(KeyInput::PageUp),
        KeyCode::PageDown => Some(KeyInput::PageDown),
        KeyCode::Backspace => Some(KeyInput::Backspace),
        KeyCode::Delete => Some(KeyInput::Delete),
        _ => None,
    }
}
