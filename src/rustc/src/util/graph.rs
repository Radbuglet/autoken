use std::collections::VecDeque;

use petgraph::{
    algo::TarjanScc,
    graph::{EdgeIndex, NodeIndex},
    stable_graph::StableGraph,
    visit::{EdgeRef, IntoNeighbors, IntoNodeIdentifiers, NodeIndexable},
    Direction,
};

#[derive(Default)]
pub struct OutEdgeIter {
    buffer: Vec<(EdgeIndex, NodeIndex)>,
}

impl OutEdgeIter {
    pub fn iter<N, E>(
        &mut self,
        graph: &mut StableGraph<N, E>,
        node: NodeIndex,
    ) -> impl ExactSizeIterator<Item = (EdgeIndex, NodeIndex)> + '_ {
        self.buffer.clear();
        self.buffer.extend(
            graph
                .edges_directed(node, Direction::Outgoing)
                .map(|edge| (edge.id(), edge.target())),
        );
        self.buffer.iter().copied()
    }
}

pub fn tarjan_scc_filter_trivial<G>(g: G) -> Vec<Vec<G::NodeId>>
where
    G: IntoNodeIdentifiers + IntoNeighbors + NodeIndexable,
{
    let mut sccs = Vec::new();
    {
        let mut tarjan_scc = TarjanScc::new();
        tarjan_scc.run(g, |scc| {
            if scc.len() > 1 {
                sccs.push(scc.to_vec());
            }
        });
    }
    sccs
}

pub fn propagate_graph<N, E>(
    graph: &mut StableGraph<N, E>,
    mut merge_into: impl FnMut(&mut StableGraph<N, E>, EdgeIndex, NodeIndex, NodeIndex),
    mut replicate: impl FnMut(&mut StableGraph<N, E>, NodeIndex, NodeIndex),
) {
    const SENTINEL_IN_SCC_NOT_VISITED: usize = usize::MAX - 1;
    const SENTINEL_IN_SCC_VISITED: usize = usize::MAX;

    let mut out_edges = OutEdgeIter::default();

    // Prepare a regular toposort. We do this now so we can reuse the in-degree buffer to store
    // sentinel values for the SCC phase.
    let mut topo_in_degs = Vec::new();
    let mut topo_visit_queue = Vec::new();

    for node in graph.node_indices() {
        debug_assert_eq!(node.index(), topo_in_degs.len());

        let in_degree = graph.edges_directed(node, Direction::Incoming).count();
        topo_in_degs.push(in_degree);

        if in_degree == 0 {
            topo_visit_queue.push(node);
        }
    }

    // Propagate in strongly-connected components.
    let mut tarjan_visit_queue = VecDeque::new();

    for nodes in tarjan_scc_filter_trivial(&*graph) {
        // Give each of the nodes in the component a sentinel in-degree.
        for &node in &nodes {
            debug_assert_ne!(topo_in_degs[node.index()], 0);
            topo_in_degs[node.index()] = SENTINEL_IN_SCC_NOT_VISITED;
        }

        // Visit every node in the component in level-order.
        let (first, remaining) = nodes.split_first().unwrap();
        tarjan_visit_queue.clear();
        tarjan_visit_queue.push_front(*first);

        while let Some(node) = tarjan_visit_queue.pop_front() {
            // Mark this node as visited.
            topo_in_degs[node.index()] = SENTINEL_IN_SCC_VISITED;

            // Look for nodes which we've yet to visit.
            for (edge, target) in out_edges.iter(graph, node) {
                if topo_in_degs[target.index()] != SENTINEL_IN_SCC_NOT_VISITED {
                    continue;
                }

                // ...and merge them.
                // N.B. the handling of sentinels ensures that we never have the scenario where
                // `node == target`.
                merge_into(graph, edge, node, target);

                // Finally, add them to the queue.
                tarjan_visit_queue.push_back(target);
            }
        }

        debug_assert!(!topo_in_degs
            .iter()
            .any(|v| *v == SENTINEL_IN_SCC_NOT_VISITED));

        // Replicate the first node's results.
        for &remaining in remaining {
            replicate(graph, remaining, *first);
        }
    }

    // Propagate everywhere else.
    while let Some(node) = topo_visit_queue.pop() {
        debug_assert_eq!(topo_in_degs[node.index()], 0);

        for (edge, target) in out_edges.iter(graph, node) {
            // Handle merge
            if node != target {
                merge_into(graph, edge, node, target);
            }

            // Handle toposort logic
            let target_deg = &mut topo_in_degs[target.index()];

            if *target_deg != SENTINEL_IN_SCC_VISITED {
                *target_deg -= 1;

                if *target_deg == 0 {
                    topo_visit_queue.push(target);
                }
            }
        }
    }
}
