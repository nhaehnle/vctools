// SPDX-License-Identifier: GPL-3.0-or-later

use std::hash::Hash;
use std::{collections::HashMap, ops::Range};

pub type Layout1D = Vec<u16>;

#[derive(Debug, Clone, Copy)]
pub struct Constraint1D {
    min: u16,
    max: u16,
}
impl Default for Constraint1D {
    fn default() -> Self {
        Self {
            min: 0,
            max: u16::MAX,
        }
    }
}
impl Constraint1D {
    pub fn unconstrained() -> Self {
        Self::default()
    }

    pub fn new(min: u16, max: u16) -> Self {
        assert!(min <= max);
        Self { min, max }
    }

    pub fn new_min(min: u16) -> Self {
        Self { min, max: u16::MAX }
    }

    pub fn new_fixed(size: u16) -> Self {
        Self {
            min: size,
            max: size,
        }
    }

    pub fn min(self) -> u16 {
        self.min
    }

    pub fn max(self) -> u16 {
        self.max
    }
}

#[derive(Debug, Clone)]
pub struct LayoutItem1D<Id> {
    pub(crate) id: Option<Id>,
    pub(crate) persistent_id: bool,
    pub(crate) constraint: Constraint1D,
}
impl<Id> LayoutItem1D<Id> {
    pub fn new(constraint: Constraint1D) -> Self {
        Self {
            id: None,
            persistent_id: false,
            constraint,
        }
    }

    pub fn id(mut self, id: Id, persistent: bool) -> Self {
        self.id = Some(id);
        self.persistent_id = persistent;
        self
    }
}

#[derive(Debug, Default)]
struct CacheItem {
    size: u16,
    persistent: bool,
}

#[derive(Debug)]
pub struct LayoutCache<Id> {
    items: HashMap<Id, CacheItem>,
}
impl<Id> Default for LayoutCache<Id> {
    fn default() -> Self {
        Self {
            items: HashMap::new(),
        }
    }
}
impl<Id: Eq + Hash> LayoutCache<Id> {
    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn get(&self, id: &Id) -> Option<u16> {
        self.items.get(id).map(|item| item.size)
    }

    pub fn save_persistent<F>(&mut self, prev: LayoutCache<Id>, mut lookup_new_id: F)
    where
        F: FnMut(Id) -> Id,
    {
        for (old_id, item) in prev.items.into_iter() {
            if item.persistent {
                let new_id = lookup_new_id(old_id);
                self.items.entry(new_id).or_insert(item);
            }
        }
    }
}

#[derive(Debug)]
struct Item<Id> {
    item: LayoutItem1D<Id>,
    pos: u16,
    size: u16,
}
impl<Id> Item<Id> {
    pub fn new(item: LayoutItem1D<Id>, pos: u16, size: u16) -> Self {
        Self { item, pos, size }
    }
}

#[derive(Debug)]
pub struct LayoutEngine<Id> {
    items: Vec<Item<Id>>,
    drags: Vec<(usize, i16)>,
    size: u16,
}
impl<Id> Default for LayoutEngine<Id> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            drags: Vec::new(),
            size: 0,
        }
    }
}
impl<Id: Eq + Hash> LayoutEngine<Id> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn position(&self) -> u16 {
        self.size
    }

    pub fn add(
        &mut self,
        cache: &LayoutCache<Id>,
        old_id: Option<Id>,
        item: LayoutItem1D<Id>,
    ) -> (u16, u16) {
        assert!(item.id.is_some() || !item.persistent_id);

        let pos = self.size;
        let size = if item.id.is_some() {
            let cached_size = old_id
                .and_then(|old_id| cache.items.get(&old_id))
                .map(|item| item.size)
                .unwrap_or(0);
            std::cmp::max(
                std::cmp::min(cached_size, item.constraint.max),
                item.constraint.min,
            )
        } else {
            assert!(item.constraint.min == item.constraint.max);
            item.constraint.min
        };

        self.items.push(Item::new(item, pos, size));
        self.size += size;

        (pos, size)
    }

    // Drag the anchor, which we think of as an infinitely thin line *between* rows by the given
    // delta.
    pub fn drag(&mut self, anchor: u16, delta: i16) {
        // There can be 0-sized "slack" items, so handle the case where multiple items have the
        // same position gracefully.
        let idx = if delta > 0 {
            self.items.partition_point(|item| item.pos <= anchor) - 1
        } else {
            self.items.partition_point(|item| item.pos < anchor)
        };
        assert!(idx > 0 && self.items[idx].pos == anchor);
        self.drags.push((idx, delta))
    }

    /// Returns (layout_changed, total_size).
    pub fn finish(mut self, constraint: Constraint1D, cache: &mut LayoutCache<Id>) -> (bool, u16) {
        // Apply drags
        self.drags.sort();

        let mut calc = Calculator1D::new(self.size, constraint, &mut self.items);

        for (item_idx, delta) in self.drags {
            calc.move_start(item_idx, delta);
        }

        // Apply global constraint
        if calc.total < constraint.min {
            let mut delta = (constraint.min - calc.total) as i16;
            for i in (0..calc.items.len()).rev() {
                if delta == 0 {
                    break;
                }

                delta -= calc.do_grow(i, delta, 0..0);
            }
        } else if calc.total > constraint.max {
            let mut delta = (calc.total - constraint.max) as i16;
            for i in (0..calc.items.len()).rev() {
                if delta == 0 {
                    break;
                }

                delta -= calc.do_shrink(i, delta, 0..0);
            }
        }

        // Build new layout cache
        let result = (calc.changed, calc.total);

        for item in self.items {
            if let Some(id) = item.item.id {
                cache.items.insert(
                    id,
                    CacheItem {
                        size: item.size,
                        persistent: item.item.persistent_id,
                    },
                );
            }
        }

        result
    }
}

#[derive(Debug, Default)]
struct Calculator1D<'items, Id> {
    total: u16,
    constraint: Constraint1D,
    items: &'items mut [Item<Id>],
    changed: bool,
}
impl<'items, Id> Calculator1D<'items, Id> {
    fn new(total: u16, constraint: Constraint1D, items: &'items mut [Item<Id>]) -> Self {
        Self {
            total,
            constraint,
            items,
            changed: false,
        }
    }

    fn move_start(&mut self, index: usize, mut delta: i16) {
        if delta > 0 {
            for i in (0..index).rev() {
                if delta == 0 {
                    break;
                }

                delta -= self.do_grow(i, delta, index..self.items.len());
            }
        } else if delta < 0 {
            for i in (0..index).rev() {
                if delta == 0 {
                    break;
                }

                delta += self.do_shrink(i, -delta, index..self.items.len());
            }
        }
    }

    fn do_grow(&mut self, index: usize, delta: i16, shrink_range: Range<usize>) -> i16 {
        let mut delta = std::cmp::min(
            delta as u16,
            self.items[index].item.constraint.max - self.items[index].size,
        );

        let mut growth = 0;
        let slack = std::cmp::min(self.constraint.max.saturating_sub(self.total), delta);
        self.items[index].size += slack;
        self.total += slack;
        delta -= slack;
        growth += slack;

        for i in shrink_range.rev() {
            if delta == 0 {
                break;
            }

            if i != index {
                let slack = std::cmp::min(
                    self.items[i].size - self.items[i].item.constraint.min,
                    delta,
                );
                self.items[index].size += slack;
                self.items[i].size -= slack;
                delta -= slack;
                growth += slack;
            }
        }

        if growth != 0 {
            self.changed = true;
        }

        growth as i16
    }

    fn do_shrink(&mut self, index: usize, delta: i16, grow_range: Range<usize>) -> i16 {
        let mut delta = std::cmp::min(
            delta as u16,
            self.items[index].size - self.items[index].item.constraint.min,
        );

        let mut shrink = 0;

        for i in grow_range.rev() {
            if delta == 0 {
                break;
            }

            if i != index {
                let slack = std::cmp::min(
                    self.items[i].item.constraint.max - self.items[i].size,
                    delta,
                );
                self.items[index].size -= slack;
                self.items[i].size += slack;
                delta -= slack;
                shrink += slack;
            }
        }

        self.items[index].size -= delta;
        self.total -= delta;
        shrink += delta;

        if shrink != 0 {
            self.changed = true;
        }

        shrink as i16
    }
}

//        // Step 1: Clamp pre-existing sizes to any constraints that may have changed.
//        let mut num_new: u16 = 0;
//        let mut min_new: u16 = 0;
//        for (layout, constraint) in sizes.iter_mut().zip(constraints.iter()) {
//            if let Some(height) = layout {
//                *layout = Some(std::cmp::min(std::cmp::max(*height, constraint.min), constraint.max));
//            } else {
//                num_new += 1;
//                min_new = min_new.saturating_add(constraint.min);
//            }
//        }
//
//        // Step 2: Pre-distribute unused size to new elements.
//        let mut sum_sizes = sizes.iter().copied().map(Option::unwrap_or_default).fold(0, u16::saturating_add);
//        let init_new = if num_new != 0 { total.saturating_sub(sum_sizes) / num_new } else { 0 };
//
//        let mut sizes: Vec<u16> =
//            sizes.into_iter().zip(&constraints)
//                .map(|(size, constraint)| {
//                    size.unwrap_or_else(|| {
//                        let size = std::cmp::min(std::cmp::max(init_new, constraint.min), constraint.max);
//                        sum_sizes = sum_sizes.saturating_add(size);
//                        size
//                    })
//                }).collect();
//
//        // Step 3: Constrain sizes to the total available size if possible.
//        if sum_sizes < total {
//            for i in (0..sizes.len()).rev() {
//                if sum_sizes == total {
//                    break;
//                }
//
//                let slack = std::cmp::min(constraints[i].max - sizes[i], total - sum_sizes);
//                sizes[i] += slack;
//                sum_sizes += slack;
//            }
//        } else {
//            for i in (0..sizes.len()).rev() {
//                if sum_sizes == total {
//                    break;
//                }
//
//                let slack = std::cmp::min(sizes[i] - constraints[i].min, sum_sizes - total);
//                sizes[i] -= slack;
//                sum_sizes -= slack;
//            }
//        }
//
//        Self {
//            constraints,
//            total,
//            height: sum_sizes,
//            sizes,
//        }
//    }
//
//    pub fn layout(&self) -> &[u16] {
//        &self.sizes
//    }
//
//    pub fn morph(&mut self, index: usize, constraint: Constraint1D, mut size: u16) {
//        self.constraints[index] = constraint;
//
//        size = std::cmp::max(std::cmp::min(size, self.constraints[index].max), self.constraints[index].min);
//
//        if size > self.sizes[index] {
//            let delta = size - self.sizes[index];
//            self.do_grow(index, delta, 0..self.sizes.len());
//        } else {
//            let delta = self.sizes[index] - size;
//            self.do_shrink(index, delta, 0..self.sizes.len());
//        }
//    }
//}
//
