pub use ratatui::crossterm::event::*;
use ratatui::layout::{Position, Rect};

use crate::state::{Builder, Handled};

pub fn on_key_press<'handler>(
    builder: &mut Builder<'_, '_, 'handler>,
    key_code: KeyCode,
    mut callback: impl FnMut(&KeyEvent) + 'handler,
) {
    let handler = move |event: &Event| {
        match event {
            Event::Key(ev) if ev.kind == KeyEventKind::Press && ev.code == key_code => {
                callback(ev);
                Handled::Yes
            }
            _ => Handled::No,
        }
    };
    builder.add_event_handler(handler);
}

pub fn on_mouse_press<'handler>(
    builder: &mut Builder<'_, '_, 'handler>,
    area: Rect,
    button: MouseButton,
    mut callback: impl FnMut(&MouseEvent) + 'handler,
) {
    let handler = move |event: &Event| {
        match event {
            Event::Mouse(ev) if ev.kind == MouseEventKind::Down(button)
                && area.contains(Position::new(ev.column, ev.row)) =>
            {
                callback(ev);
                Handled::Yes
            }
            _=> Handled::No,                        
        }
    };
    builder.add_event_handler(handler);
}
