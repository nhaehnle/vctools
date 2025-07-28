use std::borrow::Cow;

use ratatui::text::Span;

use crate::{state::Builder, theme::Themed};

pub fn add_label<'title, T>(
    builder: &mut Builder,
    title: T,
) where
    T: Into<Cow<'title, str>>,
{
    let area = builder.take_lines_fixed(1);
    let span = Span::from(title).theme_text(builder);
    builder.frame().render_widget(span, area);
}
