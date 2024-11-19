use std::cell::{Cell, RefCell};

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    event::{Event, KeyCode, KeyEventKind, MouseEventKind, MouseButton},
    state::{Builder, Handled, Renderable},
    theme::Themed,
};

use ratatui::{prelude::*, text::Span};

pub trait CheckBoxState {
    fn get(&self) -> bool;
    fn toggle(&mut self);
}

pub trait CheckBoxStateRef<'a> {
    fn as_check_box_state(self) -> impl CheckBoxState + 'a;
}

impl<'a> CheckBoxStateRef<'a> for &'a mut bool {
    fn as_check_box_state(self) -> impl CheckBoxState + 'a {
        struct CheckBoxStateImpl<'a>(&'a mut bool);
        impl CheckBoxState for CheckBoxStateImpl<'_> {
            fn get(&self) -> bool {
                *self.0
            }
            fn toggle(&mut self) {
                *self.0 = !*self.0;
            }
        }
        CheckBoxStateImpl(self)
    }
}
impl<'a> CheckBoxStateRef<'a> for &'a RefCell<bool> {
    fn as_check_box_state(self) -> impl CheckBoxState + 'a {
        struct CheckBoxStateImpl<'a>(&'a RefCell<bool>);
        impl CheckBoxState for CheckBoxStateImpl<'_> {
            fn get(&self) -> bool {
                *self.0.borrow()
            }
            fn toggle(&mut self) {
                let mut state = self.0.borrow_mut();
                *state = !*state;
            }
        }
        CheckBoxStateImpl(self)
    }
}
impl<'a> CheckBoxStateRef<'a> for &'a RefCell<&'a mut bool> {
    fn as_check_box_state(self) -> impl CheckBoxState + 'a {
        struct CheckBoxStateImpl<'a>(&'a RefCell<&'a mut bool>);
        impl<'a> CheckBoxState for CheckBoxStateImpl<'a> {
            fn get(&self) -> bool {
                **self.0.borrow()
            }
            fn toggle(&mut self) {
                let mut state = self.0.borrow_mut();
                **state = !**state;
            }
        }
        CheckBoxStateImpl(self)
    }
}

impl<'a> CheckBoxStateRef<'a> for &'a Cell<bool> {
    fn as_check_box_state(self) -> impl CheckBoxState + 'a {
        struct CheckBoxStateImpl<'a>(&'a Cell<bool>);
        impl CheckBoxState for CheckBoxStateImpl<'_> {
            fn get(&self) -> bool {
                self.0.get()
            }
            fn toggle(&mut self) {
                self.0.set(!self.0.get());
            }
        }
        CheckBoxStateImpl(self)
    }
}

pub fn add_check_box<'builder, 'render, 'handler, 'tmp, S>(
    builder: &mut Builder<'builder, 'render, 'handler>,
    title: &'tmp str,
    state: S,
)
where
    S: CheckBoxStateRef<'handler>,
{
    let mut state = state.as_check_box_state();

    let text_width = title.graphemes(true).count() as u16;

    let id = builder.add_widget(title, true);
    let has_focus = builder.has_focus(id);
    let text = format!("[{state_char}] {title}", state_char = if state.get() { '*' } else { ' ' });
    let area = builder.take_lines(1);

    let mut span = Span::from(text);
    if has_focus {
        span = span.theme_highlight(builder);
        builder.add_render(Renderable::SetCursor(Position::new(area.x + 1, area.y)));
    } else {
        span = span.theme_text(builder);
    }
    builder.add_render(Renderable::Span(area, span));

    let click_area = Rect { width: 4 + text_width, ..area };

    builder.add_event_handler(move |event| {
        match event {
            Event::Key(ev) if has_focus && ev.kind == KeyEventKind::Press
                    && ev.code == KeyCode::Char(' ') => {
                state.toggle();
                Handled::Yes
            }
            Event::Mouse(ev) if ev.kind == MouseEventKind::Down(MouseButton::Left)
                    && click_area.contains(Position::new(ev.column, ev.row)) => {
                state.toggle();
                Handled::Yes
            }
            _ => Handled::No,
        }
    });

    // TODO: Set focus via mouse
}
