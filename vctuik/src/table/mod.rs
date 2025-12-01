// SPDX-License-Identifier: GPL-3.0-or-later

//! Tree/table widget and associated infrastructure.

pub mod simple_table;
mod widget;

use ratatui::text::Span;

/// Table source trait.
///
/// The abstract data model has:
///  - columns with simple textual titles
///  - items with globally unique IDs
///   - ID 0 is reserved for the virtual root -- top-level items have parent 0
///  - each item's data is produced as ratatui `Span`s for each column
pub trait TableSource {
    /// Whether an item with the given ID exists.
    fn exists(&self, item_id: u64) -> bool;

    /// Return the number of children of the item with the given ID.
    fn num_children(&self, item_id: u64) -> usize;

    /// Return the ID of the child at the given index of the item with the given ID.
    fn child_id(&self, item_id: u64, child_idx: usize) -> u64;

    /// Return the parent ID of the item with the given ID.
    fn parent_id(&self, item_id: u64) -> u64;

    /// Return the index of the given item amount its parent's children.
    fn child_idx(&self, item_id: u64) -> usize;

    /// Return the data for the given item in the column with the given index.
    fn get_data(&self, item_id: u64, column_idx: usize) -> Vec<Span<'_>>;
}

pub use widget::{Column, Table};
