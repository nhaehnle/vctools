// SPDX-License-Identifier: GPL-3.0-or-later

use std::{borrow::Cow, cmp::Ordering, collections::HashMap};

use itertools::{FoldWhile, Itertools};

use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::Block,
};

use crate::{
    event::*,
    layout::{Constraint1D, LayoutCache, LayoutEngine, LayoutItem1D},
    state::Builder,
    table::TableSource,
};

trait SourceExtras {
    /// Iterate over all ancestors, including the item itself, towards the root.
    /// The root itself is not produced by the iterator.
    fn ancestor_ids(&self, item_id: u64) -> impl Iterator<Item = u64> + '_;

    /// Like `ancestor_ids`, but does not include the item itself.
    fn strict_ancestor_ids(&self, item_id: u64) -> impl Iterator<Item = u64> + '_;

    /// Return true if `ancestor_id` is an ancestor of (and *not* equal to) `descendant_id`.
    fn is_strict_ancestor(&self, ancestor_id: u64, descendant_id: u64) -> bool;

    /// Return the nearest common ancestor.
    ///
    /// For each of LHS and RHS, also return the child ID of that ancestor on
    /// the path to the given argument ID, or None if the ancestor is equal to
    /// that argument ID.
    fn nearest_common_ancestor(&self, lhs: u64, rhs: u64) -> (u64, Option<u64>, Option<u64>);
}
impl<T: TableSource + ?Sized> SourceExtras for T {
    fn ancestor_ids<'a>(&'a self, item_id: u64) -> impl Iterator<Item = u64> + 'a {
        std::iter::successors(Some(item_id), |&item_id| {
            let parent_id = self.parent_id(item_id);
            if parent_id == 0 {
                None
            } else {
                Some(parent_id)
            }
        })
    }

    fn strict_ancestor_ids<'a>(&'a self, item_id: u64) -> impl Iterator<Item = u64> + 'a {
        self.ancestor_ids(item_id).skip(1)
    }

    fn is_strict_ancestor(&self, ancestor_id: u64, descendant_id: u64) -> bool {
        self.strict_ancestor_ids(descendant_id)
            .any(|id| id == ancestor_id)
    }

    fn nearest_common_ancestor(
        &self,
        mut lhs: u64,
        mut rhs: u64,
    ) -> (u64, Option<u64>, Option<u64>) {
        if lhs == rhs {
            return (lhs, None, None);
        }

        // Collect paths towards the root from both LHS and RHS.
        //
        // We don't know the relative depths of LHS and RHS. By advancing both
        // paths at the same time, we can guarantee that the number of parent
        // queries is at most twice the distance the maximum distance to the
        // nearest common ancestor.
        let mut lhs_path = vec![lhs];
        let mut rhs_path = vec![rhs];
        while lhs != 0 && rhs != 0 {
            let lhs_parent = self.parent_id(lhs);
            if let Some(rhs_idx) = rhs_path.iter().position(|&id| id == lhs_parent) {
                return (
                    lhs_parent,
                    Some(lhs),
                    rhs_idx.checked_sub(1).map(|idx| rhs_path[idx]),
                );
            }
            lhs_path.push(lhs_parent);
            lhs = lhs_parent;

            let rhs_parent = self.parent_id(rhs);
            if let Some(lhs_idx) = lhs_path.iter().position(|&id| id == rhs_parent) {
                return (
                    rhs_parent,
                    lhs_idx.checked_sub(1).map(|idx| lhs_path[idx]),
                    Some(rhs),
                );
            }
            rhs_path.push(rhs_parent);
            rhs = rhs_parent;
        }

        while lhs != 0 {
            let lhs_parent = self.parent_id(lhs);
            if let Some(rhs_idx) = rhs_path.iter().position(|&id| id == lhs_parent) {
                return (
                    lhs_parent,
                    Some(lhs),
                    rhs_idx.checked_sub(1).map(|idx| rhs_path[idx]),
                );
            }
            lhs_path.push(lhs_parent);
            lhs = lhs_parent;
        }

        while rhs != 0 {
            let rhs_parent = self.parent_id(rhs);
            if let Some(lhs_idx) = lhs_path.iter().position(|&id| id == rhs_parent) {
                return (
                    rhs_parent,
                    lhs_idx.checked_sub(1).map(|idx| lhs_path[idx]),
                    Some(rhs),
                );
            }
            rhs_path.push(rhs_parent);
            rhs = rhs_parent;
        }

        // We can't reach here because lhs_path ends up containing 0, and so
        // the final loop over rhs ancenstors should find that entry when it also
        // reaches 0.
        unreachable!();
    }
}

#[derive(Debug, Default)]
pub struct TableState {
    /// Path to the top row from the virtual root (including the top row).
    top_row_path: Vec<u64>,

    /// Lines on screen, (depth, item_id).
    screen: Vec<(usize, u64)>,

    selection: Option<u64>,

    /// Items that were explicitly (un-)collapsed by the user.
    collapsed: HashMap<u64, bool>,

    default_collapsed: bool,

    column_cache: LayoutCache<usize>,
}

struct LiveState<'a> {
    source: &'a dyn TableSource,
    state: &'a mut TableState,

    /// Height of the widget area. The actual screen in state.screen may be shorter.
    height: usize,

    /// Computed (x, width) for each column.
    column_extents: Vec<(u16, u16)>,
}
impl<'a> LiveState<'a> {
    fn new<'b>(
        state: &'a mut TableState,
        width: u16,
        height: usize,
        source: &'a dyn TableSource,
        columns: &[Column<'b>],
    ) -> Self {
        // Update columns.
        let column_extents = {
            let old_cache = std::mem::take(&mut state.column_cache);
            let mut layout = LayoutEngine::<usize>::new();

            let items = columns
                .iter()
                .map(|column| (Some(column.source_id), column.constraint));
            Itertools::intersperse(items, (None, Constraint1D::new_fixed(1))).for_each(
                |(source_id, constraint)| {
                    let mut item = LayoutItem1D::new(constraint);
                    if let Some(source_id) = source_id {
                        item = item.id(source_id, true);
                    }
                    layout.add(&old_cache, source_id, item);
                },
            );

            layout.finish(Constraint1D::new_fixed(width), &mut state.column_cache);
            state.column_cache.save_persistent(old_cache, |id| id);

            columns
                .iter()
                .map(|column| state.column_cache.get(&column.source_id).unwrap())
                .scan(0, |x, width| {
                    let result = (*x, width);
                    *x += width + 1;
                    Some(result)
                })
                .collect()
        };

        let mut live = LiveState {
            source,
            state,
            height,
            column_extents,
        };

        // Preserve collapsed state and selection.
        live.state.collapsed.retain(|id, _| source.exists(*id));

        if let Some(selection_id) = live.state.selection {
            if !source.exists(selection_id) {
                // Try to preserve a selection near to where we were last frame
                if let Some(y) = live.screen_pos(selection_id) {
                    let depth = live.state.screen[y].0;
                    live.state.selection =
                        live.state.screen[(y+1)..].iter()
                            // Prefer to select a later item, but only at the same depth
                            .map_while(|&(d, id)| if d == depth { Some(id) } else { None })
                            .find(|id| source.exists(*id))
                            .or_else(|| {
                                // Fall back to an earlier item at equal or lesser depth
                                live.state.screen[..y].iter()
                                    .rev()
                                    .scan(depth, |depth, &(d, id)| {
                                        if d <= *depth {
                                            *depth = d;
                                            Some(Some(id))
                                        } else {
                                            Some(None)
                                        }
                                    })
                                    .flatten()
                                    .find(|id| source.exists(*id))
                            });
                } else {
                    live.state.selection = None;
                }
            }
        }

        let initial_selection_y = live.state.selection.and_then(|id| live.screen_pos(id));
        let mut selection_y = initial_selection_y;

        if let Some(selection) = live.state.selection {
            for ancestor_id in live.source.strict_ancestor_ids(selection) {
                if live.is_collapsed(ancestor_id) {
                    live.state.selection = Some(ancestor_id);
                    selection_y = live
                        .screen_pos(ancestor_id)
                        .or_else(|| initial_selection_y.map(|y| y.saturating_sub(1)));
                }
            }
        }

        let was_empty = live.state.screen.is_empty();

        // Determine what we want our scroll position to be.
        let Some((target_id, target_y)) =
            // First, try to keep the selection on the same row if it is on screen.
            live.state.selection.and_then(|id| {
                selection_y.map(|y| (id, y))
            })
            // Next, try to keep the top-most remaining row stable.
            .or_else(|| {
                live.state.screen.iter()
                    .enumerate()
                    .find(|(_, (_, item_id))| source.exists(*item_id))
                    .map(|(y, (_, item_id))| (*item_id, y))
            })
            // Everything we previously had on-screen disappeared.
            // Try to put the nearest surviving ancestor near the top.
            .or_else(|| {
                live.state.top_row_path.iter()
                    .rev()
                    .find(|id| source.exists(**id))
                    .map(|id| (*id, height / 4))
            })
            // No ancestor remains, just reset to the top.
            .or_else(|| {
                if source.num_children(0) > 0 {
                    Some((source.child_id(0, 0), 0))
                } else {
                    None
                }
            })
        else {
            // The table is entirely empty.
            live.state.top_row_path.clear();
            live.state.screen.clear();
            debug_assert!(live.state.selection.is_none());
            debug_assert!(live.state.collapsed.is_empty());
            return live;
        };

        live.update_screen(target_id, target_y);

        if was_empty && !live.state.screen.is_empty() {
            live.state.selection = Some(live.state.screen[0].1);
        }

        live
    }

    fn update_screen(&mut self, target_id: u64, target_y: usize) {
        let top_row_id = {
            let mut current_id = target_id;
            for _ in 0..target_y {
                if let Some((prev_id, _)) = self.prev_item(current_id) {
                    current_id = prev_id;
                } else {
                    break;
                }
            }
            current_id
        };

        let depth: usize = {
            let mut current_id = self.source.parent_id(top_row_id);
            let mut depth = 0;
            while current_id != 0 {
                current_id = self.source.parent_id(current_id);
                depth += 1;
            }
            depth
        };

        // Fill the screen from the targetted top row.
        let mut screen = Vec::with_capacity(self.height);
        screen.push((depth, top_row_id));
        for _ in 1..self.height {
            let (depth, item_id) = screen.last().copied().unwrap();
            let Some((next_id, relative_depth)) = self.next_item(item_id) else {
                break;
            };
            screen.push((depth.checked_add_signed(relative_depth).unwrap(), next_id));
        }

        // If we didn't fill the entire screen, attempt to correct that by going backwards.
        let slack = self.height - screen.len();
        let mut top = std::iter::repeat_n((), slack)
            .scan((depth, top_row_id), |state, _| {
                if let Some((prev_id, relative_depth)) = self.prev_item(state.1) {
                    state.0 = state.0.checked_add_signed(relative_depth).unwrap();
                    state.1 = prev_id;
                    Some(*state)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        top.reverse();
        screen.splice(0..0, top);

        self.state.screen = screen;

        self.state.top_row_path.clear();
        if let Some((_, item_id)) = self.state.screen.first() {
            let mut current_id = *item_id;
            while current_id != 0 {
                self.state.top_row_path.push(current_id);
                current_id = self.source.parent_id(current_id);
            }
        }
        self.state.top_row_path.reverse();
    }

    fn screen_pos(&self, item_id: u64) -> Option<usize> {
        self.state.screen.iter().position(|(_, id)| *id == item_id)
    }

    fn is_collapsed(&self, item_id: u64) -> bool {
        self.state
            .collapsed
            .get(&item_id)
            .copied()
            .unwrap_or(self.state.default_collapsed)
    }

    /// Find the nearest (non-strict) ancestor of the given item that is potentially visible
    /// on screen (i.e. not hidden due to a collapsed ancestor). May return `item_id`.
    fn nearest_visible_ancestor(&self, item_id: u64) -> u64 {
        self.source
            .strict_ancestor_ids(item_id)
            .fold(item_id, |current_id, ancestor_id| {
                if self.is_collapsed(ancestor_id) {
                    ancestor_id
                } else {
                    current_id
                }
            })
    }

    /// Find the last descendant of the given item, or the item itself if it has no children.
    /// Returns (id, relative_depth)
    fn last_descendant(&self, item_id: u64) -> (u64, isize) {
        let mut current = item_id;
        let mut depth = 0;
        loop {
            if self.is_collapsed(current) {
                return (current, depth);
            }
            let num_children = self.source.num_children(current);
            if num_children == 0 {
                return (current, depth);
            }
            current = self.source.child_id(current, num_children - 1);
            depth += 1;
        }
    }

    /// Returns Some((item_id, relative_depth)) if there is a previous item.
    fn prev_item(&self, item_id: u64) -> Option<(u64, isize)> {
        let parent_id = self.source.parent_id(item_id);
        let child_idx = self.source.child_idx(item_id);
        if child_idx > 0 {
            let prev_sibling = self.source.child_id(parent_id, child_idx - 1);
            Some(self.last_descendant(prev_sibling))
        } else if parent_id != 0 {
            Some((parent_id, -1))
        } else {
            None
        }
    }

    /// Returns Some((item_id, relative_depth)) if there is a next item.
    fn next_item(&self, item_id: u64) -> Option<(u64, isize)> {
        if !self.is_collapsed(item_id) && self.source.num_children(item_id) > 0 {
            Some((self.source.child_id(item_id, 0), 1))
        } else {
            let mut current = item_id;
            let mut depth = 0;
            loop {
                let parent_id = self.source.parent_id(current);
                let child_idx = self.source.child_idx(current);
                let num_children = self.source.num_children(parent_id);
                if child_idx + 1 < num_children {
                    break Some((self.source.child_id(parent_id, child_idx + 1), depth));
                }
                if parent_id == 0 {
                    break None;
                }
                current = parent_id;
                depth -= 1;
            }
        }
    }

    fn set_collapsed(&mut self, item_id: u64, collapsed: bool) {
        let was_collapsed = self.state.collapsed.insert(item_id, collapsed);
        if let Some(was_collapsed) = was_collapsed {
            if was_collapsed == collapsed {
                return; // No change.
            }
        }

        if collapsed {
            let top_row_id = self.state.screen[0].1;
            if self.source.is_strict_ancestor(item_id, top_row_id) {
                // The collapsed item is not on screen, but it's an ancestor of
                // the top row. The collapsed item should be at the top of the
                // new screen.
                self.update_screen(item_id, 0);
            } else {
                // Leave the top row in place and rebuild the screen.
                //
                // We could first check if the collapsed item is even on screen,
                // and skip the rebuild if it isn't. However, in practice this
                // function really should only be called for items on screen.
                self.update_screen(top_row_id, 0);
            }
        } else {
            // If the expanded item is currently on screen, update the screen to
            // include its children.
            if let Some(_) = self.screen_pos(item_id) {
                self.update_screen(self.state.screen[0].1, 0);
            }
        }
    }

    fn scroll_into_view(&mut self, item_id: u64) {
        // Can only use this function if the item can actually be shown on the screen.
        assert!(self.nearest_visible_ancestor(item_id) == item_id);

        let margin = std::cmp::min(4, self.height / 3);

        let ordering = 'ordering: {
            if let Some(y) = self.screen_pos(item_id) {
                // Item is already on screen, just check whether it's in the margins.
                if y < margin {
                    Ordering::Less
                } else if y > self.height - margin - 1 {
                    Ordering::Greater
                } else {
                    Ordering::Equal
                }
            } else {
                // Item is off-screen. Check whether it is above or below.
                let (screen_ancestor, screen_top_prev, screen_bot_prev) =
                    self.source.nearest_common_ancestor(
                        self.state.screen.first().unwrap().1,
                        self.state.screen.last().unwrap().1,
                    );

                let (item_ancestor, item_prev, screen_prev) = self
                    .source
                    .nearest_common_ancestor(item_id, screen_ancestor);

                let Some(item_prev) = item_prev else {
                    // The chosen item is an ancestor of what's on screen.
                    debug_assert!(item_ancestor == item_id);
                    break 'ordering Ordering::Less;
                };

                if let Some(screen_prev) = screen_prev {
                    // item_prev is a sibling of screen_prev, and the entire
                    // screen is descendant of screen_prev.
                    debug_assert!(item_prev != screen_prev);
                    break 'ordering Ord::cmp(
                        &self.source.child_idx(item_prev),
                        &self.source.child_idx(screen_prev),
                    );
                }
                debug_assert!(item_ancestor == screen_ancestor);

                let Some(screen_top_prev) = screen_top_prev else {
                    // The top row of the screen is an ancestor of the item.
                    debug_assert!(item_ancestor == self.state.screen[0].1);
                    break 'ordering Ordering::Greater;
                };

                let screen_bot_prev = screen_bot_prev.unwrap();

                // item_prev, screen_top_prev, screen_bot_prev all have the
                // same parent.
                let item_child_idx = self.source.child_idx(item_prev);
                let screen_top_child_idx = self.source.child_idx(screen_top_prev);
                let screen_bot_child_idx = self.source.child_idx(screen_bot_prev);
                debug_assert!(screen_top_child_idx < screen_bot_child_idx);

                if item_child_idx <= screen_top_child_idx {
                    Ordering::Less
                } else {
                    debug_assert!(item_child_idx >= screen_bot_child_idx);
                    Ordering::Greater
                }
            }
        };

        match ordering {
            Ordering::Less => {
                self.update_screen(item_id, margin);
            }
            Ordering::Greater => {
                self.update_screen(item_id, self.height - margin - 1);
            }
            Ordering::Equal => {}
        }
    }

    fn scroll_by(&mut self, delta: isize) {
        if self.state.screen.is_empty() {
            return;
        }

        if delta > 0 {
            let new_top_row = std::iter::repeat_n((), delta as usize)
                .fold_while(self.state.screen[0].1, |current_id, _| {
                    if let Some((next_id, _)) = self.next_item(current_id) {
                        FoldWhile::Continue(next_id)
                    } else {
                        FoldWhile::Done(current_id)
                    }
                })
                .into_inner();
            self.update_screen(new_top_row, 0);
        } else if delta < 0 {
            let new_top_row = std::iter::repeat_n((), (-delta) as usize)
                .fold_while(self.state.screen[0].1, |current_id, _| {
                    if let Some((prev_id, _)) = self.prev_item(current_id) {
                        FoldWhile::Continue(prev_id)
                    } else {
                        FoldWhile::Done(current_id)
                    }
                })
                .into_inner();
            self.update_screen(new_top_row, 0);
        }
    }

    fn move_by(&mut self, delta: isize) {
        if self.state.screen.is_empty() {
            return;
        }

        if delta > 0 {
            for _ in 0..delta {
                let selection = self
                    .state
                    .selection
                    .unwrap_or_else(|| self.source.child_id(0, 0));
                if let Some((next_id, _)) = self.next_item(selection) {
                    self.state.selection = Some(next_id);
                } else {
                    break;
                }
            }
        } else if delta < 0 {
            for _ in 0..(-delta) {
                let selection = self
                    .state
                    .selection
                    .unwrap_or_else(|| self.last_descendant(0).0);
                if let Some((prev_id, _)) = self.prev_item(selection) {
                    self.state.selection = Some(prev_id);
                } else {
                    break;
                }
            }
        }

        self.scroll_into_view(self.state.selection.unwrap());
    }

    fn move_to_no_scroll(&mut self, item_id: u64) {
        self.state.selection = Some(item_id);
    }

    fn move_to(&mut self, item_id: u64) {
        self.state.selection = Some(item_id);
        self.scroll_into_view(item_id);
    }
}

#[derive(Debug)]
pub struct Column<'table> {
    pub source_id: usize,
    pub title: Cow<'table, str>,
    pub constraint: Constraint1D,
}
impl<'table> Column<'table> {
    pub fn new(
        source_id: usize,
        title: impl Into<Cow<'table, str>>,
        constraint: Constraint1D,
    ) -> Self {
        Self {
            source_id,
            title: title.into(),
            constraint,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TableResult {
    pub has_focus: bool,

    /// Item ID of the selected item, if any.
    pub selection: Option<u64>,
}

pub struct Table<'table> {
    source: &'table dyn TableSource,
    state: Option<&'table mut TableState>,
    id: Option<Cow<'table, str>>,
    columns: Vec<Column<'table>>,
    default_collapsed: bool,
    show_headers: bool,
}
impl std::fmt::Debug for Table<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("Table")
            .field("id", &self.id)
            .field("default_collapsed", &self.default_collapsed)
            .field("show_headers", &self.show_headers)
            .finish_non_exhaustive()
    }
}
impl<'table> Table<'table> {
    pub fn new(source: &'table dyn TableSource) -> Self {
        Self {
            source,
            state: None,
            id: None,
            columns: Vec::new(),
            default_collapsed: false,
            show_headers: false,
        }
    }

    pub fn columns(self, columns: Vec<Column<'table>>) -> Self {
        Self { columns, ..self }
    }

    pub fn state(self, state: &'table mut TableState) -> Self {
        Self {
            state: Some(state),
            ..self
        }
    }

    pub fn id(self, id: impl Into<Cow<'table, str>>) -> Self {
        Self {
            id: Some(id.into()),
            ..self
        }
    }

    pub fn default_collapsed(self, default_collapsed: bool) -> Self {
        Self {
            default_collapsed,
            ..self
        }
    }

    pub fn show_headers(self, show_headers: bool) -> Self {
        Self {
            show_headers,
            ..self
        }
    }

    pub fn build(mut self, builder: &mut Builder) -> TableResult {
        let state_id = builder.add_state_id(self.id.unwrap_or("table".into()));
        let state = self.state.unwrap_or_else(|| builder.get_state(state_id));

        let area =
            builder.take_lines(LayoutItem1D::new(Constraint1D::new_min(5)).id(state_id, true));
        let has_focus = builder.check_focus(state_id);

        if self.columns.is_empty() {
            self.columns
                .push(Column::new(0, "", Constraint1D::new_min(5)));
        }

        let header_height = std::cmp::min(if self.show_headers { 1 } else { 0 }, area.height);
        let body_height = area.height.saturating_sub(header_height);

        let header_area = Rect {
            height: header_height,
            ..area
        };
        let body_area = Rect {
            height: body_height,
            y: area.y.saturating_add(header_height),
            ..area
        };

        state.default_collapsed = self.default_collapsed;

        let mut live = LiveState::new(
            state,
            header_area.width,
            body_height as usize,
            self.source,
            &self.columns,
        );

        let page_size = std::cmp::max(
            (body_area.height / 2) as isize + 1,
            body_area.height as isize - 5,
        );
        let mouse_page_size = std::cmp::min(5, page_size);

        if has_focus {
            if builder.on_key_press(KeyCode::Left) {
                if let Some(selection) = live.state.selection {
                    if !live.is_collapsed(selection) && live.source.num_children(selection) != 0 {
                        live.set_collapsed(selection, true);
                    } else {
                        let parent = live.source.parent_id(selection);
                        if parent != 0 {
                            live.move_to(parent);
                        }
                    }
                }
            }
            if builder.on_key_press(KeyCode::Right) {
                if let Some(selection) = live.state.selection {
                    if live.is_collapsed(selection) && live.source.num_children(selection) != 0 {
                        live.set_collapsed(selection, false);
                    }
                }
            }
            if builder.on_key_press(KeyCode::Down) {
                live.move_by(1);
            }
            if builder.on_key_press(KeyCode::Up) {
                live.move_by(-1);
            }
            if builder.on_key_press(KeyCode::Home) {
                if !live.state.screen.is_empty() {
                    live.move_to(live.source.child_id(0, 0));
                }
            }
            if builder.on_key_press(KeyCode::End) {
                if !live.state.screen.is_empty() {
                    live.move_to(live.last_descendant(0).0);
                }
            }
            if builder.on_key_press(KeyCode::PageDown) {
                live.move_by(page_size);
            }
            if builder.on_key_press(KeyCode::PageUp) {
                live.move_by(-page_size);
            }
        }

        if let Some(point) = builder.on_mouse_press(body_area, MouseButton::Left) {
            let rx = point.x.saturating_sub(body_area.x) as usize;
            let ry = point.y.saturating_sub(body_area.y) as usize;
            if ry < live.state.screen.len() {
                let (depth, item_id) = live.state.screen[ry];
                live.move_to_no_scroll(item_id);

                if rx == 2 * depth && live.source.num_children(item_id) > 0 {
                    let was_collapsed = live.is_collapsed(item_id);
                    live.set_collapsed(item_id, !was_collapsed);
                }
            }
            builder.grab_focus(state_id);
        }
        if builder.on_mouse_scroll_up(body_area).is_some() {
            live.scroll_by(-mouse_page_size);
        }
        if builder.on_mouse_scroll_down(body_area).is_some() {
            live.scroll_by(mouse_page_size);
        }

        // Render the widget.
        if header_height != 0 {
            let block = Block::new().style(builder.theme().modal_background);
            builder.frame().render_widget(block, header_area);

            let header_style = builder.theme().text(builder.theme_context()).header0;

            for (idx, column) in self.columns.iter_mut().enumerate() {
                let (x, width) = live.column_extents[column.source_id];
                let column_area = Rect {
                    x: header_area.x + x,
                    width,
                    ..header_area
                };
                let span = Span::styled(
                    std::mem::replace(&mut column.title, "".into()),
                    header_style,
                );
                builder.frame().render_widget(span, column_area);

                if idx != 0 {
                    let bar_area = Rect {
                        x: column_area.x.saturating_sub(1),
                        width: 1,
                        ..column_area
                    };
                    builder
                        .frame()
                        .render_widget(Span::from("│").style(header_style), bar_area);
                }
            }
        }

        let block = Block::new().style(
            builder
                .theme()
                .pane_background
                .patch(builder.theme().text(builder.theme_context()).normal),
        );
        builder.frame().render_widget(block, body_area);

        for (ry, (depth, item_id)) in live.state.screen.iter().copied().enumerate() {
            let indent = (depth * 2) as u16;
            let line_area = Rect {
                y: body_area.y + ry as u16,
                height: 1,
                ..body_area
            };

            let selected = live.state.selection == Some(item_id);
            if selected {
                let block =
                    Block::default().style(builder.theme().text(builder.theme_context()).selected);
                builder.frame().render_widget(block, line_area);
            }

            let base_style = if has_focus && selected {
                builder.theme().text(builder.theme_context()).highlight
            } else {
                builder.theme().text(builder.theme_context()).normal
            };

            for (idx, column) in self.columns.iter().enumerate() {
                let column_area = Rect {
                    x: body_area
                        .x
                        .saturating_add(live.column_extents[column.source_id].0),
                    width: live.column_extents[column.source_id].1,
                    ..line_area
                };

                let item_area = if idx != 0 {
                    column_area
                } else {
                    let column_area = Rect {
                        x: column_area.x.saturating_add(indent),
                        width: column_area.width.saturating_sub(indent),
                        ..column_area
                    };

                    if self.source.num_children(item_id) != 0 {
                        // Render the folding range marker.
                        let marker = match live.is_collapsed(item_id) {
                            true => "▶",
                            false => "▼",
                        };
                        let span = Span::from(marker).style(base_style);
                        builder.frame().render_widget(span, column_area);
                    }

                    Rect {
                        x: column_area.x.saturating_add(2),
                        width: column_area.width.saturating_sub(2),
                        ..column_area
                    }
                };

                let spans = self.source.get_data(item_id, column.source_id);
                let line = Line::from(spans).style(base_style);
                builder.frame().render_widget(line, item_area);
            }
        }

        TableResult {
            has_focus,
            selection: live.state.selection,
        }
    }
}
