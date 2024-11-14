use std::borrow::Cow;

use ratatui::{layout::Rect, widgets::{Block, BorderType, Borders}};

use crate::{
    event::{Event, MouseButton, MouseEventKind},
    state::{Builder, Handled, Renderable},
    theme::Context,
};

#[derive(Debug, Default)]
pub struct PaneState {
    collapsed: bool,
    inner_height: Option<u16>,
}
impl PaneState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn collapsed(&self) -> bool {
        self.collapsed
    }

    pub fn set_collapsed(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
    }
}

struct PaneConfig {
    title: String,
    collapsible: bool,
    min_inner_height: u16,
}
impl PaneConfig {
    fn get_id(&self) -> &str {
        &self.title
    }
}

pub struct Pane<'handler> {
    config: PaneConfig,
    state: Option<&'handler mut PaneState>,
}
impl<'handler> Pane<'handler> {
    pub fn new<T>(title: T) -> Self
    where 
        T: Into<String>
    {
        Self {
            config: PaneConfig {
                title: title.into(),
                collapsible: true,
                min_inner_height: 3,
            },
            state: None,
        }
    }

    pub fn collapsible(mut self, collapsible: bool) -> Self {
        self.config.collapsible = collapsible;
        self
    }

    pub fn state(mut self, state: &'handler mut PaneState) -> Self {
        self.state = Some(state);
        self
    }
}

struct CompletePane<'panes, 'render, 'handler> {
    pane: Pane<'handler>,
    build: Box<dyn FnOnce(&mut Builder<'_, 'render, 'handler>) + 'panes>,
}

#[derive(Debug, Default)]
pub struct PanesState {
    implicit_states: Vec<Option<(String, PaneState)>>,
}

pub struct Panes<'panes, 'render, 'handler> {
    panes: Vec<CompletePane<'panes, 'render, 'handler>>,
}
impl<'panes, 'render, 'handler: 'render> Panes<'panes,'render, 'handler> {
    pub fn new() -> Self {
        Self { panes: Vec::new() }
    }

    pub fn add<F>(&mut self, pane: Pane<'handler>, build: F)
    where
        F: FnOnce(&mut Builder<'_, 'render, 'handler>) + 'panes,
    {
        self.panes.push(CompletePane { pane, build: Box::new(build) });
    }

    pub fn build<'id, Id>(self, builder: &mut Builder<'_, 'render, 'handler>, id: Id, num_lines: u16, state: &'handler mut PanesState)
    where
        Id: Into<Cow<'id, str>>,
    {
        assert!(self.panes.len() <= u16::MAX as usize);

        if self.panes.is_empty() {
            return;
        }

        let id = id.into();
        let id = builder.add_id(id, false);

        // Re-map and build the implicit states for panes that don't have externally
        // owned state.
        state.implicit_states = self.panes.iter().map(|pane| {
            let pane_id = pane.pane.config.get_id();
            if pane.pane.state.is_none() {
                Some(state.implicit_states.iter_mut()
                    .find(|entry| matches!(entry, Some((implicit_id, _)) if implicit_id == pane_id))
                    .map(|entry| entry.take())
                    .flatten()
                    .unwrap_or_else(|| (pane_id.to_string(), PaneState::new())))
            } else {
                None
            }
        }).collect();

        // Normalize the pane data
        let mut panes = self.panes.into_iter().zip(&mut state.implicit_states)
            .map(|(pane, implicit_state)| {
                (
                    pane.pane.config,
                    pane.pane.state.unwrap_or_else(|| &mut implicit_state.as_mut().unwrap().1),
                    pane.build,
                )
            })
            .collect::<Vec<_>>();

        // Calculate the initial height and distribute height to new uncollapsed panes
        let new_panes: Vec<_> =
            panes.iter().enumerate()
                .filter(|(_, (_, state, _))| !state.collapsed && state.inner_height.is_none())
                .map(|(idx, _)| idx)
                .collect();

        let mut height =
            panes.iter_mut()
                .map(|(config, state, _)| {
                    if state.inner_height.unwrap_or(0) < config.min_inner_height {
                        state.inner_height = Some(config.min_inner_height);
                    }
                    1 + (!state.collapsed).then_some(state.inner_height).flatten().unwrap_or(0)
                })
                .fold(0, u16::saturating_add);

        if !new_panes.is_empty() {
            let remaining_height = num_lines.saturating_sub(height);
            let height_assignment = remaining_height / (new_panes.len() as u16);

            for idx in &new_panes {
                *panes[*idx].1.inner_height.as_mut().unwrap() += height_assignment;
            }

            height += height_assignment * new_panes.len() as u16;
        }

        // Fixup heights to outer constraint
        for (config, state, _) in panes.iter_mut().rev() {
            if height == num_lines {
                break;
            }
            if state.collapsed {
                continue;
            }

            let inner_height = state.inner_height.as_mut().unwrap();
            if height < num_lines {
                *inner_height += num_lines - height;
                height = num_lines;
                break;
            }

            let excess = height - num_lines;
            let shrink = std::cmp::min(*inner_height - config.min_inner_height, excess);
            *inner_height -= shrink;
            height -= shrink;
        }

        builder.nest().id(id).build(|builder| {
            let mut panes = panes.into_iter().filter_map(|(config, state, build)| {
                let height = 1 + (!state.collapsed).then_some(state.inner_height).flatten().unwrap_or(0);
                let area = builder.take_lines(height);
                if area.is_empty() {
                    return None
                }

                let inner_area = Rect::new(area.x, area.y + 1, area.width, area.height - 1);

                let render = builder.add_render(Renderable::None);
                let id = builder.add_id(&config.title, false);

                let has_focus =
                    if state.collapsed {
                        false
                    } else {
                        let nest = builder.nest().id(id).context(Context::Pane).viewport(inner_area).build(build);
                        nest.has_focus
                    };

                let title = match (config.collapsible, state.collapsed()) {
                    (true, true) => format!("▶ {}", config.title),
                    (true, false) => format!("▼ {}", config.title),
                    (false, _) => config.title,
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

                *builder.get_render_mut(render).unwrap() = Renderable::Block(area, block);

                Some((state, config.collapsible, area))
            }).collect::<Vec<_>>();

            if height < num_lines {
                let area = builder.take_lines(num_lines - height);
                builder.add_render(
                    Renderable::Block(area,
                        Block::default()
                            .borders(Borders::NONE)
                            .style(builder.theme().pane_background)));
            }

            builder.add_event_handler(move |ev| {
                match ev {
                    Event::Mouse(ev) if ev.kind == MouseEventKind::Down(MouseButton::Left) => {
                        for (state, collapsible, area) in &mut panes {
                            if *collapsible && ev.column == area.x && ev.row == area.y {
                                state.set_collapsed(!state.collapsed());
                                return Handled::Yes;
                            }
                        }
                        Handled::No
                    },
                    _ => { Handled::No },
                }
            });
        });
    }
}
