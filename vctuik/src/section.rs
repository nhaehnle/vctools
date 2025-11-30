// SPDX-License-Identifier: GPL-3.0-or-later

use ratatui::{
    crossterm::event::KeyCode,
    layout::{Alignment, Rect},
    widgets::{block::Title, Block, BorderType, Borders},
};

use crate::{
    event::{Event, MouseButton, MouseEventKind},
    state::{Builder, StateId},
};

#[derive(Debug, Default)]
struct State {
    collapsed: bool,
    dragging: bool,
}

pub struct Section {
    id: Option<StateId>,
    title: String,
    collapsible: bool,
}
impl Section {
    pub fn new<T>(title: T) -> Self
    where
        T: Into<String>,
    {
        let title = title.into();
        Self {
            id: None,
            title,
            collapsible: false,
        }
    }

    pub fn id(mut self, id: StateId) -> Self {
        self.id = Some(id);
        self
    }

    pub fn collapsible(mut self, collapsible: bool) -> Self {
        self.collapsible = collapsible;
        self
    }

    pub fn build<F>(self, builder: &mut Builder, f: F) -> bool
    where
        F: FnOnce(&mut Builder),
    {
        let state_id = self.id.unwrap_or_else(|| builder.add_state_id(&self.title));
        let state: &mut State = builder.get_state(state_id);

        let is_first = builder.is_at_top();

        builder.nest().id(state_id).build(|builder| {
            let header_area = builder.take_lines_fixed(1);
            let has_focus = builder.check_group_focus(state_id);

            // Handle input
            let adjust = std::cmp::min(header_area.width, if self.collapsible { 2 } else { 0 });
            let drag_area = Rect {
                x: header_area.x + adjust,
                width: header_area.width - adjust,
                ..header_area
            };
            let collapse_area = Rect {
                width: 1,
                ..header_area
            };

            if !is_first
                && builder
                    .on_mouse_press(drag_area, MouseButton::Left)
                    .is_some()
            {
                state.dragging = true;
            } else if !is_first && state.dragging {
                if let Some(Event::Mouse(ev)) = builder.peek_event() {
                    if matches!(ev.kind, MouseEventKind::Drag(_)) {
                        let delta = ev.row as i16 - header_area.y as i16;
                        if delta != 0 {
                            builder.layout_drag(header_area.y, delta);
                        }
                    } else {
                        state.dragging = false;
                    }
                }
            } else if self.collapsible
                && builder
                    .on_mouse_press(collapse_area, MouseButton::Left)
                    .is_some()
            {
                state.collapsed = !state.collapsed;
            }

            // Draw header
            let title = match (self.collapsible, state.collapsed) {
                (true, true) => format!("▶ {}", self.title),
                (true, false) => format!("▼ {}", self.title),
                (false, _) => self.title.clone(),
            };

            let mut block = Block::default()
                .title(title)
                .borders(Borders::TOP)
                .style(builder.theme().pane_background);

            if has_focus {
                block = block.border_type(BorderType::Thick);
                block = block.border_style(builder.theme().pane_frame_focus);
            } else {
                block = block.border_type(BorderType::Plain);
                block = block.border_style(builder.theme().pane_frame_normal);
            }

            if !is_first {
                block = block.title(Title::from("↕").alignment(Alignment::Right));
            }

            builder.frame().render_widget(block, header_area);

            if !state.collapsed {
                f(builder);
            }

            if self.collapsible && has_focus {
                if builder.on_key_press(KeyCode::Left) && !state.collapsed {
                    state.collapsed = true;
                    builder.need_refresh();
                }
                if builder.on_key_press(KeyCode::Right) && state.collapsed {
                    state.collapsed = false;
                    builder.need_refresh();
                }
            }
        });

        !state.collapsed
    }
}

pub fn with_section<'a, 'outer_builder, 'inner_builder, 'store, 'frame, F, R>(
    builder: &'a mut Builder<'outer_builder, 'store, 'frame>,
    title: impl Into<String>,
    f: F,
) -> Option<R>
where
    F: FnOnce(&mut Builder<'_, '_, '_>) -> R,
{
    let mut result = None;
    Section::new(title)
        .collapsible(true)
        .build(builder, |builder| {
            result = Some(f(builder));
        });
    result
}
