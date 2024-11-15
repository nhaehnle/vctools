use std::ops::Range;


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
    pub fn new(min: u16, max: u16) -> Self {
        assert!(min <= max);
        Self { min, max }
    }

    pub fn new_min(min: u16) -> Self {
        Self { min, max: u16::MAX }
    }

    pub fn new_exact(size: u16) -> Self {
        Self { min: size, max: size }
    }

    pub fn min(self) -> u16 {
        self.min
    }

    pub fn max(self) -> u16 {
        self.max
    }
}

#[derive(Debug, Default, Clone)]
pub struct Constrained1D {
    constraints: Vec<Constraint1D>,
    total: u16,
    height: u16,
    sizes: Layout1D,
}
impl Constrained1D {
    pub fn constrain(constraints: Vec<Constraint1D>, total: u16, mut sizes: Vec<Option<u16>>) -> Self {
        assert!(constraints.len() == sizes.len());
        assert!(sizes.len() <= u16::MAX as usize);

        // Step 1: Clamp pre-existing sizes to any constraints that may have changed.
        let mut num_new: u16 = 0;
        let mut min_new: u16 = 0;
        for (layout, constraint) in sizes.iter_mut().zip(constraints.iter()) {
            if let Some(height) = layout {
                *layout = Some(std::cmp::min(std::cmp::max(*height, constraint.min), constraint.max));
            } else {
                num_new += 1;
                min_new = min_new.saturating_add(constraint.min);
            }
        }

        // Step 2: Pre-distribute unused size to new elements.
        let mut sum_sizes = sizes.iter().copied().map(Option::unwrap_or_default).fold(0, u16::saturating_add);
        let init_new = if num_new != 0 { total.saturating_sub(sum_sizes) / num_new } else { 0 };

        let mut sizes: Vec<u16> =
            sizes.into_iter().zip(&constraints)
                .map(|(size, constraint)| {
                    size.unwrap_or_else(|| {
                        let size = std::cmp::min(std::cmp::max(init_new, constraint.min), constraint.max);
                        sum_sizes = sum_sizes.saturating_add(size);
                        size
                    })
                }).collect();

        // Step 3: Constrain sizes to the total available size if possible.
        if sum_sizes < total {
            for i in (0..sizes.len()).rev() {
                if sum_sizes == total {
                    break;
                }

                let slack = std::cmp::min(constraints[i].max - sizes[i], total - sum_sizes);
                sizes[i] += slack;
                sum_sizes += slack;
            }
        } else {
            for i in (0..sizes.len()).rev() {
                if sum_sizes == total {
                    break;
                }

                let slack = std::cmp::min(sizes[i] - constraints[i].min, sum_sizes - total);
                sizes[i] -= slack;
                sum_sizes -= slack;
            }
        }

        Self {
            constraints,
            total,
            height: sum_sizes,
            sizes,
        }
    }

    pub fn layout(&self) -> &[u16] {
        &self.sizes
    }

    pub fn morph(&mut self, index: usize, constraint: Constraint1D, mut size: u16) {
        self.constraints[index] = constraint;

        size = std::cmp::max(std::cmp::min(size, self.constraints[index].max), self.constraints[index].min);

        if size > self.sizes[index] {
            let delta = size - self.sizes[index];
            self.do_grow(index, delta, 0..self.sizes.len());
        } else {
            let delta = self.sizes[index] - size;
            self.do_shrink(index, delta, 0..self.sizes.len());
        }
    }

    pub fn move_start(&mut self, index: usize, new_position: u16) {
        let old_position = self.sizes[0..index].iter().copied().fold(0, u16::saturating_add);
        if new_position > old_position {
            let mut delta = new_position - old_position;
            for i in (0..index).rev() {
                if delta == 0 {
                    break;
                }

                delta -= self.do_grow(i, delta, index..self.sizes.len());
            }
        } else {
            let mut delta = old_position - new_position;
            for i in (0..index).rev() {
                if delta == 0 {
                    break;
                }

                delta -= self.do_shrink(i, delta, index..self.sizes.len());
            }
        }
    }

    fn do_grow(&mut self, index: usize, mut delta: u16, shrink_range: Range<usize>) -> u16 {
        delta = std::cmp::min(delta, self.constraints[index].max - self.sizes[index]);

        let mut growth = 0;
        let slack = std::cmp::min(self.total.saturating_sub(self.height), delta);
        self.sizes[index] += slack;
        self.height += slack;
        delta -= slack;
        growth += slack;

        for i in shrink_range.rev() {
            if delta == 0 {
                break;
            }

            if i != index {
                let slack = std::cmp::min(self.sizes[i] - self.constraints[i].min, delta);
                self.sizes[index] += slack;
                self.sizes[i] -= slack;
                delta -= slack;
                growth += slack;
            }
        }

        growth
    }

    fn do_shrink(&mut self, index: usize, mut delta: u16, grow_range: Range<usize>) -> u16 {
        delta = std::cmp::min(delta, self.sizes[index] - self.constraints[index].min);

        let mut shrink = 0;

        for i in grow_range.rev() {
            if delta == 0 {
                break;
            }

            if i != index {
                let slack = std::cmp::min(self.constraints[i].max - self.sizes[i], delta);
                self.sizes[index] -= slack;
                self.sizes[i] += slack;
                delta -= slack;
                shrink += slack;
            }
        }

        self.sizes[index] -= delta;
        self.height -= delta;
        shrink += delta;

        shrink
    }
}
