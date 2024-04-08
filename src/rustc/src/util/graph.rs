use core::{fmt, hash};

use rustc_hash::FxHashSet;

use super::hash::FxHashMap;

const INFINITE_DEPTH: u32 = u32::MAX;

type GraphPropagatorFunc<'f, Cx, Node, Data> =
    dyn Fn(&mut GraphPropagatorCx<'_, 'f, Cx, Node, Data>, Node) -> Data + 'f;

pub struct GraphPropagator<'f, Cx, Node, Data> {
    // User supplied context
    cx: Cx,

    // User-supplied callback for computing facts for a node.
    compute_facts: &'f GraphPropagatorFunc<'f, Cx, Node, Data>,

    // A mapping from nodes which have finished computing to their facts.
    fact_map: FxHashMap<Node, Data>,

    // A mapping from nodes which have *not* finished computing to their depth in the DFS.
    depth_map: FxHashMap<Node, u32>,

    // An index-map from depths to the set of nodes which recurse back to it.
    scc_sets: Vec<FxHashSet<Node>>,
}

impl<'f, Cx, Node, Data> GraphPropagator<'f, Cx, Node, Data>
where
    Node: fmt::Debug + Copy + hash::Hash + Eq,
    Data: Clone,
{
    pub fn new(cx: Cx, compute_facts: &'f GraphPropagatorFunc<'f, Cx, Node, Data>) -> Self {
        Self {
            cx,
            compute_facts,
            fact_map: FxHashMap::default(),
            depth_map: FxHashMap::default(),
            scc_sets: Vec::new(),
        }
    }

    pub fn context(&self) -> &Cx {
        &self.cx
    }

    pub fn context_mut(&mut self) -> &mut Cx {
        &mut self.cx
    }

    pub fn fact_computer(&self) -> &'f GraphPropagatorFunc<'f, Cx, Node, Data> {
        self.compute_facts
    }

    pub fn fact_map(&self) -> &FxHashMap<Node, Data> {
        &self.fact_map
    }

    pub fn fact_map_mut(&mut self) -> &mut FxHashMap<Node, Data> {
        &mut self.fact_map
    }

    pub fn into_fact_map(self) -> FxHashMap<Node, Data> {
        self.fact_map
    }

    pub fn analyze(&mut self, start: Node) -> &Data {
        self.analyze_inner(start, 0);
        &self.fact_map[&start]
    }

    fn analyze_inner(&mut self, my_node: Node, my_depth: u32) -> u32 {
        // Ensure that we're not recursing to a node which is in the process of being visited.
        if self.fact_map.contains_key(&my_node) {
            return INFINITE_DEPTH;
        }

        if let Some(depth) = self.depth_map.get(&my_node) {
            return *depth;
        }

        self.depth_map.insert(my_node, my_depth);

        // Add an entry to the `scc_sets` map.
        self.scc_sets.push(FxHashSet::default());

        // Compute facts for this node
        let compute_facts = self.compute_facts;
        let mut prop_cx = GraphPropagatorCx {
            propagator: self,
            min_back_depth: INFINITE_DEPTH,
            child_depth: my_depth + 1,
        };
        let my_facts = (compute_facts)(&mut prop_cx, my_node);
        let min_back_depth = prop_cx.min_back_depth;

        // If we were self-recursive, add an entry to the `scc_sets` map.
        // We don't push ourself because there's no need to clone our own facts.
        if min_back_depth != INFINITE_DEPTH && min_back_depth != my_depth {
            self.scc_sets[min_back_depth as usize].insert(my_node);
        }

        // If we're the root of the SCC, let's copy over all our facts to everything in the SCC.
        let my_scc_set = self.scc_sets.pop().unwrap();
        let min_back_depth_for_caller = if min_back_depth == my_depth {
            for node in my_scc_set {
                self.fact_map.get_mut(&node).unwrap().clone_from(&my_facts);
            }

            // We just discharged the back-references and parent functions only care whether
            // their ancestors were referenced by a descendant.
            INFINITE_DEPTH
        } else if min_back_depth == INFINITE_DEPTH {
            // Otherwise, if no descendants of this node contribute to an SCC, let's just do nothing.

            // We just discharged the back-references and parent functions only care whether
            // their ancestors were referenced by a descendant.
            INFINITE_DEPTH
        } else {
            // Otherwise, an even earlier function has to take care of unifying the SCC.
            self.scc_sets.last_mut().unwrap().extend(my_scc_set);

            // An ancestor still has to handle this.
            min_back_depth
        };

        // Update the fact map
        self.fact_map.insert(my_node, my_facts);
        self.depth_map.remove(&my_node);

        min_back_depth_for_caller
    }
}

pub struct GraphPropagatorCx<'p, 'f, Cx, Node, Data> {
    propagator: &'p mut GraphPropagator<'f, Cx, Node, Data>,
    min_back_depth: u32,
    child_depth: u32,
}

impl<'p, 'f, Cx, Node, Data> GraphPropagatorCx<'p, 'f, Cx, Node, Data>
where
    Node: fmt::Debug + Copy + hash::Hash + Eq,
    Data: Clone,
{
    pub fn analyze(&mut self, node: Node) -> Option<&mut Data> {
        let its_depth = self.propagator.analyze_inner(node, self.child_depth);
        self.min_back_depth = self.min_back_depth.min(its_depth);
        self.propagator.fact_map_mut().get_mut(&node)
    }

    pub fn context_mut(&mut self) -> &mut Cx {
        self.propagator.context_mut()
    }
}
