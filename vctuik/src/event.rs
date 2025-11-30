// SPDX-License-Identifier: GPL-3.0-or-later

use std::any::Any;

pub use ratatui::crossterm::event::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeySequence {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}
impl KeySequence {
    pub fn new(key: KeyCode, modifiers: KeyModifiers) -> Self {
        KeySequence {
            code: key,
            modifiers,
        }
        .normalized()
    }

    pub fn matches(&self, ev: &KeyEvent) -> bool {
        self.code == ev.code && self.modifiers == ev.modifiers
    }

    fn normalized(mut self) -> Self {
        let ch = match self.code {
            KeyCode::Char(c) => c,
            _ => return self,
        };

        if ch.is_uppercase() {
            self.modifiers.insert(KeyModifiers::SHIFT);
        } else if self.modifiers.contains(KeyModifiers::SHIFT) {
            let mut uppercase = ch.to_uppercase();
            if uppercase.len() == 1 {
                self.code = KeyCode::Char(uppercase.next().unwrap());
            }
        }

        self
    }
}
impl From<KeyCode> for KeySequence {
    fn from(code: KeyCode) -> Self {
        KeySequence::new(code, KeyModifiers::empty())
    }
}

pub trait WithModifiers {
    type Output;
    fn with_modifiers(self, modifiers: KeyModifiers) -> Self::Output;
}
impl WithModifiers for KeyCode {
    type Output = KeySequence;

    fn with_modifiers(self, modifiers: KeyModifiers) -> Self::Output {
        KeySequence::new(self, modifiers)
    }
}

pub(crate) enum EventExt {
    Event(Event),
    Custom(Box<dyn Any + Send + Sync>),
}
impl std::fmt::Debug for EventExt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventExt::Event(ev) => write!(f, "EventExt::Event({:?})", ev),
            EventExt::Custom(_) => write!(f, "EventExt::Custom(..)"),
        }
    }
}
