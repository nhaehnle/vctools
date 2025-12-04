// SPDX-License-Identifier: MIT

pub trait PartitionPointExt {
    type Item;

    /// Return the first index of a slice for which `pred` is false (or the length of the slice
    /// if `pred` is true for all elements).
    ///
    /// `pred` must be true for a (possibly empty) prefix of the slice and false for the remainder.
    ///
    /// If `forward` is true, we assume that the predicate is true for the element at `hint_idx`.
    /// If `forward` is false, we assume that the predicate is false for the element at `hint_idx`.
    ///
    /// This is *not* just the same as calling `partition_point` on a sub-range. The initial
    /// search is biased towards the hint index. This allows a heuristic speed-up for successive
    /// binary searches into a slice where the searched-for points tend to be close together.
    fn partition_point_with_hint<P>(
        &self,
        hint_idx: usize,
        forward: bool,
        pred: P,
    ) -> usize
    where
        P: Fn(&Self::Item) -> bool;
}
impl<T> PartitionPointExt for [T] {
    type Item = T;

    /// Return the first landmark index for which `pred` is false (or the length of the landmarks vector).
    ///
    /// `pred` must be true for a (possibly empty) prefix of landmarks and false for the remainder.
    ///
    /// If `forward` is true, we assume that the predicate is true for the element at `hint_idx`.
    /// If `forward` is false, we assume that the predicate is false for the element at `hint_idx`.
    fn partition_point_with_hint<P>(&self, mut hint_idx: usize, forward: bool, pred: P) -> usize
    where
        P: Fn(&Self::Item) -> bool,
    {
        // Invariant: left of `begin` is known true, `end` is known false.
        let (mut begin, mut end) = 'pre: {
            // Exponential search from the initial ("hint") index in the given direction.
            if forward {
                // Invariant for forward search: hint_idx is known true.
                let mut step = 1;
                while hint_idx + step < self.len() {
                    if !pred(&self[hint_idx + step]) {
                        break 'pre (hint_idx + 1, hint_idx + step);
                    }

                    hint_idx += step;
                    step *= 2;
                }
                (hint_idx + 1, self.len())
            } else {
                // Invariant for backward search: hint_idx + 1 is known false.
                let mut step = 1;
                while step <= hint_idx + 1 {
                    if pred(&self[hint_idx + 1 - step]) {
                        break 'pre (hint_idx + 2 - step, hint_idx + 1);
                    }

                    hint_idx -= step;
                    step *= 2;
                }
                (0, hint_idx + 1)
            }
        };

        // Binary search within the found range.
        while begin < end {
            let mid = (begin + end) / 2;
            if pred(&self[mid]) {
                begin = mid + 1;
            } else {
                end = mid;
            }
        }

        begin
    }
}
