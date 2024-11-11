use std::cell::RefCell;

use crate::{prelude::*, event, state::{Builder, Renderable}, theme::Themed};

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

pub fn add_checkbox<'builder, 'render, 'handler, 'tmp, S>(
    builder: &mut Builder<'builder, 'render, 'handler>,
    title: &'tmp str,
    state: S,
)
where
    S: CheckBoxStateRef<'handler>,
{
    let mut state = state.as_check_box_state();

    let id = builder.add_id(title, true);
    let has_focus = builder.has_focus(id);
    let text = format!("[{state_char}] {title}", state_char = if state.get() { '*' } else { ' ' });
    let area = builder.take_lines(1);

    let mut span = Span::from(text);
    if has_focus {
        span = span.theme_highlight(builder);
    } else {
        span = span.theme_text(builder);
    }
    builder.add_render(Renderable::Span(area, span));

    if has_focus {
        event::on_key_press(builder, event::KeyCode::Char(' '), move |_| state.toggle());
    }
}
