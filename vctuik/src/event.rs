pub use ratatui::crossterm::event::*;

use crate::state::{Builder, EventHandler, Handled};

pub fn on_key_press<'handler>(
    builder: &mut Builder<'_, '_, 'handler>,
    key_code: KeyCode,
    callback: impl FnMut(Event) + 'handler,
) {
    struct Impl<C> {
        key_code: KeyCode,
        callback: C,
    }
    impl<C: FnMut(Event)> EventHandler for Impl<C> {
        fn handle_key_event(&mut self, event: KeyEvent) -> Handled {
            if event.kind == KeyEventKind::Press && event.code == self.key_code {
                (self.callback)(Event::Key(event));
                Handled::Yes
            } else {
                Handled::No
            }
        }
    }
    builder.add_event_handler(Impl {
        key_code,
        callback,
    });
}
