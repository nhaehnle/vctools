use std::borrow::Cow;

use ratatui::{layout::{Alignment, Rect}, widgets::{block::Title, Block, BorderType, Borders}};

use crate::{
    event::{Event, MouseButton, MouseEventKind}, layout::{Constrained1D, Constraint1D}, state::{Builder, Handled, Renderable}, theme::Context
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

    fn constraint(&self, state: &PaneState) -> Constraint1D {
        if state.collapsed {
            Constraint1D::new_exact(1)
        } else {
            Constraint1D::new_min(1 + self.min_inner_height)
        }
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

#[derive(Debug)]
struct DragState {
    pane_id: String,
}

#[derive(Debug, Default)]
struct PanesState {
    implicit_states: Vec<Option<(String, PaneState)>>,
    mouse_capture: Option<DragState>,
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

    pub fn build<'id, Id>(self, builder: &mut Builder<'_, 'render, 'handler>, id: Id, num_lines: u16)
    where
        Id: Into<Cow<'id, str>>,
    {
        self.build_impl(builder, id.into(), num_lines);
    }

    fn build_impl(self, builder: &mut Builder<'_, 'render, 'handler>, id: Cow<'_, str>, num_lines: u16)
    {
        assert!(self.panes.len() <= u16::MAX as usize);

        if self.panes.is_empty() {
            return;
        }

        let (id, state) = builder.add_state_widget::<PanesState, _>(id, false);

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

        // Update mouse drag.
        let drag_pane_idx =
            state.mouse_capture.as_ref()
                .and_then(|drag| self.panes.iter().position(|pane| pane.pane.config.get_id() == drag.pane_id));
        if drag_pane_idx.is_none() {
            state.mouse_capture = None;
        }

        // Normalize the pane data
        let panes = self.panes.into_iter().zip(&mut state.implicit_states)
            .map(|(pane, implicit_state)| {
                (
                    pane.pane.config,
                    pane.pane.state.unwrap_or_else(|| &mut implicit_state.as_mut().unwrap().1),
                    pane.build,
                )
            })
            .collect::<Vec<_>>();

        // Compute the layout
        let constraints = panes.iter()
            .map(|(config, state, _)| config.constraint(state))
            .collect();
        let sizes = panes.iter()
            .map(|(_, state, _)|
                if state.collapsed {
                    Some(1)
                } else {
                    state.inner_height.map(|height| height.saturating_add(1))
                })
            .collect();
        let layout = Constrained1D::constrain(constraints, num_lines, sizes);

        let area = builder.take_lines(num_lines);

        builder.nest().id(id).build(|builder| {
            let mut row = 0;

            let panes = panes.into_iter().enumerate().filter_map(|(pane_idx, (config, state, build))| {
                let height = std::cmp::min(layout.layout()[pane_idx], area.height - row);
                let pane_area = Rect {
                    y: row,
                    height,
                    ..area
                };
                row += height;

                if pane_area.is_empty() {
                    return None
                }

                let inner_area = Rect::new(pane_area.x, pane_area.y + 1, pane_area.width, pane_area.height - 1);

                let render = builder.add_render(Renderable::None);
                let id = builder.add_widget(&config.title, false);

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
                    (false, _) => config.title.clone(),
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

                if pane_idx != 0 {
                    block = block.title(Title::from("↕").alignment(Alignment::Right));
                }

                *builder.get_render_mut(render).unwrap() = Renderable::Block(pane_area, block);

                Some((state, config, pane_area))
            }).collect::<Vec<_>>();

            if row < area.height {
                let area = Rect {
                    y: row,
                    height: area.height - row,
                    ..area
                };
                builder.add_render(
                    Renderable::Block(area,
                        Block::default()
                            .borders(Borders::NONE)
                            .style(builder.theme().modal_background)));
            }

            struct PanesHandlerState<'handler> {
                panes: Vec<(&'handler mut PaneState, PaneConfig, Rect)>,
                mouse_capture: &'handler mut Option<DragState>,
                layout: Constrained1D,
            }
            impl PanesHandlerState<'_> {
                fn sync_layout(&mut self) {
                    for ((state, _, _), height) in self.panes.iter_mut().zip(self.layout.layout()) {
                        if !state.collapsed {
                            state.inner_height = Some(height.saturating_sub(1));
                        }
                    } 
                }
            }
            let mut state = PanesHandlerState {
                panes,
                mouse_capture: &mut state.mouse_capture,
                layout,
            };
            state.sync_layout();

            if let Some(drag_pane_idx) = drag_pane_idx {
                builder.add_mouse_capture_handler(move |ev| {
                    match ev {
                        Event::Mouse(ev) => {
                            match ev.kind {
                                MouseEventKind::Drag(_) => {
                                    let row = std::cmp::min(std::cmp::max(ev.row, area.y), area.y + area.height - 1);
                                    state.layout.move_start(drag_pane_idx, row - area.y);
                                    state.sync_layout();
                                },
                                _ => *state.mouse_capture = None,
                            }
                            Handled::Yes
                        },
                        _ => Handled::No,
                    }
                });
            } else {
                builder.add_event_handler(move |ev| {
                    match ev {
                        Event::Mouse(ev) if ev.kind == MouseEventKind::Down(MouseButton::Left) => {
                            for (pane_idx, (pane_state, config, area)) in state.panes.iter_mut().enumerate() {
                                if ev.row != area.y || ev.column < area.x || ev.column >= area.x + area.width {
                                    continue;
                                }
                                if ev.column == area.x && config.collapsible {
                                    let new_collapsed = !pane_state.collapsed();
                                    pane_state.set_collapsed(new_collapsed);
                                    state.layout.morph(
                                        pane_idx,
                                        config.constraint(pane_state),
                                        if new_collapsed {
                                            1
                                        } else {
                                            1 + pane_state.inner_height.unwrap_or(0)
                                        });
                                    state.sync_layout();
                                    return Handled::Yes;
                                }

                                if ev.column >= area.x + if config.collapsible { 2 } else { 0 } {
                                    *state.mouse_capture = Some(DragState {
                                        pane_id: config.get_id().into(),
                                    });
                                    return Handled::Yes;
                                }
                            }
                            Handled::No
                        },
                        _ => Handled::No,
                    }
                });
            }
        });
    }
}
