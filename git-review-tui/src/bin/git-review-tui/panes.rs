use std::{collections::HashMap, fmt};

use ratatui::{
    prelude::*,
    crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers},
    widgets::{Block, BorderType, Borders, StatefulWidget, Widget},
};

use vctuik::theme::{Theme, Themed};

#[derive(Debug)]
pub enum Response {
    NotHandled,
    Handled,
    Route(usize, Event),
}

#[derive(Debug, Default)]
struct PaneState {
    visible: bool,
}

#[derive(Debug)]
pub struct PanesState {
    panes: HashMap<usize, PaneState>,
    focus: usize,
    tab_order: Vec<usize>,
}
impl PanesState {
    pub fn new() -> Self {
        Self::default()
    }

    fn get_state(&self, key: usize) -> Option<&PaneState> {
        self.panes.get(&key)
    }

    fn get_state_mut(&mut self, key: usize) -> &mut PaneState {
        self.panes.entry(key).or_insert(PaneState {
            visible: true,
            ..Default::default()
        })
    }

    pub fn set_visible(&mut self, key: usize, visible: bool) {
        self.get_state_mut(key).visible = visible;
    }

    pub fn is_visible(&self, key: usize) -> bool {
        self.get_state(key).map_or(true, |state| state.visible)
    }

    pub fn handle_event(&mut self, ev: Event) -> Response {
        match ev {
            Event::Key(key) => {
                if key.code == KeyCode::Tab &&
                   !KeyModifiers::SHIFT.complement().intersects(key.modifiers) {
                    if key.kind == KeyEventKind::Press && !self.tab_order.is_empty() {
                        let backwards = key.modifiers.contains(KeyModifiers::SHIFT);

                        let order_idx =
                            *self.tab_order.iter()
                                .find(|&&x| x == self.focus)
                                .unwrap_or(&self.tab_order.len());
                        if backwards {
                            if order_idx > 0 {
                                self.focus = self.tab_order[order_idx - 1];
                            } else {
                                self.focus = self.tab_order[self.tab_order.len() - 1];
                            }
                        } else {
                            if order_idx < self.tab_order.len() - 1 {
                                self.focus = self.tab_order[order_idx + 1];
                            } else {
                                self.focus = self.tab_order[0];
                            }
                        }
                    }
                    return Response::Handled;
                }
                Response::Route(self.focus, ev)
            }
            _ => { Response::NotHandled }
        }
    }
}
impl Default for PanesState {
    fn default() -> Self {
        Self {
            panes: HashMap::new(),
            focus: 0,
            tab_order: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct PanesLayout {
    panes: HashMap<usize, Rect>,
}
impl PanesLayout {
    pub fn inner(&self, key: usize) -> Option<Rect> {
        self.panes.get(&key).map(|rect| *rect)
    }
}

trait MutWidgetRef {
    fn render_ref(&mut self, area: Rect, buf: &mut Buffer);
}

struct WidgetBox<'state>(Box<dyn MutWidgetRef + 'state>);
impl<'state> fmt::Debug for WidgetBox<'state> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WidgetBox")
    }
}

struct TypeErasedWidget<W: Widget> {
    widget: Option<W>,
}
impl<'state, W: Widget> MutWidgetRef for TypeErasedWidget<W> {
    fn render_ref(&mut self, area: Rect, buf: &mut Buffer) {
        self.widget.take().unwrap().render(area, buf);
    }
}

struct TypeErasedStatefulWidget<'state, W: StatefulWidget> {
    widget: Option<W>,
    state: &'state mut W::State,
}
impl<'state, W: StatefulWidget> MutWidgetRef for TypeErasedStatefulWidget<'state, W> {
    fn render_ref(&mut self, area: Rect, buf: &mut Buffer) {
        self.widget.take().unwrap().render(area, buf, &mut self.state);
    }
}

#[derive(Debug)]
pub struct Pane<'state> {
    key: usize,
    title: String,
    constraint: Option<Constraint>,
    widget: Option<WidgetBox<'state>>,
}
impl<'state> Pane<'state> {
    pub fn new(key: usize, title: &str) -> Self {
        Self {
            key,
            title: title.into(),
            constraint: None,
            widget: None,
        }
    }

    pub fn constraint(mut self, constraint: Constraint) -> Self {
        self.constraint = Some(constraint);
        self
    }

    pub fn widget<W: Widget + 'state>(mut self, widget: W) -> Self {
        let boxed = Box::new(TypeErasedWidget { widget: Some(widget) });
        self.widget = Some(WidgetBox(boxed));
        self
    }

    pub fn stateful_widget<W: StatefulWidget + 'state>(mut self, widget: W, state: &'state mut W::State) -> Self {
        let boxed = Box::new(TypeErasedStatefulWidget {
            widget: Some(widget),
            state,
        });
        self.widget = Some(WidgetBox(boxed));
        self
    }
}

#[derive(Debug)]
pub struct Panes<'state, 'theme> {
    panes: Vec<Pane<'state>>,
    theme: Option<&'theme Theme>,
}
impl<'state, 'theme> Panes<'state, 'theme> {
    pub fn new(panes: Vec<Pane<'state>>) -> Self {
        Self {
            panes,
            theme: None,
        }
    }

    pub fn add_pane(mut self, pane: Pane<'state>) -> Self {
        self.panes.push(pane);
        self
    }

    pub fn theme(mut self, theme: &'theme Theme) -> Self {
        self.theme = Some(theme);
        self
    }

    pub fn layout(&self, state: &PanesState, area: Rect) -> PanesLayout {
        let panes: Vec<_> = self.panes.iter().filter(|pane| {
            state.get_state(pane.key).map_or(true, |state| state.visible)
        }).collect();

        let constraints: Vec<_> = panes.iter().map(|pane| {
            vec![
                Constraint::Length(1),
                pane.constraint.unwrap_or(Constraint::Percentage(100 / self.panes.len() as u16))
            ]
        }).flatten().collect();
        let layout = Layout::new(Direction::Vertical, constraints).split(area);

        PanesLayout {
            panes: panes.into_iter().zip(layout.into_iter().skip(1).step_by(2))
                .map(|(pane, &layout)| (pane.key, layout))
                .collect(),
        }
    }
}
impl<'state, 'theme> StatefulWidget for Panes<'state, 'theme> {
    type State = PanesState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let layout = self.layout(state, area);

        let panes = self.panes.into_iter().filter_map(|pane| {
            layout.inner(pane.key).map(|inner_area| (pane, inner_area))
        }).collect::<Vec<_>>();

        state.tab_order = panes.iter().map(|(pane, _)| pane.key).collect();

        if state.tab_order.iter().find(|&&x| x == state.focus).is_none() {
            state.focus = state.tab_order[0];
        }

        for (pane, inner_area) in panes {
            let pane_area = Rect {
                x: inner_area.x,
                y: inner_area.y - 1,
                width: inner_area.width,
                height: inner_area.height + 1,
            };

            let mut block = Block::default()
                .title(pane.title)
                .borders(Borders::TOP);

            if state.focus == pane.key {
                block = block.border_type(BorderType::Thick)
            } else {
                block = block.border_type(BorderType::Plain)
            }

            if let Some(theme) = self.theme {
                block = block.style(theme.pane_background);
                if state.focus == pane.key {
                    block = block.border_style(theme.pane_frame_highlighted);
                } else {
                    block = block.border_style(theme.pane_frame_normal);
                }
            }

            block.render(pane_area, buf);

            if let Some(mut widget) = pane.widget {
                widget.0.render_ref(inner_area, buf);
            }
        }
    }
}
