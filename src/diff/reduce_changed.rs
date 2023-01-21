// SPDX-License-Identifier: MIT

/// Reduce the number of changed lines in a diff by identifying and matching up
/// lines that are common between the old and new sides of the diff. This is
/// expressed in a slightly unusual way, but the core of it is computing a diff
/// from scratch, given sequences of "old" and "new" lines.
///
/// There are multiple algorithms implemented in this module.
///
/// Explicitly stored line numbers are generally 1-based in the core algorithms.
/// This is consistent with the overall [`Diff`] and [`Block`] structure and is
/// useful for handling boundary conditions.
///
/// There are some common observations that the algorithms make use of, starting
/// from the naive dynamic program for minimizing the number of "changed" lines
/// or, equivalently, maximizing the number of "unchanged" lines (which we also
/// tend to call "matched" lines in this context).
///
/// The bipartite _match graph_ has nodes representing old line numbers on the
/// left and nodes representing new line numbers on the right. Edges connect
/// matchable line numbers -- that is, they connect equal lines.
///
/// Computing a diff is equivalent to choosing a set of edges from the match
/// graph, but one cannot just choose any set of edges.
///
/// The edges of the match graph are the nodes of the _conflict graph_. Nodes
/// in the conflict graph are connected if the corresponding match edges "cross"
/// and cannot be chosen in a diff simultaneously.
///
/// Computing a diff is equivalent to choosing an independent set in the
/// conflict graph.
///
/// The _search graph_ is a directed graph on the nodes of the conflict graph,
/// with edges between independent nodes pointing in a strictly forward
/// direction (i.e., towards increasing line numbers). The search graph is
/// augmented with artificial source and sink nodes.
///
/// Computing a diff is equivalent to choosing a source-sink path in the
/// conflict graph.
///
/// Matchable lines can also be represented as points in the plane, with line
/// numbers serving as coordinates.
///
/// A path in the search graph corresponds to a strictly increasing sequence of
/// points. That is, each point is in the strictly positive quadrant rooted at
/// its predecessor.
///
/// The plane can be equipped with a cost function equivalent to the costs
/// in the dynamic programming table.
///
/// See [`DiffAlgorithm`] for more.

use std::collections::{HashMap, BinaryHeap, hash_map::Entry};

use crate::diff::*;

#[derive(Debug, Clone, Copy)]
pub enum DiffAlgorithm {
    /// Dijkstra search in the search graph, restricting outgoing edges to
    /// those forming a Pareto set.
    ///
    /// This algorithm can be superlinear for various reasons.
    GraphSearch,

    /// Sweep-line over the quadrant arrangement created by matchable edges,
    /// with some heuristics to ensure that the worst-case runtime is
    /// log-linear.
    SweepLine,

    /// Sweep-line over the quadrant arrangement created by matchable edges.
    SweepLineExact,
}
impl Default for DiffAlgorithm {
    fn default() -> Self {
        Self::SweepLine
    }
}
impl DiffAlgorithm {
    fn run(self, buffer: &Buffer, old_begin: u32, new_begin: u32,
           old: &[DiffRef], new: &[DiffRef], unimportant: bool) -> Vec<Block> {
        match self {
        Self::GraphSearch => diff_graph_search(buffer, old_begin, new_begin, old, new, unimportant),
        Self::SweepLine => diff_sweep_line(buffer, old_begin, new_begin, old, new, unimportant),
        Self::SweepLineExact => diff_sweep_line_exact(buffer, old_begin, new_begin, old, new, unimportant),
        }
    }
}

#[derive(Debug)]
struct ReverseBlockCollector<'a> {
    old_begin: u32,
    new_begin: u32,
    old: &'a [DiffRef],
    new: &'a [DiffRef],
    unimportant: bool,
    blocks: Vec<Block>,
    unchanged_old_offset: u32,
    unchanged_new_offset: u32,
    unchanged_count: u32,
}
impl<'a> ReverseBlockCollector<'a> {
    fn new(old_begin: u32, new_begin: u32, old: &'a [DiffRef], new: &'a [DiffRef], unimportant: bool) -> Self {
        Self {
            old_begin,
            new_begin,
            old,
            new,
            unimportant,
            blocks: Vec::new(),
            unchanged_old_offset: old.len() as u32,
            unchanged_new_offset: new.len() as u32,
            unchanged_count: 0,
        }
    }

    fn finish(mut self) -> Vec<Block> {
        self.commit_any_unchanged();
        if self.unchanged_old_offset != 0 || self.unchanged_new_offset != 0 {
            self.commit_changed(0, 0);
        }

        self.blocks.reverse();
        self.blocks
    }

    fn add_unchanged(&mut self, old_linenum: u32, new_linenum: u32, count: u32) {
        assert!(old_linenum >= 1);
        assert!(new_linenum >= 1);

        let old_offset = old_linenum - 1;
        let new_offset = new_linenum - 1;

        assert!(old_offset + count <= self.unchanged_old_offset);
        assert!(new_offset + count <= self.unchanged_new_offset);

        if old_offset + count == self.unchanged_old_offset &&
           new_offset + count == self.unchanged_new_offset {
            self.unchanged_old_offset -= count;
            self.unchanged_new_offset -= count;
            self.unchanged_count += count;
            return
        }

        self.commit_any_unchanged();
        self.commit_changed(old_offset + count, new_offset + count);

        self.unchanged_old_offset = old_offset;
        self.unchanged_new_offset = new_offset;
        self.unchanged_count = count;
    }

    fn commit_any_unchanged(&mut self) {
        if self.unchanged_count == 0 {
            return
        }

        let begin = self.unchanged_old_offset as usize;
        let end = begin + self.unchanged_count as usize;
        self.blocks.push(Block {
            old_begin: self.old_begin + self.unchanged_old_offset,
            new_begin: self.new_begin + self.unchanged_new_offset,
            contents: BlockContents::UnchangedKnown(self.old[begin..end].into()),
        });
    }

    fn commit_changed(&mut self, old_offset: u32, new_offset: u32) {
        let old_count = self.unchanged_old_offset - old_offset;
        let new_count = self.unchanged_new_offset - new_offset;
        assert!(old_count != 0 || new_count != 0);

        self.blocks.push(Block {
            old_begin: self.old_begin + old_offset,
            new_begin: self.new_begin + new_offset,
            contents: BlockContents::Changed {
                old: self.old[old_offset as usize..(old_offset + old_count) as usize].into(),
                new: self.new[new_offset as usize..(new_offset + new_count) as usize].into(),
                unimportant: self.unimportant,
            },
        });
    }
}

fn diff_graph_search(buffer: &Buffer, old_begin: u32, new_begin: u32,
                     old: &[DiffRef], new: &[DiffRef], unimportant: bool) -> Vec<Block>
{
    /// A node in the graph of the dynamic program, using 1-based indices into
    /// the lines array. Node(0,0) is the initial state of the search.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    struct Node(u32, u32);

    /// Positions (i.e. lines) where line contents can be found.
    ///
    /// We expect the common case to be that most lines are only found once
    /// and a few lines are found many times, so we try to optimize for that.
    #[derive(Debug, Clone, Copy)]
    enum Positions {
        None,
        One([u32; 1]),
        Many(u32),
    }
    impl Positions {
        fn is_empty(&self) -> bool {
            match self {
            Positions::None => true,
            _ => false,
            }
        }

        fn get<'a>(&'a self, mgr: &'a PositionMgr) -> &'a [u32] {
            match self {
            Positions::None => { &[] },
            Positions::One(pos) => { pos },
            Positions::Many(slot) => { &mgr.many[*slot as usize] },
            }
        }

        fn push(&mut self, mgr: &mut PositionMgr, next: u32) {
            match *self {
            Positions::None => { *self = Positions::One([next]); },
            Positions::One([pos]) => {
                assert!(next > pos);
                let slot = mgr.many.len() as u32;
                mgr.many.push([pos, next].into());
                *self = Positions::Many(slot);
            },
            Positions::Many(slot) => {
                let many = &mut mgr.many[slot as usize];
                assert!(next > *many.last().unwrap());
                many.push(next);
            },
            }
        }
    }

    struct PositionMgr {
        many: Vec<Vec<u32>>,
    }
    impl PositionMgr {
        fn new() -> Self {
            Self {
                many: Vec::new(),
            }
        }
    }

    // Build the bipartite graph.
    let mut position_mgr = PositionMgr::new();
    let mut old_hash: HashMap<&[u8], Positions> = HashMap::new();
    let mut new_hash: HashMap<&[u8], Positions> = HashMap::new();

    old_hash.reserve(old.len());
    new_hash.reserve(new.len());

    for (idx, line) in old.iter().enumerate() {
        old_hash.entry(&buffer[*line])
            .and_modify(|pos| pos.push(&mut position_mgr, idx as u32 + 1))
            .or_insert(Positions::One([idx as u32 + 1]));
    }
    for (idx, line) in new.iter().enumerate() {
        new_hash.entry(&buffer[*line])
            .and_modify(|pos| pos.push(&mut position_mgr, idx as u32 + 1))
            .or_insert(Positions::One([idx as u32 + 1]));
    }

    let old_edges: Vec<Positions> =
        [Positions::One([0])].into_iter()
            .chain(old.iter().map(|&line| *new_hash.get(&buffer[line]).unwrap_or(&Positions::None)))
            .chain([Positions::One([new.len() as u32 + 1])])
            .collect();
    let new_edges: Vec<Positions> =
        [Positions::One([0])].into_iter()
            .chain(new.iter().map(|&line| *old_hash.get(&buffer[line]).unwrap_or(&Positions::None)))
            .chain([Positions::One([old.len() as u32 + 1])])
            .collect();

    let mut old_next: Vec<u32> = Vec::new();
    let mut new_next: Vec<u32> = Vec::new();

    old_next.resize(old_edges.len(), 0);
    new_next.resize(new_edges.len(), 0);

    old_next.iter_mut().zip(old_edges.iter()).enumerate().rfold(
        old_edges.len() as u32 - 1,
        |mut next, (idx, (old_next, old_edges))| {
            *old_next = next;
            if !old_edges.is_empty() {
                next = idx as u32;
            }
            next
        });

    new_next.iter_mut().zip(new_edges.iter()).enumerate().rfold(
        new_edges.len() as u32 - 1,
        |mut next, (idx, (new_next, new_edges))| {
            *new_next = next;
            if !new_edges.is_empty() {
                next = idx as u32;
            }
            next
        });

    // Shortest path search
    //
    // The nodes map contains discovered nodes and their best-found cost and
    // corresponding predecessor.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Pending(Node, u32);
    impl PartialOrd for Pending {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(other.1.cmp(&self.1))
        }
    }
    impl Ord for Pending {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            self.partial_cmp(other).unwrap()
        }
    }
    let mut nodes: HashMap<Node, (u32, Node)> = HashMap::new();
    let mut queue: BinaryHeap<Pending> = BinaryHeap::new();

    nodes.reserve(old.len() + new.len());
    queue.reserve(old.len() + new.len());

    nodes.insert(Node(0, 0), (0, Node(0, 0)));
    queue.push(Pending(Node(0, 0), 0));

    loop {
        let Some(Pending(current, cost)) = queue.pop() else { panic!() };
        if nodes.get(&current).unwrap().0 < cost {
            continue
        }
        if current == Node(old_edges.len() as u32 - 1, new_edges.len() as u32 - 1) {
            break
        }

        let mut visit_edge = |next: Node| {
            let cost = cost + (next.0 - current.0 - 1) + (next.1 - current.1 - 1);
            let mut proceed = true;
            nodes.entry(next)
                .and_modify(|(next_cost, next_prev)| {
                    if cost < *next_cost {
                        *next_cost = cost;
                        *next_prev = current;
                    } else {
                        proceed = false;
                    }
                })
                .or_insert((cost, current));
            if proceed {
                queue.push(Pending(next, cost));
            }

        };

        let mut old_candidate = old_next[current.0 as usize];
        let mut new_candidate = new_next[current.1 as usize];
        let mut old_bound = u32::MAX;
        let mut new_bound = u32::MAX;

        loop {
            // Find and visit the best edge incident to old_candidate, if any.
            let old_edges = old_edges[old_candidate as usize].get(&position_mgr);
            let idx = old_edges.partition_point(|pos| *pos <= current.1);
            if idx < old_edges.len() {
                let new = old_edges[idx];
                assert!(new >= new_candidate);
                if new < new_bound {
                    visit_edge(Node(old_candidate, new));
                    new_bound = new;
                    if new_bound <= new_candidate {
                        break
                    }
                }
            }
            old_candidate = old_next[old_candidate as usize];
            if old_candidate >= old_bound {
                break
            }

            // Find and visit the best edge incident to new_candidate, if any.
            let new_edges = new_edges[new_candidate as usize].get(&position_mgr);
            let idx = new_edges.partition_point(|pos| *pos <= current.0);
            if idx < new_edges.len() {
                let old = new_edges[idx];
                assert!(old >= old_candidate);
                if old < old_bound {
                    visit_edge(Node(old, new_candidate));
                    old_bound = old;
                    if old_bound <= old_candidate {
                        break
                    }
                }
            }
            new_candidate = new_next[new_candidate as usize];
            if new_candidate >= new_bound {
                break
            }
        }
    }

    let mut collect = ReverseBlockCollector::new(old_begin, new_begin, old, new, unimportant);
    let mut current = Node(old_edges.len() as u32 - 1, new_edges.len() as u32 - 1);

    loop {
        let next = nodes.get(&current).unwrap().1;
        if next.0 == 0 {
            break
        }
        collect.add_unchanged(next.0, next.1, 1);
        current = next;
    }

    collect.finish()
}

#[derive(Debug, Clone, Copy)]
struct DiffLine {
    pos_old: (u32, u32), // count, pos_backing idx
    pos_new: (u32, u32), // count, pos_backing idx
}

#[derive(Debug)]
struct PosBacking(Vec<u32>);
impl PosBacking {
    fn insert(&mut self, linenums: &[u32]) -> (u32, u32) {
        let result = (self.0.len() as u32, linenums.len() as u32);
        self.0.extend(linenums);
        result
    }

    fn get(&self, pos: (u32, u32)) -> &[u32] {
        let begin = pos.0 as usize;
        let count = pos.1 as usize;
        &self.0[begin..begin + count]
    }
}

#[derive(Debug)]
struct DiffProblem {
    /// Indices into `(in)frequent_lines`
    old: Vec<i32>,

    /// Indices into `(in)frequent_lines`
    new: Vec<i32>,

    /// Frequent lines
    frequent_lines: Vec<DiffLine>,

    /// Infrequent lines
    infrequent_lines: Vec<DiffLine>,

    /// Backing store for line positions
    pos_backing: PosBacking,

    /// Unique lines that occur strictly more often than this on one side are
    /// considered "frequent".
    cutoff: usize,
}
impl DiffProblem {
    fn new(buffer: &Buffer, old: &[DiffRef], new: &[DiffRef], cutoff: usize) -> Self {
        /// Positions (i.e. lines) where line contents can be found.
        ///
        /// We expect the common case to be that most lines are only found once
        /// and a few lines are found many times, so we try to optimize for that.
        #[derive(Debug)]
        enum SortedSmallVec {
            Empty,
            One([u32; 1]),
            Many(Vec<u32>),
        }
        impl SortedSmallVec {
            fn is_empty(&self) -> bool {
                match self {
                Self::Empty => true,
                _ => false,
                }
            }

            fn len(&self) -> usize {
                match self {
                Self::Empty => 0,
                Self::One(_) => 1,
                Self::Many(vec) => vec.len(),
                }
            }

            fn get(&self) -> &[u32] {
                match self {
                Self::Empty => { &[] },
                Self::One(contents) => { contents },
                Self::Many(contents) => { contents },
                }
            }

            fn push(&mut self, x: u32) {
                match self {
                Self::Empty => { *self = Self::One([x]); },
                Self::One([first]) => {
                    assert!(x > *first);
                    *self = Self::Many([*first, x].into());
                },
                Self::Many(vec) => {
                    assert!(x > *vec.last().unwrap());
                    vec.push(x);
                },
                }
            }
        }

        struct TempLine {
            pos_short: SortedSmallVec,
            pos_long: SortedSmallVec,
            mapping: i32,
        }
        impl TempLine {
            fn new() -> Self {
                Self {
                    pos_short: SortedSmallVec::Empty,
                    pos_long: SortedSmallVec::Empty,
                    mapping: 0,
                }
            }
        }

        if old.len() + new.len() >= i32::MAX as usize {
            panic!("should be prevented by checks in diff::Buffer")
        }

        let mut hash: HashMap<&[u8], usize> = HashMap::new();
        let mut temp_lines: Vec<_> = Vec::new();

        let short;
        let long;
        let short_is_old;
        if old.len() <= new.len() {
            (short, long) = (old, new);
            short_is_old = true;
        } else {
            (short, long) = (new, old);
            short_is_old = false;
        }

        hash.reserve(short.len());
        temp_lines.reserve_exact(short.len() + 1);
        temp_lines.push(TempLine::new());

        let mut backing_size = 0;
        let short: Vec<usize> = short.iter().enumerate().map(|(idx, line)| {
            let (line_idx, line) =
                match hash.entry(&buffer[*line]) {
                Entry::Occupied(entry) => (*entry.get(), &mut temp_lines[*entry.get()]),
                Entry::Vacant(entry) => {
                    temp_lines.push(TempLine::new());
                    (*entry.insert(temp_lines.len() - 1), temp_lines.last_mut().unwrap())
                }
                };

            line.pos_short.push(idx as u32 + 1);
            backing_size += 1;

            line_idx
        }).collect();
        let long: Vec<usize> = long.iter().enumerate().map(|(idx, line)| {
            if let Some(&line_idx) = hash.get(&buffer[*line]) {
                temp_lines[line_idx].pos_long.push(idx as u32 + 1);
                backing_size += 1;

                line_idx
            } else {
                0
            }
        }).collect();

        let mut pos_backing = PosBacking(Vec::with_capacity(backing_size));

        let mut frequent_lines = Vec::new();
        let mut infrequent_lines = Vec::new();
        for line in &mut temp_lines {
            if line.pos_short.is_empty() || line.pos_long.is_empty() {
                continue
            }

            let pos_short = pos_backing.insert(line.pos_short.get());
            let pos_long = pos_backing.insert(line.pos_long.get());

            let diff_line = if short_is_old {
                    DiffLine { pos_old: pos_short, pos_new: pos_long }
                } else {
                    DiffLine { pos_old: pos_long, pos_new: pos_short }
                };

            let line_frequent = line.pos_short.len() > cutoff &&
                                line.pos_long.len() > cutoff;
            if line_frequent {
                frequent_lines.push(diff_line);
                line.mapping = -(frequent_lines.len() as i32);
            } else {
                infrequent_lines.push(diff_line);
                line.mapping = infrequent_lines.len() as i32;
            }
        }

        let old;
        let new;
        if short_is_old {
            (old, new) = (short, long);
        } else {
            (old, new) = (long, short);
        }

        DiffProblem {
            old: old.into_iter().map(|idx| temp_lines[idx].mapping).collect(),
            new: new.into_iter().map(|idx| temp_lines[idx].mapping).collect(),
            frequent_lines,
            infrequent_lines,
            pos_backing,
            cutoff,
        }
    }

    fn get_line(&self, line_idx: i32) -> Option<&DiffLine> {
            if line_idx < 0 { Some(&self.frequent_lines[(-line_idx) as usize - 1]) }
            else if line_idx == 0 { None }
            else { Some(&self.infrequent_lines[line_idx as usize - 1]) }
    }

    fn get_pos_old(&self, line_idx: i32) -> &[u32] {
        match self.get_line(line_idx) {
        Some(line) => self.pos_backing.get(line.pos_old),
        None => &[],
        }
    }

    fn get_pos_new(&self, line_idx: i32) -> &[u32] {
        match self.get_line(line_idx) {
        Some(line) => self.pos_backing.get(line.pos_new),
        None => &[],
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SweepPoint {
    linenum: u32,
    node_ref: SweepNodeRef,
}

#[derive(Debug)]
struct SweepNode {
    old_linenum: u32,
    new_linenum: u32,
    pred_ref: SweepNodeRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SweepNodeRef {
    idx: u32,
    num_matched: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SweepLineResult {
    Exact,
    HaveCutoff,
}

#[derive(Debug)]
struct SweepLine {
    problem: DiffProblem,
    nodes: Vec<SweepNode>,
    sweep_line: Vec<SweepPoint>,
    subproblem_counter: Vec<usize>,
}
impl SweepLine {
    fn new(problem: DiffProblem) -> Self {
        let subproblem_counter = std::iter::repeat(0).take(problem.frequent_lines.len()).collect();
        Self {
            problem,
            nodes: [SweepNode {
                old_linenum: 1,
                new_linenum: 1,
                pred_ref: SweepNodeRef { idx: 0, num_matched: 0 }
            }].into(),
            sweep_line: Vec::new(),
            subproblem_counter,
        }
    }
}

/// Provide a "view" of SweepLine in terms of "x" and "y" instead of "old" and
/// "new", with either possible mapping.
///
/// We use this to generically implement algorithms that work on ranges of
/// both old and new lines, and which may benefit to flip the order of
/// iteration.
#[derive(Debug)]
struct SweepLineXY<'a, const OLD_IS_X: bool> {
    problem: &'a mut DiffProblem,
    nodes: &'a mut Vec<SweepNode>,
    sweep_line: &'a mut Vec<SweepPoint>,
    subproblem_counter: &'a mut Vec<usize>,
}

impl<'a, const OLD_IS_X: bool> SweepLineXY<'a, OLD_IS_X> {
    fn old_new<T>(x: T, y: T) -> (T, T) {
        if OLD_IS_X {
            (x, y)
        } else {
            (y, x)
        }
    }

    fn x_y<T>(old: T, new: T) -> (T, T) {
        if OLD_IS_X {
            (old, new)
        } else {
            (new, old)
        }
    }

    fn node_x(node: &SweepNode) -> u32 {
        Self::x_y(node.old_linenum, node.new_linenum).0
    }

    fn node_y(node: &SweepNode) -> u32 {
        Self::x_y(node.old_linenum, node.new_linenum).1
    }

    fn line_ys(problem: &DiffProblem, line_idx: i32) -> &[u32] {
        if OLD_IS_X {
            problem.get_pos_new(line_idx)
        } else {
            problem.get_pos_old(line_idx)
        }
    }

    fn make_node(&self, x: u32, y: u32, pred_ref: SweepNodeRef) -> SweepNode {
        let (old_linenum, new_linenum) = Self::old_new(x, y);
        SweepNode { old_linenum, new_linenum, pred_ref }
    }

    fn run(&mut self, subproblem_counter: usize, start_node_ref: SweepNodeRef,
           end_old_linenum: u32, end_new_linenum: u32) -> (SweepLineResult, SweepNodeRef) {

        let start_node = &self.nodes[start_node_ref.idx as usize];
        let begin_old_linenum = start_node.old_linenum + start_node_ref.num_matched;
        let begin_new_linenum = start_node.new_linenum + start_node_ref.num_matched;
        assert!(begin_old_linenum >= 1);
        assert!(begin_new_linenum >= 1);

        let (begin_x, begin_y) = Self::x_y(begin_old_linenum, begin_new_linenum);
        let (end_x, end_y) = Self::x_y(end_old_linenum, end_new_linenum);

        assert!(self.sweep_line.is_empty());
        self.sweep_line.push(SweepPoint {
            linenum: begin_y - 1,
            node_ref: start_node_ref,
        });

        let mut last_fast_insert = 0;
        let mut have_cutoff = false;

        for x in begin_x..end_x {
            let line_idx = Self::x_y(&self.problem.old, &self.problem.new).0[x as usize - 1];

            let ys;
            if subproblem_counter == 0 {
                if line_idx <= 0 {
                    if line_idx != 0 {
                        have_cutoff = true;
                    }
                    continue
                }

                let mut tmp = Self::line_ys(self.problem, line_idx);
                while let Some((&y, tail)) = tmp.split_first() {
                    if y >= begin_y {
                        break
                    }
                    tmp = tail;
                }
                if tmp.is_empty() {
                    continue
                }
                ys = tmp;
            } else {
                if line_idx >= 0 {
                    if line_idx == 0 {
                        continue
                    }

                    ys = Self::line_ys(self.problem, line_idx);
                } else {
                    let counter = &mut self.subproblem_counter[(-line_idx) as usize - 1];
                    if *counter == subproblem_counter {
                        continue
                    }
                    *counter = subproblem_counter;

                    let line = &mut self.problem.frequent_lines[(-line_idx) as usize - 1];
                    let old = self.problem.pos_backing.get(line.pos_old);
                    let new = self.problem.pos_backing.get(line.pos_new);

                    let old_begin = old.partition_point(|&linenum| linenum < begin_old_linenum);
                    let old_end = old.partition_point(|&linenum| linenum < end_old_linenum);

                    let new_begin = new.partition_point(|&linenum| linenum < begin_new_linenum);
                    let new_end = new.partition_point(|&linenum| linenum < end_new_linenum);

                    let old_count = old_end - old_begin;
                    let new_count = new_end - new_begin;

                    if old_count == old.len() && new_count == new.len() {
                        have_cutoff = true;
                        continue
                    }

                    let new_line = DiffLine {
                        pos_old: (line.pos_old.0 + old_begin as u32, old_count as u32),
                        pos_new: (line.pos_new.0 + new_begin as u32, new_count as u32),
                    };

                    line.pos_old.1 = old_begin as u32;
                    line.pos_new.1 = new_begin as u32;

                    let new_line_idx;
                    if old_count == 0 || new_count == 0 {
                        new_line_idx = 0;
                    } else {
                        if old_count > self.problem.cutoff && new_count > self.problem.cutoff {
                            have_cutoff = true;
                            self.problem.frequent_lines.push(new_line);
                            self.subproblem_counter.push(subproblem_counter);
                            new_line_idx = -(self.problem.frequent_lines.len() as i32);
                        } else {
                            self.problem.infrequent_lines.push(new_line);
                            new_line_idx = self.problem.infrequent_lines.len() as i32;
                        }
                    }

                    let old_clamped = self.problem.pos_backing.get(new_line.pos_old);
                    for &linenum in old_clamped {
                        self.problem.old[linenum as usize - 1] = new_line_idx;
                    }

                    let new_clamped = self.problem.pos_backing.get(new_line.pos_new);
                    for &linenum in new_clamped {
                        self.problem.new[linenum as usize - 1] = new_line_idx;
                    }

                    if new_line_idx <= 0 {
                        continue
                    }

                    ys = Self::x_y(old_clamped, new_clamped).1;
                }
            }

            fn can_insert_at(linenum: u32, at: Option<&SweepPoint>) -> bool {
                at.map(|point| linenum < point.linenum).unwrap_or(true)
            }

            if ys.len() == 1 {
                // Fast path for sequences of uniquely matchable lines.
                let y = ys[0];
                if y < begin_y || end_y <= y {
                    continue
                }

                let mut last_point = &self.sweep_line[last_fast_insert];
                last_fast_insert += 1;

                if last_point.linenum >= y ||
                   !can_insert_at(y, self.sweep_line.get(last_fast_insert)) {
                    // Fast insert sequence got interrupted. Need to re-find our
                    // position.
                    last_fast_insert = self.sweep_line.partition_point(|point| point.linenum < y);
                    last_point = &self.sweep_line[last_fast_insert - 1];
                    if !can_insert_at(y, self.sweep_line.get(last_fast_insert)) {
                        continue
                    }
                }

                let point =
                    if last_point.linenum + 1 == y &&
                       Self::node_x(&self.nodes[last_point.node_ref.idx as usize])
                            + last_point.node_ref.num_matched == x {
                        assert!(
                            Self::node_y(&self.nodes[last_point.node_ref.idx as usize])
                                + last_point.node_ref.num_matched == y);
                        SweepPoint {
                            linenum: y,
                            node_ref: SweepNodeRef {
                                idx: last_point.node_ref.idx,
                                num_matched: last_point.node_ref.num_matched + 1,
                            },
                        }
                    } else {
                        self.nodes.push(self.make_node(x, y, last_point.node_ref));
                        SweepPoint {
                            linenum: y,
                            node_ref: SweepNodeRef {
                                idx: self.nodes.len() as u32 - 1,
                                num_matched: 1,
                            },
                        }
                    };

                if last_fast_insert >= self.sweep_line.len() {
                    self.sweep_line.push(point);
                } else {
                    self.sweep_line[last_fast_insert] = point;
                }
                continue
            }

            for &y in ys.iter().rev() {
                let idx = self.sweep_line.partition_point(|point| point.linenum < y);
                let pred_point = self.sweep_line[idx - 1];
                let point = SweepPoint {
                    linenum: y,
                    node_ref: SweepNodeRef {
                        idx: self.nodes.len() as u32,
                        num_matched: 1,
                    },
                };

                if idx == self.sweep_line.len() {
                    self.sweep_line.push(point);
                } else if y < self.sweep_line[idx].linenum {
                    self.sweep_line[idx] = point;
                } else {
                    continue
                }

                self.nodes.push(self.make_node(x, y, pred_point.node_ref));
            }
        };

        for line_idx in &mut self.problem.old[begin_old_linenum as usize - 1..end_old_linenum as usize - 1] {
            *line_idx = std::cmp::min(*line_idx, 0);
        }
        for line_idx in &mut self.problem.new[begin_new_linenum as usize - 1..end_new_linenum as usize - 1] {
            *line_idx = std::cmp::min(*line_idx, 0);
        }
        self.problem.infrequent_lines.clear();

        let result = if have_cutoff { SweepLineResult::HaveCutoff } else { SweepLineResult::Exact };
        let end_node_ref = self.sweep_line.last().unwrap().node_ref;
        self.sweep_line.clear();
        (result, end_node_ref)
    }
}

impl SweepLine {
    fn run(&mut self, subproblem_counter: usize, start_node_ref: SweepNodeRef,
           end_old_linenum: u32, end_new_linenum: u32) -> (SweepLineResult, SweepNodeRef) {
        let start_node = &self.nodes[start_node_ref.idx as usize];
        let old_count = end_old_linenum - start_node.old_linenum;
        let new_count = end_new_linenum - start_node.new_linenum;

        if old_count <= new_count {
            SweepLineXY::<true> {
                problem: &mut self.problem,
                nodes: &mut self.nodes,
                sweep_line: &mut self.sweep_line,
                subproblem_counter: &mut self.subproblem_counter,
            }.run(subproblem_counter, start_node_ref, end_old_linenum, end_new_linenum)
        } else {
            SweepLineXY::<true> {
                problem: &mut self.problem,
                nodes: &mut self.nodes,
                sweep_line: &mut self.sweep_line,
                subproblem_counter: &mut self.subproblem_counter,
            }.run(subproblem_counter, start_node_ref, end_old_linenum, end_new_linenum)
        }
    }

    /// Force a split of the given sub-problem in a way that guarantees a
    /// logarithmic depth of the sub-problem tree.
    ///
    /// Keep this simple: it shouldn't happen outside of really weird or
    /// adversarial inputs, so let's not worry too much about diff quality here.
    fn force_split(&mut self, subproblem_counter: usize, start_node_ref: SweepNodeRef,
                   end_old_linenum: u32, end_new_linenum: u32) -> SweepNodeRef {
        let start_node = &self.nodes[start_node_ref.idx as usize];
        let begin_old_linenum = start_node.old_linenum + start_node_ref.num_matched;
        let begin_new_linenum = start_node.new_linenum + start_node_ref.num_matched;

        let old_count = end_old_linenum - begin_old_linenum;
        let new_count = end_new_linenum - begin_new_linenum;
        let lines;
        if old_count <= new_count {
            lines = &self.problem.old[begin_old_linenum as usize - 1..end_old_linenum as usize - 1];
        } else {
            lines = &self.problem.new[begin_new_linenum as usize - 1..end_new_linenum as usize - 1];
        }

        let mut best: Option<(&[u32], &[u32], u64)> = None;
        for &line_idx in lines {
            assert!(line_idx <= 0);
            if line_idx >= 0 {
                continue
            }

            let counter = &mut self.subproblem_counter[(-line_idx) as usize - 1];
            if *counter == subproblem_counter {
                continue
            }
            *counter = subproblem_counter;

            fn strip_front(mut linenums: &[u32], begin: u32) -> &[u32] {
                while let Some((&linenum, tail)) = linenums.split_first() {
                    if linenum >= begin {
                        break
                    }
                    linenums = tail;
                }
                linenums
            }

            let new_linenums = strip_front(self.problem.get_pos_new(line_idx), begin_new_linenum);
            if new_linenums.is_empty() {
                continue
            }
            assert!(*new_linenums.last().unwrap() < end_new_linenum);

            let old_linenums = strip_front(self.problem.get_pos_old(line_idx), begin_old_linenum);
            assert!(*old_linenums.last().unwrap() < end_old_linenum);

            let count = std::cmp::min(old_linenums.len(), new_linenums.len());

            fn cost_fn(first: u32, second: u32) -> u64 {
                let gap = (second - first - 1) as u64;
                gap * gap
            }
            fn find_best_points(begin: u32, points: &[u32], end: u32, count: usize) -> (&[u32], u64) {
                let mut cost: u64 = 0;
                for (&prev, &next) in
                    [begin - 1].iter().chain(&points[..count])
                        .zip(points.iter().chain([end].iter())) {
                    cost += cost_fn(prev, next);
                }

                let mut best_start = 0;
                let mut best_line_cost = cost;

                for (shift, ((&del, &del_next), (&ins_prev, &ins))) in
                    points.iter().zip(&points[1..])
                        .zip(points[count - 1..].iter().zip(&points[count..]))
                        .enumerate()
                {
                    cost -= cost_fn(begin - 1, del);
                    cost -= cost_fn(del, del_next);
                    cost += cost_fn(begin - 1, del_next);

                    cost -= cost_fn(ins_prev, end);
                    cost += cost_fn(ins_prev, ins);
                    cost += cost_fn(ins, end);

                    if cost < best_line_cost {
                        best_line_cost = cost;
                        best_start = 1 + shift;
                    }
                }

                (&points[best_start..best_start + count], best_line_cost)
            }

            let (old, old_cost) = find_best_points(begin_old_linenum, old_linenums, end_old_linenum, count);
            let (new, new_cost) = find_best_points(begin_new_linenum, new_linenums, end_new_linenum, count);
            let cost = old_cost + new_cost;

            if best.is_none() || cost < best.unwrap().2 {
                best = Some((old, new, cost));
            }
        }

        let mut pred_ref = start_node_ref;

        if let Some((old_linenums, new_linenums, _)) = best {
            for (&old_linenum, &new_linenum) in old_linenums.iter().zip(new_linenums) {
                let pred_node = &self.nodes[pred_ref.idx as usize];
                if pred_node.old_linenum + 1 == old_linenum &&
                   pred_node.new_linenum + 1 == new_linenum {
                    pred_ref.num_matched += 1
                } else {
                    self.nodes.push(SweepNode { old_linenum, new_linenum, pred_ref });
                    pred_ref = SweepNodeRef {
                        idx: self.nodes.len() as u32 - 1,
                        num_matched: 1,
                    };
                }
            }
        }

        pred_ref
    }
}

fn diff_sweep_line_exact(buffer: &Buffer, old_begin: u32, new_begin: u32,
                         old: &[DiffRef], new: &[DiffRef], unimportant: bool) -> Vec<Block> {
    let problem = DiffProblem::new(buffer, old, new, usize::MAX);
    let mut sweep_line = SweepLine::new(problem);

    sweep_line.sweep_line.reserve_exact(new.len());
    sweep_line.nodes.reserve(2 * new.len());

    let (_, mut current_ref) =
        sweep_line.run(0, SweepNodeRef { idx: 0, num_matched: 0 },
                       old.len() as u32 + 1, new.len() as u32 + 1);
    let mut collect = ReverseBlockCollector::new(old_begin, new_begin, old, new, unimportant);
    while current_ref.num_matched != 0 {
        let node = &sweep_line.nodes[current_ref.idx as usize];
        collect.add_unchanged(node.old_linenum, node.new_linenum, current_ref.num_matched);
        current_ref = node.pred_ref;
    }
    collect.finish()
}

fn diff_sweep_line(buffer: &Buffer, old_begin: u32, new_begin: u32,
                   old: &[DiffRef], new: &[DiffRef], unimportant: bool) -> Vec<Block> {
    let problem = DiffProblem::new(buffer, old, new, 3);
    let mut sweep_line = SweepLine::new(problem);
    let mut collect = ReverseBlockCollector::new(old_begin, new_begin, old, new, unimportant);

    sweep_line.sweep_line.reserve_exact(new.len());
    sweep_line.nodes.reserve(2 * new.len());

    // Whether and how to recurse.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum RecursePolicy {
        No,
        TopLevel(u32),
        ForceSplit(u32),
    }

    // Stack entries are (node_idx, recurse policy)
    type StackEntry = (u32, RecursePolicy);
    let mut stack: Vec<StackEntry> = Vec::new();
    let mut top_of_stack: StackEntry = (0, RecursePolicy::TopLevel(1));

    // let (result, mut current_idx) = sweep_line.run(0, 0, old.len() as u32 + 1, new.len() as u32 + 1);


    let mut subproblem_counter = 0;
    let mut current_ref = SweepNodeRef { idx: 0, num_matched: 0 };
    let mut prev_old_linenum = old.len() as u32 + 1;
    let mut prev_new_linenum = new.len() as u32 + 1;

    loop {
        sweep_line.nodes.truncate(current_ref.idx as usize + 1);
        let mut node = &sweep_line.nodes[current_ref.idx as usize];

        #[cfg(feature = "debug-diff")]
        println!("old: {}+{}..{}  new: {}+{}..{}",
            node.old_linenum, current_ref.num_matched, prev_old_linenum,
            node.new_linenum, current_ref.num_matched, prev_new_linenum);

        if top_of_stack.1 != RecursePolicy::No {
            // Scan for matching lines at the head and tail of the unmatched region
            // between nodes.
            //
            // Heuristic: Scan from the head before scanning from the tail.
            // The rationale is that this leads to nicer-looking diffs in the
            // common case of adding an entire new function between other functions
            // in typical source code: the initial sweep-line run results in an
            // unmatched section of the form:
            //
            //    OLD                 NEW
            //
            //    | }                 | }
            //    |                   |
            //                        | function header {
            //                        |   ...
            //                        | }
            //                        |
            //
            // The old lines can fully match either at the head or the tail. But
            // logically, they belong to the preceding function, so it's better to
            // match them at the head.
            //
            // This heuristic still isn't perfect for code that has function
            // comments framed by ASCII art.
            //
            // TODO: Explain why we don't do the tail
            let count = std::cmp::min(prev_old_linenum - node.old_linenum - current_ref.num_matched,
                                      prev_new_linenum - node.new_linenum - current_ref.num_matched);
            let head_match_count =
                sweep_line.problem.old[(node.old_linenum + current_ref.num_matched - 1) as usize..].iter()
                    .zip(&sweep_line.problem.new[(node.new_linenum + current_ref.num_matched - 1) as usize..])
                    .take(count as usize)
                    .take_while(|(&old, &new)| old != 0 && old == new)
                    .count();
            current_ref.num_matched += head_match_count as u32;

            #[cfg(feature = "debug-diff")]
            if head_match_count != 0 {
                println!("  match head: {}", head_match_count);
            }

            // Scan for and skip lines that are trivially known to be
            // unmatchable. This commonly helps handle sub-problems faster.
            let is_unmatchable = |line_idx: &i32| *line_idx == 0;

            let current_old_linenum = node.old_linenum + current_ref.num_matched;
            let old_count = prev_old_linenum - current_old_linenum;
            let unmatchable_old_count =
                sweep_line.problem.old[current_old_linenum as usize - 1..prev_old_linenum as usize - 1]
                    .iter().cloned().take_while(is_unmatchable).count() as u32;

            let current_new_linenum = node.new_linenum + current_ref.num_matched;
            let new_count = prev_new_linenum - current_new_linenum;
            let unmatchable_new_count =
                sweep_line.problem.new[current_new_linenum as usize - 1..prev_new_linenum as usize - 1]
                    .iter().cloned().take_while(is_unmatchable).count() as u32;

            #[cfg(feature = "debug-diff")]
            if unmatchable_old_count != 0 || unmatchable_new_count != 0 {
                println!("  unmatchable old: {} new: {}", unmatchable_old_count, unmatchable_new_count);
            }

            if unmatchable_old_count < old_count && unmatchable_new_count < new_count {
                if unmatchable_old_count > 0 || unmatchable_new_count > 0 {
                    // Remember that we do not have to revisit the unmatchable lines.
                    if top_of_stack.0 != current_ref.idx {
                        stack.push(top_of_stack);
                        stack.push((current_ref.idx, RecursePolicy::No));
                    } else {
                        if stack.last().map(|&(_, policy)| policy != RecursePolicy::No).unwrap_or(true) {
                            stack.push((current_ref.idx, RecursePolicy::No));
                        }
                    }

                    sweep_line.nodes.push(SweepNode {
                        old_linenum: current_old_linenum + unmatchable_old_count,
                        new_linenum: current_new_linenum + unmatchable_new_count,
                        pred_ref: current_ref,
                    });
                    current_ref = SweepNodeRef {
                        idx: sweep_line.nodes.len() as u32 - 1,
                        num_matched: 0,
                    };
                    top_of_stack.0 = current_ref.idx;
                    continue
                }

                if let RecursePolicy::ForceSplit(limit) = top_of_stack.1 {
                    let size = std::cmp::max(prev_old_linenum - current_old_linenum,
                                             prev_new_linenum - current_new_linenum);
                    if size > limit {
                        // Force split along a heuristically chosen frequent line.
                        current_ref =
                            sweep_line.force_split(
                                subproblem_counter, current_ref,
                                prev_old_linenum, prev_new_linenum);
                        subproblem_counter += 1;
                        continue
                    }
                }

                #[cfg(feature = "debug-diff")]
                println!("  invoke old: {}..{} new: {}..{}",
                    node.old_linenum + current_ref.num_matched, prev_old_linenum,
                    node.new_linenum + current_ref.num_matched, prev_new_linenum);

                let (result_state, result_ref) =
                    sweep_line.run(subproblem_counter, current_ref, prev_old_linenum, prev_new_linenum);
                subproblem_counter += 1;
                let recurse =
                    if result_state == SweepLineResult::HaveCutoff {
                        // We need to recurse because some potentially matchable
                        // lines were ignored.
                        Some(match top_of_stack.1 {
                        RecursePolicy::TopLevel(level) if level > 0 => RecursePolicy::TopLevel(level - 1),
                        _ => {
                            // Split any remaining subproblems that are large. This
                            // ensures that the depth of the subproblem tree is at most
                            // logarithmic.
                            let start_node = &sweep_line.nodes[current_ref.idx as usize];
                            let count = std::cmp::max(
                                prev_old_linenum - start_node.old_linenum - current_ref.num_matched,
                                prev_new_linenum - start_node.new_linenum - current_ref.num_matched);
                            assert!(count as usize > sweep_line.problem.cutoff);

                            RecursePolicy::ForceSplit(count / 3)
                        }
                        })
                    } else if result_ref != current_ref {
                        // The sweep generated new nodes. We need to recurse to
                        // feed them into the collector, but the subproblems are
                        // known to not have any matchable lines.
                        Some(RecursePolicy::No)
                    } else {
                        None
                    };

                if let Some(policy) = recurse {
                    if top_of_stack.0 != current_ref.idx {
                        stack.push(top_of_stack);
                    }
                    top_of_stack = (current_ref.idx, policy);

                    current_ref = result_ref;
                    continue
                }

                // No new nodes were found and we confirmed that no further
                // matches are possible. We may have matched some additional , proceed as if we didn't even attempt.
                node = &sweep_line.nodes[current_ref.idx as usize];
            }
        }

        if current_ref.num_matched != 0 {
            collect.add_unchanged(node.old_linenum, node.new_linenum, current_ref.num_matched as u32);
        }

        if current_ref.idx == top_of_stack.0 {
            if current_ref.idx == 0 {
                break
            }
            top_of_stack = stack.pop().unwrap();
        }

        current_ref = node.pred_ref;
        prev_old_linenum = node.old_linenum;
        prev_new_linenum = node.new_linenum;
    }
    collect.finish()
}

/// Reduce changed blocks by applying a diff algorithm within each changed block.
///
/// Returns the reduced file and a boolean indicating whether there is any
/// change remaining.
pub fn reduce_changed_file(buffer: &Buffer, mut file: DiffFile, algorithm: DiffAlgorithm) -> (DiffFile, bool) {
    let mut have_change = false;

    let BlockContents::EndOfDiff { old_has_newline_at_eof, new_has_newline_at_eof, .. }
        = file.blocks.last().unwrap().contents else { panic!() };
    let mut blocks = std::mem::take(&mut file.blocks).into_iter().peekable();
    loop {
        let block = blocks.next();
        if block.is_none() {
            break
        }
        let mut block = block.unwrap();

        if !block.contents.is_changed() {
            file.blocks.push(block);
            continue;
        }

        let BlockContents::Changed { old, new, unimportant } = block.contents else { panic!() };
        let mut old = &old[..];
        let mut new = &new[..];

        // First, trim back and front. This is required for correctness if
        // the last line is at EOF and differs in newlines.
        //
        // It is always optimal in terms of number of unchanged lines that
        // are extracted, and since head and tail are likely to be merged
        // with neighboring unchanged blocks, it is likely to be best for
        // readability of the resulting diff.
        let mut tail: Vec<Block> = Vec::new();

        if old_has_newline_at_eof != new_has_newline_at_eof &&
           !old.is_empty() && !new.is_empty() &&
           blocks.peek().unwrap().is_end_of_diff() {
            let old_line;
            let new_line;
            (old_line, old) = old.split_last().unwrap();
            (new_line, new) = new.split_last().unwrap();

            have_change = true;
            tail.push(Block {
                old_begin: block.old_begin + old.len() as u32,
                new_begin: block.new_begin + new.len() as u32,
                contents: BlockContents::Changed {
                    old: [*old_line].into(),
                    new: [*new_line].into(),
                    unimportant,
                },
            });
        }

        let tail_count = old.iter().rev().zip(new.iter().rev())
            .take_while(|(&old_ref, &new_ref)| &buffer[old_ref] == &buffer[new_ref])
            .count();
        if tail_count > 0 {
            let old_lines;
            (old, old_lines) = old.split_at(old.len() - tail_count);
            (new, _) = new.split_at(new.len() - tail_count);

            tail.push(Block {
                old_begin: block.old_begin + old.len() as u32,
                new_begin: block.new_begin + new.len() as u32,
                contents: BlockContents::UnchangedKnown(old_lines.into()),
            });
        }

        let head_count = old.iter().zip(new.iter())
            .take_while(|(&old_ref, &new_ref)| &buffer[old_ref] == &buffer[new_ref])
            .count();
        if head_count > 0 {
            let old_lines;
            (old_lines, old) = old.split_at(head_count);
            (_, new) = new.split_at(head_count);

            file.blocks.push(Block {
                old_begin: block.old_begin,
                new_begin: block.new_begin,
                contents: BlockContents::UnchangedKnown(old_lines.into()),
            });
            block.old_begin += head_count as u32;
            block.new_begin += head_count as u32;
        }

        if !old.is_empty() || !new.is_empty() {
            have_change = true;
            file.blocks.extend(algorithm.run(buffer, block.old_begin, block.new_begin,
                                             old, new, unimportant));
        }

        file.blocks.extend(tail.into_iter().rev());
    }

    file.simplify();

    (file, have_change)
}

/// Reduce changed blocks by applying a diff algorithm within each changed block.
pub fn reduce_changed_diff(buffer: &Buffer, mut diff: Diff, algorithm: DiffAlgorithm) -> Diff {
    for mut file in std::mem::take(&mut diff.files) {
        let have_change;
        (file, have_change) = reduce_changed_file(buffer, file, algorithm);

        if have_change {
            diff.files.push(file);
        }
    }

    diff
}
