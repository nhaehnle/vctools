use crate::{prelude::*, event, state::{Builder, Renderable}};

use ratatui::{prelude::*, text::Span};

pub fn add<'builder, 'render, 'handler, 'tmp>(
    builder: &mut Builder<'builder, 'render, 'handler>,
    title: &'tmp str,
    state: &'handler mut bool,
) {
    let id = builder.add_id(title, true);
    let has_focus = builder.has_focus(id);
    let text = format!("[{state_char}] {title}", state_char = if *state { '*' } else { ' ' });
    let area = builder.take_lines(1);

    let mut span = Span::from(text);
    if has_focus {
        span = span.bold();
    }
    builder.add_render(Renderable::Span(area, span));

    if has_focus {
        event::on_key_press(builder, event::KeyCode::Char(' '), |_| *state = !*state);
    }
}
