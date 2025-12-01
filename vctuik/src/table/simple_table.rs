// SPDX-License-Identifier: GPL-3.0-or-later

///! A convenient table source for simple use cases that are not too large.
///!
///! Using this table source involves setting up a persistent `SourceState`
///! object that lives across a frame.
///!
///! Each frame, the `build` method is used to build a `Source` for that frame.
///! The contents of the table are fully re-built each frame, but the state
///! persisted in `SourceState` is used to ensure that items with corresponding
///! keys are treated as stable across frames.
use std::{
    borrow::Cow,
    collections::{hash_map::Entry, HashMap},
    hash::Hash,
    ops::Range,
};

use ratatui::{style::Style, text::Span};

use super::TableSource;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StyleId(u32);

/// Persistent state for a table source that uses the given key type to identify
/// items.
#[derive(Debug, Default)]
pub struct SourceState<K> {
    // (internal parent_id, external item id) -> internal item id
    map: HashMap<(u64, K), u64>,
}
impl<K> SourceState<K> {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn build<'state, 'source>(&'state mut self) -> SourceBuilder<'state, 'source, K> {
        let mut old_ids: Vec<_> = self.map.values().cloned().collect();
        old_ids.sort_by(|a, b| a.cmp(b));

        let mut items = HashMap::new();
        items.insert(
            0,
            Item {
                parent_id: 0,
                child_idx: 0,
                children: 0..0,
                data: vec![],
            },
        );
        let builder = GenericSourceBuilder {
            styles: vec![Style::default()],
            items,
            old_ids,
            old_ids_idx: 0,
            next_id: 1,
            child_links: vec![],
        };
        SourceBuilder {
            builder,
            state: self,
            new_map: HashMap::new(),
        }
    }
}

#[derive(Debug)]
struct GenericSourceBuilder<'source> {
    /// List of styles by StyleId.
    styles: Vec<Style>,

    /// Map of all items by internal ID.
    items: HashMap<u64, Item<'source>>,

    /// List of all (non-root) IDs
    old_ids: Vec<u64>,
    old_ids_idx: usize,
    next_id: u64,

    /// List of all child links (parent_id, child_id).
    child_links: Vec<(u64, u64)>,
}
impl<'source> GenericSourceBuilder<'source> {
    fn set_default_style(&mut self, style: Style) {
        self.styles[0] = style;
    }

    fn add_style(&mut self, style: Style) -> StyleId {
        self.styles.push(style);
        StyleId((self.styles.len() - 1) as u32)
    }

    fn add<'builder>(
        &'builder mut self,
        parent: u64,
        id: Option<u64>,
    ) -> ItemBuilder<'builder, 'source> {
        assert!(self.items.contains_key(&parent));

        let id = id.unwrap_or_else(|| {
            // We aren't re-using an ID from the previous frame, so we need to
            // find a new one. Skip all known IDs from the previous frame,
            // since they may still show up.
            while self.old_ids_idx < self.old_ids.len() {
                assert!(self.next_id <= self.old_ids[self.old_ids_idx]);
                if self.next_id < self.old_ids[self.old_ids_idx] {
                    break;
                }
                self.next_id += 1;
                self.old_ids_idx += 1;
            }

            let id = self.next_id;
            self.next_id += 1;
            id
        });

        let Entry::Vacant(entry) = self.items.entry(id) else {
            // This is an internal logic error.
            panic!("Item with ID {id} already exists");
        };
        let item = entry.insert(Item {
            parent_id: parent,
            child_idx: 0,
            children: 0..0,
            data: vec![],
        });

        self.child_links.push((parent, id));

        ItemBuilder { id, item }
    }

    fn finish(mut self) -> Source<'source> {
        // Compute the child lists for each item.
        //
        // Group the children of parents together, but keep the relative order
        // of children stable per parent.
        self.child_links.sort_by_key(|(parent, _)| *parent);

        let mut children = Vec::with_capacity(self.child_links.len());

        let mut prev_parent_id = 0;
        let mut start = 0;
        for (parent_id, child_id) in self.child_links.into_iter() {
            if prev_parent_id != parent_id {
                self.items.get_mut(&prev_parent_id).unwrap().children = start..children.len();

                prev_parent_id = parent_id;
                start = children.len();
            }

            let child = self.items.get_mut(&child_id).unwrap();
            debug_assert!(child.parent_id == parent_id);
            child.child_idx = children.len() - start;

            children.push(child_id);
        }

        self.items.get_mut(&prev_parent_id).unwrap().children = start..children.len();

        Source {
            styles: self.styles,
            items: self.items,
            children,
        }
    }
}

#[derive(Debug)]
pub struct SourceBuilder<'state, 'source, K> {
    builder: GenericSourceBuilder<'source>,
    state: &'state mut SourceState<K>,
    new_map: HashMap<(u64, K), u64>,
}

impl<'state, 'source, K> SourceBuilder<'state, 'source, K> {
    /// Set the default style to be used by table items.
    pub fn set_default_style(&mut self, style: Style) {
        self.builder.set_default_style(style);
    }

    /// Add a style that can be cheaply referenced by items.
    pub fn add_style(&mut self, style: Style) -> StyleId {
        self.builder.add_style(style)
    }

    /// Finish building the table source and return it.
    pub fn finish(self) -> impl TableSource + 'source {
        self.state.map = self.new_map;
        self.builder.finish()
    }
}
impl<'state, 'source, K: Eq + Hash> SourceBuilder<'state, 'source, K> {
    /// Add a new child to the parent with the given ID.
    ///
    /// ID 0 is always the virtual root, so use 0 to add top-level items.
    ///
    /// If a `key` is given, and there was an item with the same key under the
    /// equivalent parent in the previous frame, then this new item is considered
    /// to be equivalent as well.
    pub fn add<'builder>(
        &'builder mut self,
        parent: u64,
        key: K,
    ) -> ItemBuilder<'builder, 'source> {
        let key = (parent, key);
        let id = self.state.map.get(&key).cloned();

        let builder = self.builder.add(parent, id);

        let is_new = self.new_map.insert(key, builder.id).is_none();
        assert!(
            is_new,
            "Item with the same key already exists under the same parent"
        );

        builder
    }
}

#[derive(Debug)]
struct Item<'widget> {
    parent_id: u64,
    child_idx: usize,
    children: Range<usize>,
    data: Vec<(StyleId, Cow<'widget, str>)>,
}

#[derive(Debug)]
pub struct ItemBuilder<'builder, 'source> {
    id: u64,
    item: &'builder mut Item<'source>,
}
impl<'builder, 'source> ItemBuilder<'builder, 'source> {
    pub fn raw_impl(self, column_idx: usize, text: Cow<'source, str>) -> Self {
        if column_idx >= self.item.data.len() {
            self.item
                .data
                .resize(column_idx + 1, (StyleId(0), "".into()));
        }
        self.item.data[column_idx] = (StyleId(0), text);
        self
    }

    pub fn raw(self, column_idx: usize, text: impl Into<Cow<'source, str>>) -> Self {
        self.raw_impl(column_idx, text.into())
    }

    pub fn styled_impl(self, column_idx: usize, text: Cow<'source, str>, style: StyleId) -> Self {
        if column_idx >= self.item.data.len() {
            self.item
                .data
                .resize(column_idx + 1, (StyleId(0), "".into()));
        }
        self.item.data[column_idx] = (style, text);
        self
    }

    pub fn styled(
        self,
        column_idx: usize,
        text: impl Into<Cow<'source, str>>,
        style: StyleId,
    ) -> Self {
        self.styled_impl(column_idx, text.into(), style)
    }

    pub fn id(self) -> u64 {
        self.id
    }
}

#[derive(Debug)]
struct Source<'source> {
    styles: Vec<Style>,
    items: HashMap<u64, Item<'source>>,
    children: Vec<u64>,
}
impl TableSource for Source<'_> {
    fn exists(&self, item_id: u64) -> bool {
        self.items.contains_key(&item_id)
    }

    fn num_children(&self, item_id: u64) -> usize {
        self.items.get(&item_id).unwrap().children.len()
    }

    fn child_id(&self, item_id: u64, child_idx: usize) -> u64 {
        let range = self.items.get(&item_id).unwrap().children.clone();
        assert!(child_idx < range.len());
        self.children[range.start + child_idx]
    }

    fn parent_id(&self, item_id: u64) -> u64 {
        self.items.get(&item_id).unwrap().parent_id
    }

    fn child_idx(&self, item_id: u64) -> usize {
        self.items.get(&item_id).unwrap().child_idx
    }

    fn get_data(&self, item_id: u64, column_idx: usize) -> Vec<Span<'_>> {
        let item = self.items.get(&item_id).unwrap();
        let (style_id, text) = item
            .data
            .get(column_idx)
            .map(|(style_id, text)| (style_id.0, text.as_ref()))
            .unwrap_or((0, ""));
        vec![Span::styled(text, self.styles[style_id as usize])]
    }
}
