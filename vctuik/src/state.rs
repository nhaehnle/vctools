use std::{borrow::Cow, collections::HashMap};

use ratatui::{
    layout::{Position, Rect},
    Frame,
};

use vctuik_unsafe_internals::state;

use crate::{
    event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind},
    layout::{Constraint1D, LayoutCache, LayoutEngine, LayoutItem1D},
    theme::{Context, Theme}
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StateId(usize);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Focus {
    ghost: Option<String>,

    /// Index into the focus chain.
    index: usize,
}

#[derive(Debug)]
struct IdEntry {
    name: String,
    other_id: Option<StateId>,
}

#[derive(Default)]
struct IdState {
    id_map: HashMap<String, StateId>,
    ids: Vec<IdEntry>,
    focus_chain: Vec<StateId>,
    focus: Option<Focus>,
}
impl IdState {
    fn clear(&mut self) {
        self.id_map.clear();
        self.ids.clear();
        self.focus_chain.clear();
        self.focus = None;
    }
}

#[derive(Default)]
struct IdStore {
    previous: IdState,
    current: IdState,
}
impl IdStore {
    pub fn add_state_id(&mut self, name: String) -> StateId {
        let previous = &mut self.previous;
        let current = &mut self.current;

        let old_id = previous.id_map.get(&*name).map(|x| *x);
        let new_id = StateId(current.ids.len());

        if let Some(old_id) = old_id {
            previous.ids[old_id.0].other_id = Some(new_id);
        }

        let name: String = name.into();
        assert!(!self.current.id_map.contains_key(&name));
        self.current.id_map.insert(name.clone(), new_id);
        self.current.ids.push(IdEntry { name, other_id: old_id });

        new_id
    }
}

#[derive(Default)]
struct LayoutStore {
    previous: LayoutCache<StateId>,
    current: LayoutCache<StateId>,
}

#[derive(Default)]
pub(crate) struct Store {
    ids: IdStore,
    layout: LayoutStore,
    state: state::Store<StateId>,
}

pub(crate) struct BuildStore<'store, 'frame> {
    ids: &'store mut IdStore,
    layout: &'store mut LayoutStore,
    state_builder: state::Builder<'store, StateId>,
    pub(crate) frame: &'store mut Frame<'frame>,
    theme: &'store Theme,
    event: Option<Event>,
    event_handled: bool,
    pub(crate) need_refresh: bool,
}
impl<'store, 'frame> BuildStore<'store, 'frame> {
    pub(crate) fn new(state: &'store mut Store, theme: &'store Theme,
                      frame: &'store mut Frame<'frame>,
                      event: Option<Event>) -> Self {
        let ids = &mut state.ids;
        let layout = &mut state.layout;
        let state_builder = state::Builder::new(&mut state.state);

        BuildStore {
            ids,
            layout,
            state_builder,
            frame,
            theme,
            event,
            event_handled: false,
            need_refresh: false,
        }
    }

    fn event_handled(&mut self) -> bool {
        let ret = !self.event_handled;
        self.event_handled = true;
        ret
    }

    pub fn current_layout_mut(&mut self) -> &mut LayoutCache<StateId> {
        &mut self.layout.current
    }

    pub fn end_frame(&mut self) {
        // Focus handling
        //
        // Ghost tracking and initial focus selection
        if self.ids.current.focus.is_none() {
            // If we previously had focus (possibly a ghost)...
            if let Some(focus) = &mut self.ids.previous.focus {
                let was_ghost = focus.ghost.is_some();

                // ... either maintain the old ghost or raise a new one
                let ghost =
                    focus.ghost
                        .take()
                        .unwrap_or_else(|| {
                            let old_id = self.ids.previous.focus_chain[focus.index];
                            self.ids.previous.ids[old_id.0].name.clone()
                        });
                // ... iterate over the old focus chain starting from the old index
                let index =
                    self.ids.previous.focus_chain[focus.index..].iter()
                        .chain(self.ids.previous.focus_chain[..focus.index].iter())
                        // ... and find the first ID that has a current correspondent
                        .find_map(|old_id| {
                            self.ids.previous.ids[old_id.0].other_id
                                .and_then(|new_id| {
                                    self.ids.current.focus_chain
                                        .iter()
                                        .enumerate()
                                        .find(|(_, id)| **id == new_id)
                                        .map(|(index, _)| index)
                                })
                        })
                        .unwrap_or(0);

                // Track the ghost
                self.ids.current.focus = Some(Focus {
                    ghost: Some(ghost),
                    index,
                });

                if !was_ghost {
                    // May have to redraw a section header
                    self.need_refresh = true;
                }
            }
        }

        // We handle keyboard events last, after all other widgets have had a chance to intercept
        // so that text fields can capture tabs.
        if !self.ids.current.focus_chain.is_empty() && !self.event_handled {
            let Some(Focus { ghost, index }) = &self.ids.current.focus else { unreachable!() };

            let new_focus = match self.event {
                Some(Event::Key(ev)) if ev.kind == KeyEventKind::Press => {
                    if ev.code == KeyCode::Tab || ev.code == KeyCode::Down {
                        Some(Focus {
                            ghost: None,
                            index:
                                if ghost.is_some() { *index }
                                else if index + 1 < self.ids.current.focus_chain.len() { index + 1 }
                                else { 0 },
                        })
                    } else if ev.code == KeyCode::BackTab || ev.code == KeyCode::Up {
                        Some(Focus {
                            ghost: None,
                            index: if *index > 0 { index - 1 } else { self.ids.current.focus_chain.len() - 1},
                        })
                    } else {
                        None
                    }
                },
                _ => None,
            };
            if new_focus.is_some() {
                self.ids.current.focus = new_focus;
                self.event_handled = true;
                self.need_refresh = true;
            }
        }

        // State double-buffer management
        self.layout.current.save_persistent(
            std::mem::take(&mut self.layout.previous),
            |old_id| {
                self.ids.previous.ids[old_id.0].other_id.unwrap_or_else(|| {
                    self.ids.add_state_id(self.ids.previous.ids[old_id.0].name.clone())
                })
            });

        self.ids.previous.clear();
        self.layout.previous.clear();
        for IdEntry { other_id, .. } in self.ids.current.ids.iter_mut() {
            *other_id = None;
        }
        std::mem::swap(&mut self.ids.previous, &mut self.ids.current);
        std::mem::swap(&mut self.layout.previous, &mut self.layout.current);
    }
}

pub struct Builder<'builder, 'store, 'frame> {
    store: &'builder mut BuildStore<'store, 'frame>,
    name_prefix: String,
    theme_context: Context,
    viewport: Rect,
    layout: &'builder mut LayoutEngine<StateId>,
}
impl<'builder, 'store, 'frame> Builder<'builder, 'store, 'frame> {
    pub(crate) fn new(store: &'builder mut BuildStore<'store, 'frame>, layout: &'builder mut LayoutEngine<StateId>, viewport: Rect) -> Self {
        Builder {
            store,
            name_prefix: String::new(),
            theme_context: Context::None,
            viewport,
            layout,
        }
    }

    pub fn context(&self) -> Context {
        self.theme_context
    }

    pub fn frame<'slf>(&'slf mut self) -> &'slf mut Frame<'frame> {
        &mut self.store.frame
    }

    pub fn theme(&self) -> &Theme {
        self.store.theme
    }

    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    pub fn is_at_top(&self) -> bool {
        self.layout.position() == 0
    }

    pub fn take_lines(&mut self, item: LayoutItem1D<StateId>) -> Rect {
        let old_id = item.id.and_then(|id| self.store.ids.current.ids[id.0].other_id);
        let (pos, size) = self.layout.add(&self.store.layout.previous, old_id, item);

        let rel_y = std::cmp::min(pos, self.viewport.height);
        let height = std::cmp::min(size, self.viewport.height - rel_y);

        let area = Rect {
            x: self.viewport.x,
            y: self.viewport.y + rel_y,
            width: self.viewport.width,
            height,
        };

        area
    }

    pub fn take_lines_fixed(&mut self, lines: u16) -> Rect {
        self.take_lines(LayoutItem1D::new(Constraint1D::new_fixed(lines)))
    }

    pub fn add_slack(&mut self) {
        let state_id = self.add_state_id("_slack".into());
        self.take_lines(LayoutItem1D::new(Constraint1D::unconstrained()).id(state_id, true));
    }

    pub fn layout_drag(&mut self, y: u16, delta: i16) {
        self.layout.drag(y, delta);
    }

    pub fn has_group_focus(&self) -> bool {
        let name =
            // See if we already found focus this frame...
            self.store.ids.current.focus
                .as_ref()
                .map(|focus| {
                    let id = self.store.ids.current.focus_chain[focus.index];
                    &self.store.ids.current.ids[id.0].name
                })
                // ... else use the focus from last frame if there was one
                .or_else(|| {
                    self.store.ids.previous.focus
                        .as_ref()
                        .and_then(|focus| {
                            focus.ghost.is_none()
                                .then(|| {
                                    let id = self.store.ids.previous.focus_chain[focus.index];
                                    &self.store.ids.previous.ids[id.0].name
                                })
                        })
                });
        name.and_then(|name| {
            name.strip_prefix(&self.name_prefix)
                .map(|suffix| suffix.starts_with("-##-"))
        }).unwrap_or(false)
    }

    pub fn check_focus(&mut self, id: StateId) -> bool {
        // Register the state ID as being able to receive focus.
        self.store.ids.current.focus_chain.push(id);

        let Some(old_focus) = self.store.ids.previous.focus.as_ref() else {
            // Initial focus selection in the first frame.
            if self.store.ids.current.focus.is_none() {
                self.store.ids.current.focus = Some(Focus {
                    ghost: None,
                    index: self.store.ids.current.focus_chain.len() - 1,
                });
                self.store.need_refresh = true;
                return true;
            } else {
                return false;
            }
        };

        // Carry old focus forward.
        let inherit_focus =
            if let Some(old_id) = self.store.ids.current.ids[id.0].other_id {
                old_focus.ghost.is_none() &&
                    self.store.ids.previous.focus_chain[old_focus.index] == old_id
            } else if let Some(ghost) = &old_focus.ghost {
                ghost == &self.store.ids.current.ids[id.0].name
            } else {
                false
            };

        if inherit_focus {
            self.store.ids.current.focus = Some(Focus {
                ghost: None,
                index: self.store.ids.current.focus_chain.len() - 1,
            });
            true
        } else {
            false
        }
    }

    pub fn grab_focus(&mut self, id: StateId) {
        let (idx, _) = self.store.ids.current.focus_chain
            .iter()
            .enumerate()
            .find(|(_, &x)| x == id)
            .unwrap();
        self.store.ids.current.focus = Some(Focus {
            ghost: None,
            index: idx,
        });
        self.store.need_refresh = true;
    }

    pub fn drop_focus(&mut self, id: StateId) {
        if let Some(focus) = &mut self.store.ids.current.focus {
            if focus.ghost.is_none() && self.store.ids.current.focus_chain[focus.index] == id {
                focus.ghost = Some(self.store.ids.current.ids[id.0].name.clone());
            } 
        }
        self.store.need_refresh = true;
    }

    pub fn need_refresh(&mut self) {
        self.store.need_refresh = true;
    }

    pub fn peek_event(&self) -> Option<&Event> {
        self.store.event.as_ref()
    }

    pub fn with_event<F, R>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&Event) -> Option<R>,
    {
        if !self.store.event_handled {
            if let Some(event) = &self.store.event {
                if let Some(result) = f(event) {
                    self.store.event_handled = true;
                    return Some(result);
                }
            }
        }
        None
    }

    pub fn on_key_press(&mut self, key_code: KeyCode) -> bool {
        matches!(
            self.store.event,
            Some(Event::Key(ev)) if ev.kind == KeyEventKind::Press && ev.code == key_code) &&
            self.store.event_handled()
    }

    pub fn on_key_press_any(&mut self, key_codes: &[KeyCode]) -> bool {
        match self.store.event {
        Some(Event::Key(ev)) if ev.kind == KeyEventKind::Press => {
            key_codes.contains(&ev.code) && self.store.event_handled()
        },
        _ => false,
        }
    }

    pub fn on_mouse_press(&mut self, area: Rect, button: MouseButton) -> Option<Position> {
        match self.store.event {
            Some(Event::Mouse(ev)) if ev.kind == MouseEventKind::Down(button) => {
                let pos = Position::new(ev.column, ev.row);
                area.contains(pos).then_some(pos).filter(|_| self.store.event_handled())
            },
            _ => None,
        }
    }

    pub fn on_mouse_scroll_down(&mut self, area: Rect) -> Option<Position> {
        match self.store.event {
            Some(Event::Mouse(ev)) if ev.kind == MouseEventKind::ScrollDown => {
                let pos = Position::new(ev.column, ev.row);
                area.contains(pos).then_some(pos).filter(|_| self.store.event_handled())
            },
            _ => None,
        }
    }

    pub fn on_mouse_scroll_up(&mut self, area: Rect) -> Option<Position> {
        match self.store.event {
            Some(Event::Mouse(ev)) if ev.kind == MouseEventKind::ScrollUp => {
                let pos = Position::new(ev.column, ev.row);
                area.contains(pos).then_some(pos).filter(|_| self.store.event_handled())
            },
            _ => None,
        }
    }

    pub fn on_mouse_scroll_left(&mut self, area: Rect) -> Option<Position> {
        match self.store.event {
            Some(Event::Mouse(ev)) if ev.kind == MouseEventKind::ScrollLeft => {
                let pos = Position::new(ev.column, ev.row);
                area.contains(pos).then_some(pos).filter(|_| self.store.event_handled())
            },
            _ => None,
        }
    }

    pub fn on_mouse_scroll_right(&mut self, area: Rect) -> Option<Position> {
        match self.store.event {
            Some(Event::Mouse(ev)) if ev.kind == MouseEventKind::ScrollRight => {
                let pos = Position::new(ev.column, ev.row);
                area.contains(pos).then_some(pos).filter(|_| self.store.event_handled())
            },
            _ => None,
        }
    }

    pub fn add_state_id(&mut self, mut name: Cow<'_, str>) -> StateId {
        assert!(!name.is_empty());
        assert!(name.find("##").is_none(), "id cannot contain '##'");

        if !self.name_prefix.is_empty() {
            name = format!("{}-##-{}", self.name_prefix, name).into();
        }

        self.store.ids.add_state_id(name.into())
    }

    pub fn get_state<'add, S>(&mut self, id: StateId) -> &'add mut S
    where
        S: Default + 'static,
        'store: 'add,
    {
        let old_id = self.store.ids.current.ids[id.0].other_id;
        self.store.state_builder.get_or_insert_default(id, old_id)
    }

    pub fn nest<'nest>(&'nest mut self) -> Nest<'nest, 'store, 'frame> {
        Nest {
            parent: None,
            builder: Builder { 
                store: self.store,
                name_prefix: self.name_prefix.clone(),
                theme_context: self.theme_context,
                viewport: self.viewport,
                layout: self.layout,
            },
        }
    }
}

pub struct Nest<'nest, 'store, 'frame> {
    builder: Builder<'nest, 'store, 'frame>,
    parent: Option<StateId>,
}
impl<'nest, 'store, 'frame> Nest<'nest, 'store, 'frame> {
    pub fn build<F, R>(mut self, f: F) -> R
    where
        F: FnOnce(&mut Builder<'nest, 'store, 'frame>) -> R,
    {
        f(&mut self.builder)
    }

    pub fn id(self, id: StateId) -> Self {
        assert!(id.0 == self.builder.store.ids.current.ids.len() - 1);
        assert!(self.parent.is_none());

        let name_prefix = self.builder.store.ids.current.ids[id.0].name.clone();

        Nest {
            builder: Builder {
                name_prefix,
                ..self.builder
            },
            parent: Some(id),
            ..self
        }
    }

//    pub fn viewport(self, viewport: Rect) -> Self {
//        Nest {
//            builder: Builder {
//                viewport,
//                position: Position::new(viewport.x, viewport.y),
//                ..self.builder
//            },
//            ..self
//        }
//    }

    pub fn theme_context(self, theme_context: Context) -> Self {
        Nest {
            builder: Builder {
                theme_context,
                ..self.builder
            },
            ..self
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NestResult {
    pub has_focus: bool,
}
