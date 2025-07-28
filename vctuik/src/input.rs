use std::borrow::Cow;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Position;
use ratatui::widgets::Block;
use ratatui::{layout::Rect, text::Span};
use unicode_segmentation::UnicodeSegmentation;
use tui_input::backend::crossterm::EventHandler;

use crate::state::Builder;
use crate::theme::Themed;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputAction {
    TextChanged,
    Enter,
}

fn update_text(input: &mut tui_input::Input, text: &str) {
    if input.value() != text {
        // TODO: Should we try to preserve the cursor position?
        *input = tui_input::Input::new(text.into());
    }
}

pub struct Input<'input> {
    id: Cow<'input, str>,
    area: Option<Rect>,
    label: Option<Cow<'input, str>>,
}
impl<'input> Input<'input> {
    pub fn new(id: impl Into<Cow<'input, str>>) -> Self {
        Input {
            id: id.into(),
            area: None,
            label: None,
        }
    }

    pub fn area(mut self, area: Rect) -> Self {
        self.area = Some(area);
        self
    }

    pub fn label(mut self, label: impl Into<Cow<'input, str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn build(self, builder: &mut Builder, text: &mut String) -> Option<InputAction> {
        let state_id = builder.add_state_id(self.id);
        let area = self.area.unwrap_or_else(|| builder.take_lines_fixed(1));
        let mut has_focus = builder.check_focus(state_id);

        let label_width =
            if let Some(label) = &self.label {
                std::cmp::min(label.graphemes(true).count() as u16 + 1, area.width)
            } else {
                0
            };

        let label_area = Rect {
            width: label_width,
            ..area
        };
        let input_area = Rect {
            x: area.x + label_width,
            width: area.width - label_width,
            ..area
        };

        let input: &mut tui_input::Input = builder.get_state(state_id);
        update_text(input, text);

        // Handle events
        let scroll = input.visual_scroll(input_area.width as usize);

        if let Some(pos) = builder.on_mouse_press(input_area, ratatui::crossterm::event::MouseButton::Left) {
            if has_focus {
                let pos = pos.x.saturating_sub(input_area.x) as usize + scroll;
                input.handle(tui_input::InputRequest::SetCursor(pos));
            }
            
            builder.grab_focus(state_id);
            has_focus = true;
        }

        let action =
            if has_focus {
                if builder.on_key_press(KeyCode::Enter) {
                    Some(InputAction::Enter)
                } else if builder.on_key_press(KeyCode::Esc) {
                    builder.drop_focus(state_id);
                    None
                } else {
                    builder.with_event(|ev| {
                        // Let tui-input handle most key events (typing, cursor movement, etc.)
                        input.handle_event(ev)
                            .and_then(|change| {
                                if change.value {
                                    *text = input.value().into();
                                    Some(InputAction::TextChanged)
                                } else {
                                    None
                                }
                            })
                    })
                }
            } else {
                None
            };

        // Render the field
        if let Some(label) = self.label {
            let label = Span::from(label).theme_text(builder);
            builder.frame().render_widget(label, label_area);
        }

        let style =
            if has_focus {
                    builder.theme().modal_text.highlight
                } else {
                    builder.theme().modal_text.normal
                };
        let block = Block::new()
            .style(builder.theme().modal_background.patch(style));
        builder.frame().render_widget(block, input_area);

        let display_text: String = text.chars()
            .skip(scroll)
            .take(area.width as usize)
            .collect();

        let themed_span =
            Span::styled(
                display_text,
                style);
        builder.frame().render_widget(themed_span, input_area);

        if has_focus {
            let cursor_pos = input.visual_cursor().saturating_sub(scroll);
            let cursor_x = input_area.x + std::cmp::min(cursor_pos as u16, input_area.width.saturating_sub(1));
            builder.frame().set_cursor_position(Position::new(cursor_x, area.y));
        }

        action
    }
}
