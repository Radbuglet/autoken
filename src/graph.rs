use petgraph::{stable_graph::NodeIndex, Graph};

pub fn check_balance(graph: &mut Graph<i32, i32>, entry: NodeIndex) {
    // First, we ensure that there is no path we can take to produce an unbounded positive or
    // negative borrow count.
    //
    // Here is the final boss for this algorithm:
    //
    //    ┌───────────┐  ┌──────────┐
    //    │           │  │          │
    //    │         ┌─▼──▼┐         │
    //    │     ┌───┤Entry├───┐     │
    //    │     │   └──┬──┘   │     │
    //    │+1   │+1    │+1    │-1   │-1
    //    │   ┌─▼─┐  ┌─▼─┐  ┌─▼─┐   │
    //    │   │ A │  │ B │  │ C │   │
    //    │   └─┬─┘  └─┬─┘  └─┬─┘   │
    //    │     │      │-1    │     │
    //    │     │    ┌─▼─┐    │     │
    //    │     └────► D ◄────┘     │
    //    │          └┬┬┬┘          │
    //    │           │││           │
    //    └───────────┘│└───────────┘
    //                 │
    //              ┌──▼──┐
    //              │Exit!│
    //              └─────┘
    //
    // Note that not all paths will result in a unbounded borrow count.
    //
    // A naïve version of this algorithm would just go through every cycle one could construct and
    // ensure that its borrow counts are properly balanced. The running time for this, however, is
    // super-linear, which is unacceptable for a static analysis program.
    //
    // We can do better by realizing that we only have to detect *one* unbalanced cycle per graph.
    // Additionally, if we find a cycle which is properly balanced consisting of nodes:
    //
    // N_1 -> N_2 -> ... -> N_n
    //
    // We know that all other paths between `N_k` and `N_{k + 1}` must have the same exact weight or
    // an alternative unbalanced path will be possible.
    //
    // In other words, for every strongly-connected component of our graph, in order for the borrows
    // to be balanced, every path from the origin to some destination must have the same weight
    // regardless of the path taken.
    // TODO

    // Now, we can disconnect all edges which cycle back into the main execution path since we know
    // that they'll have no impact on the borrow count.
    // TODO

    // Now that our graph is a simple DAG, it is easy to determine the maximum borrow count and the
    // minimum unborrow count by propagating these counts in a topological order.
    // TODO
}
