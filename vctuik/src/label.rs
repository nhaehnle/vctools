use std::borrow::Cow;

use ratatui::text::Span;

use crate::{state::{Builder, Renderable}, theme::Themed};


pub fn add_label<'builder, 'render, 'handler, T>(
    builder: &mut Builder<'builder, 'render, 'handler>,
    title: T)
where
    T: Into<Cow<'render, str>>,
{
    let area = builder.take_lines(1);
    let span = Span::from(title).theme_text(builder);
    builder.add_render(Renderable::Span(area, span));
}
