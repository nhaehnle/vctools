// SPDX-License-Identifier: GPL-3.0-or-later

use std::borrow::Cow;

use crate::{event::KeyCode, input, state::Builder, theme};
use ratatui::{
    prelude::*,
    widgets::{Block, Clear},
};

pub enum CommandAction<'action> {
    None,
    Command(String),
    Changed(&'action str),
    Cancelled,
}

#[derive(Debug, Default)]
struct State {
    popup_height: u16,
}

pub struct CommandLine<'bar: 'cmd, 'cmd> {
    id: Cow<'bar, str>,
    command: &'cmd mut Option<String>,
    help: Option<Cow<'bar, str>>,
}
impl<'bar, 'cmd> CommandLine<'bar, 'cmd> {
    pub fn new(id: impl Into<Cow<'bar, str>>, command: &'cmd mut Option<String>) -> Self {
        CommandLine {
            id: id.into(),
            help: None,
            command,
        }
    }

    pub fn help(mut self, help: impl Into<Cow<'bar, str>>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn build<F>(self, builder: &mut Builder, popup: F) -> CommandAction<'cmd>
    where
        F: FnOnce(&mut Builder, Option<&mut String>),
    {
        let state_id = builder.add_state_id(self.id);
        let state: &mut State = builder.get_state(state_id);
        let area = builder.take_lines_fixed(1);

        let mut cancelled = false;

        if self.command.is_some() && builder.on_key_press(KeyCode::Esc) {
            *self.command = None;
            cancelled = true;
        }

        let modal = self.command.is_some();
        builder
            .nest()
            .modal(state_id, modal)
            .theme_context(if modal {
                theme::Context::Modal
            } else {
                theme::Context::None
            })
            .build(|builder| {
                builder.frame().render_widget(Clear, area);
                let block = Block::new().style(
                    builder
                        .theme()
                        .modal_background
                        .patch(builder.theme().modal_text.normal),
                );
                builder.frame().render_widget(block, area);

                let popup_height = std::cmp::min(state.popup_height, area.y);
                let popup_area = Rect {
                    y: area.y - popup_height,
                    height: popup_height,
                    ..area
                };

                let background = builder.theme().modal_background;
                builder
                    .nest()
                    .popup(
                        popup_area,
                        background,
                        popup_height,
                        &mut state.popup_height,
                    )
                    .build(|builder| {
                        popup(builder, self.command.as_mut());
                    });

                if let Some(command) = self.command.as_mut() {
                    match input::Input::new("command")
                        .area(area)
                        .build(builder, command)
                    {
                        Some(input::InputAction::TextChanged) => {
                            if !command.is_empty() {
                                return CommandAction::Changed(self.command.as_ref().unwrap());
                            }
                            *self.command = None;
                            builder.need_refresh();
                            return CommandAction::Cancelled;
                        }
                        Some(input::InputAction::Enter) => {
                            let cmd = self.command.take().unwrap();
                            builder.need_refresh();
                            return CommandAction::Command(cmd);
                        }
                        None => {}
                    }
                } else {
                    let help = Span::from(self.help.unwrap_or("--".into()))
                        .style(builder.theme().modal_text.normal);
                    builder.frame().render_widget(help, area);
                }

                if cancelled {
                    CommandAction::Cancelled
                } else {
                    CommandAction::None
                }
            })
    }
}
