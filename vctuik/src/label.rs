use std::borrow::Cow;

use ratatui::{text::{Span, Text}};

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

pub fn add_multiline_label<'title, T>(
    builder: &mut Builder,
    text: T,
) where
    T: Into<Cow<'title, str>>,
{
    let text = Text::raw(text).theme_text(builder);
    add_text_label(builder, text);
}

pub fn add_text_label(builder: &mut Builder, text: Text) {
    let area = builder.take_lines_fixed(text.lines.len() as u16);
    builder.frame().render_widget(text, area);
}
