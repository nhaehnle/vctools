use std::cell::{Cell, RefCell};

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    event::{KeyCode, MouseButton},
    state::Builder,
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

pub fn add_check_box<'s>(
    builder: &mut Builder,
    title: &str,
    state: impl CheckBoxStateRef<'s>,
)
{
    let state_id = builder.add_state_id(title);
    let mut state = state.as_check_box_state();
    let has_focus = builder.check_focus(state_id);

    let text_width = title.graphemes(true).count() as u16;

    let area = builder.take_lines_fixed(1);
    let click_area = Rect { width: 4 + text_width, ..area };

    if builder.on_mouse_press(click_area, MouseButton::Left).is_some() ||
       (has_focus && builder.on_key_press(KeyCode::Char(' '))) {
        state.toggle();
    }

    let text = format!("[{state_char}] {title}", state_char = if state.get() { '*' } else { ' ' });

    let mut span = Span::from(text);
    if has_focus {
        span = span.theme_highlight(builder);
        builder.frame().set_cursor_position(Position::new(area.x + 1, area.y));
    } else {
        span = span.theme_text(builder);
    }
    builder.frame().render_widget(span, area);

    // TODO: Set focus via mouse
}
