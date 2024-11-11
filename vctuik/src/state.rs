use std::{any::Any, borrow::Cow, collections::HashMap};

use ratatui::{
    crossterm::event::{KeyCode, KeyEventKind}, layout::{Position, Rect}, Frame
};

use crate::event::KeyEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StateId(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderId(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handled {
    Yes,
    No,
}

pub trait IRenderable {
    fn render(&self, frame: &mut Frame);
}

pub enum Renderable<'render> {
    Span(Rect, ratatui::text::Span<'render>),
    Line(Rect, ratatui::text::Line<'render>),
    Text(Rect, ratatui::text::Text<'render>),
    Block(Rect, ratatui::widgets::Block<'render>),
    Other(Box<dyn IRenderable + 'render>),
    None,
}
impl Renderable<'_> {
    pub fn render(self, frame: &mut Frame) {
        match self {
            Renderable::Span(area, span) => frame.render_widget(span, area),
            Renderable::Line(area, line) => frame.render_widget(line, area),
            Renderable::Text(area, text) => frame.render_widget(text, area),
            Renderable::Block(area, block) => frame.render_widget(block, area),
            Renderable::Other(other) => other.render(frame),
            Renderable::None => {},
        }
    }
}

pub trait EventHandler {
    fn handle_key_event(&mut self, _: KeyEvent) -> Handled {
        Handled::No
    }
}

#[derive(Default)]
pub struct StateNodes {
    entries: Vec<(String, Option<Box<dyn Any>>)>,
    id_map: HashMap<String, usize>,
    focus_chain: Vec<StateId>,
    focus: Option<StateId>,
}
impl StateNodes {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.id_map.clear();
        self.focus_chain.clear();
        self.focus = None;
    }

    pub fn get_state<T: 'static>(&self, id: StateId) -> Option<&T> {
        self.entries
            .get(id.0)
            .and_then(|(_, state)| state.as_ref())
            .and_then(|state| state.downcast_ref())
    }

    fn find_in_focus_chain(&self, id: StateId) -> Option<usize> {
        self.focus_chain.iter().position(|&x| x == id)
    }
}

#[derive(Default)]
pub(crate) struct StateStore {
    pub(crate) previous: StateNodes,
    pub(crate) current: StateNodes,
}

pub(crate) struct GlobalEventHandler<'handler> {
    state: &'handler mut StateNodes,
}
impl<'handler> GlobalEventHandler<'handler> {
    pub(crate) fn new(state: &'handler mut StateNodes) -> Self {
        GlobalEventHandler { state }
    }
}
impl EventHandler for GlobalEventHandler<'_> {
    fn handle_key_event(&mut self, ev: KeyEvent) -> Handled {
        if ev.kind == KeyEventKind::Press {
            if ev.code == KeyCode::Tab {
                let mut chain_index =
                    self.state.focus
                        .and_then(|id| self.state.find_in_focus_chain(id))
                        .unwrap_or(self.state.focus_chain.len()) + 1;
                if chain_index >= self.state.focus_chain.len() {
                    chain_index = 0;
                }
                self.state.focus = Some(self.state.focus_chain[chain_index]);
                return Handled::Yes;
            }
            if ev.code == KeyCode::BackTab {
                let mut chain_index =
                    self.state.focus
                        .and_then(|id| self.state.find_in_focus_chain(id))
                        .unwrap_or(0);
                if chain_index == 0 {
                    chain_index = self.state.focus_chain.len();
                }
                self.state.focus = Some(self.state.focus_chain[chain_index - 1]);
                return Handled::Yes;
            }
        }
        Handled::No
    }
}

pub type EventHandlers<'handler> = Vec<Box<dyn EventHandler + 'handler>>;

pub(crate) struct BuildStore<'render, 'handler> {
    pub(crate) state: StateStore,
    pub(crate) render: Vec<Renderable<'render>>,
    pub(crate) handlers: EventHandlers<'handler>,
}
impl<'render, 'handler> BuildStore<'render, 'handler> {
    pub(crate) fn new(mut state: StateStore) -> Self {
        std::mem::swap(&mut state.previous, &mut state.current);
        state.current.clear();

        BuildStore {
            state,
            render: Vec::new(),
            handlers: Vec::new(),
        }
    }
}

pub struct Builder<'builder, 'render, 'handler> {
    store: &'builder mut BuildStore<'render, 'handler>,
    id_prefix: String,
    viewport: Rect,
    position: Position,
}
impl<'builder, 'render, 'handler> Builder<'builder, 'render, 'handler> {
    pub(crate) fn new(store: &'builder mut BuildStore<'render, 'handler>, viewport: Rect) -> Self {
        Builder {
            store,
            id_prefix: String::new(),
            viewport,
            position: Position::new(viewport.x, viewport.y),
        }
    }

    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    pub fn position(&self) -> Position {
        self.position
    }

    pub fn set_position(&mut self, position: Position) {
        self.position = position;
    }

    pub fn take_lines(&mut self, lines: u16) -> Rect {
        if self.position.x > self.viewport.x {
            self.position.x = self.viewport.x;
            self.position.y += 1;
        }

        let lines = std::cmp::min(lines, self.viewport.height - self.position.y.saturating_sub(self.viewport.y));

        let area = Rect {
            x: self.viewport.x,
            y: self.position.y,
            width: self.viewport.width,
            height: lines,
        };
        self.position.y += lines;

        area
    }

    pub fn has_focus(&self, id: StateId) -> bool {
        self.store.state.current.focus == Some(id)
    }

    pub fn add_id<'a, Id>(&'a mut self, id: Id, can_focus: bool) -> StateId
    where
        Id: Into<Cow<'a, str>>,
    {
        self.add_id_with_state(id, can_focus, || ()).0
    }

    pub fn add_id_with_state<'add, Id, T, F>(
        &'add mut self,
        id: Id,
        can_focus: bool,
        f: F,
    ) -> (StateId, &'add mut T)
    where
        Id: Into<Cow<'add, str>>,
        F: FnOnce() -> T,
        T: 'static,
    {
        let mut id: Cow<'_, str> = id.into();

        assert!(!id.is_empty());
        assert!(id.find("##").is_none(), "id cannot contain '##'");

        if !self.id_prefix.is_empty() {
            id = format!("{}-##-{}", self.id_prefix, id).into();
        }

        let old_index = self.store.state.previous.id_map.get(&*id);
        let state = old_index
            .and_then(|index| self.store.state.previous.entries[*index].1.take())
            .filter(|state| state.is::<T>());
        let pre_existing = state.is_some();

        let index = self.store.state.current.entries.len();
        let id = id.into_owned();
        let old = self.store.state.current.id_map.insert(id.clone(), index);
        assert!(old.is_none());

        if can_focus {
            if (self.store.state.previous.focus.is_none() && self.store.state.current.focus.is_none()) ||
               (pre_existing && self.store.state.previous.focus == Some(StateId(*old_index.unwrap()))) {
                self.store.state.current.focus = Some(StateId(index));
            }
            self.store.state.current.focus_chain.push(StateId(index));
        }

        self.store.state.current.entries.push((id, state));

        (
            StateId(index),
            self.store.state.current.entries[index].1
                .get_or_insert_with(|| Box::new(f()))
                .downcast_mut().unwrap(),
        )
    }

    pub fn add_render(&mut self, renderable: Renderable<'render>) -> RenderId {
        self.store.render.push(renderable);
        RenderId(self.store.render.len() - 1)
    }

    pub fn get_render_mut(&mut self, id: RenderId) -> Option<&mut Renderable<'render>> {
        self.store.render.get_mut(id.0)
    }

    pub fn add_event_handler<H: EventHandler + 'handler>(&mut self, h: H) {
        self.store.handlers.push(Box::new(h));
    }

    pub fn nest_id<'id, F, I>(&mut self, id: I, f: F)
    where
        F: FnOnce(&mut Builder<'_, 'render, 'handler>),
        I: Into<Cow<'id, str>>,
    {
        let id = id.into();
        let id_prefix = if self.id_prefix.is_empty() {
            id.to_string()
        } else {
            format!("{}-##-{}", self.id_prefix, id)
        };

        f(&mut Builder {
            store: self.store,
            id_prefix,
            viewport: self.viewport,
            position: self.position,
        });
    }

    pub fn nest_viewport<F>(&mut self, viewport: Rect, f: F)
    where
        F: FnOnce(&mut Builder<'_, 'render, 'handler>),
    {
        f(&mut Builder {
            store: self.store,
            id_prefix: self.id_prefix.clone(),
            viewport,
            position: Position::new(viewport.x, viewport.y),
        });
    }
}
