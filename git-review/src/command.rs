// SPDX-License-Identifier: GPL-3.0-or-later

use std::borrow::Cow;

use ratatui::{prelude::*, widgets::Block};
use vctuik::{event::KeyCode, input, label::add_label, prelude::*, state::{self, Builder}, theme::{self, Themed}};


pub enum CommandAction {
    None,
    Command(String),
    Changed,
}

pub struct CommandLine<'bar> {
    id: Cow<'bar, str>,
    command: &'bar mut Option<String>,
    help: Option<Cow<'bar, str>>,
}
impl<'bar> CommandLine<'bar> {
    pub fn new(id: impl Into<Cow<'bar, str>>, command: &'bar mut Option<String>) -> Self {
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

    pub fn build(self, builder: &mut Builder) -> CommandAction {
        let state_id = builder.add_state_id(self.id);
        let area = builder.take_lines_fixed(1);

        if self.command.is_some() && builder.on_key_press(KeyCode::Esc) {
            *self.command = None;
        }

        let modal = self.command.is_some();
        builder.nest()
            .id(state_id)
            .theme_context(if modal { theme::Context::Modal } else { theme::Context::None })
            .build(|builder| {
                let block = Block::new().style(builder.theme().modal_background.patch(builder.theme().modal_text.normal));
                builder.frame().render_widget(block, area);

                if let Some(command) = self.command.as_mut() {
                    match
                        input::Input::new("command")
                            .area(area)
                            .build(builder, command)
                    {
                        Some(input::InputAction::TextChanged) => {
                            if command.is_empty() {
                                *self.command = None;
                                builder.need_refresh();
                            }
                            return CommandAction::Changed;
                        }
                        Some(input::InputAction::Enter) => {
                            let cmd = std::mem::take(command);
                            *self.command = None;
                            builder.need_refresh();
                            return CommandAction::Command(cmd);
                        }
                        None => {},
                    }
                } else {
                    let help = Span::from(self.help.unwrap_or("--".into()))
                        .style(builder.theme().modal_text.normal);
                    builder.frame().render_widget(help, area);
                }
                CommandAction::None
            })
    }
}
