//! Unicode-safe editing for terminal text fields.

use super::model::KeyInput;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct TextField {
    value: String,
    cursor: usize,
}

impl TextField {
    #[cfg(test)]
    pub fn from_value(value: String) -> Self {
        let cursor = value.len();
        Self { value, cursor }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub(super) fn before_cursor(&self) -> &str {
        &self.value[..self.cursor]
    }

    pub(super) fn insert(&mut self, character: char) {
        self.value.insert(self.cursor, character);
        self.cursor += character.len_utf8();
    }

    pub(super) fn edit(&mut self, key: KeyInput) -> bool {
        edit_value(&mut self.value, &mut self.cursor, key)
    }
}

pub(super) fn edit_value(value: &mut String, cursor: &mut usize, key: KeyInput) -> bool {
    match key {
        KeyInput::Character(character) if !character.is_control() => {
            value.insert(*cursor, character);
            *cursor += character.len_utf8();
        }
        KeyInput::Backspace => {
            if let Some((previous, _)) = value[..*cursor].char_indices().next_back() {
                value.remove(previous);
                *cursor = previous;
            }
        }
        KeyInput::Delete => {
            if *cursor < value.len() {
                value.remove(*cursor);
            }
        }
        KeyInput::Left => {
            if let Some((previous, _)) = value[..*cursor].char_indices().next_back() {
                *cursor = previous;
            }
        }
        KeyInput::Right => {
            if let Some(character) = value[*cursor..].chars().next() {
                *cursor += character.len_utf8();
            }
        }
        KeyInput::Home => *cursor = 0,
        KeyInput::End => *cursor = value.len(),
        KeyInput::Escape
        | KeyInput::Enter
        | KeyInput::Tab
        | KeyInput::BackTab
        | KeyInput::Up
        | KeyInput::Down => {
            return false;
        }
        _ => return true,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_around_multibyte_characters_without_splitting_them() {
        let accent = '\u{e9}';
        let mut field = TextField::from_value(format!("a{accent}fox"));
        for key in [
            KeyInput::Left,
            KeyInput::Left,
            KeyInput::Left,
            KeyInput::Left,
            KeyInput::Character('z'),
            KeyInput::Right,
            KeyInput::Delete,
            KeyInput::Home,
            KeyInput::Delete,
            KeyInput::End,
            KeyInput::Backspace,
        ] {
            assert!(field.edit(key));
        }
        assert_eq!(field.value(), format!("z{accent}o"));

        let mut boundary = TextField::from_value(format!("a{accent}"));
        assert!(boundary.edit(KeyInput::Left));
        assert_eq!(boundary.before_cursor(), "a");
        assert!(boundary.edit(KeyInput::Right));
        assert_eq!(boundary.before_cursor(), format!("a{accent}"));
    }

    #[test]
    fn navigation_owned_by_the_parent_is_not_consumed() {
        let mut field = TextField::from_value("query".to_owned());
        for key in [
            KeyInput::Escape,
            KeyInput::Enter,
            KeyInput::Tab,
            KeyInput::BackTab,
            KeyInput::Up,
            KeyInput::Down,
        ] {
            assert!(!field.edit(key));
        }
        assert_eq!(field.value(), "query");
    }
}
