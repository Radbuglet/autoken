// Pseudocode of the new `propagateFacts` algorithm.

function propagateFacts<Node, Facts extends object>(
    start: Node,
    compute_facts: (
        node: Node,
        compute_facts: (node: Node) => (Facts | null),
    ) => Facts,
): Map<Node, Facts> {
    // Holds either the computed facts for a node or its visit depth if it is in the process of
    // being computed.
    const fact_map = new Map<Node, Facts | number>();

    // An index-map holding the set of nodes which recursed back to the given depth.
    const sccs_to_connect_to: Set<Node>[] = [];

    type RecurseResult = Readonly<{ facts: Facts | null, min_back_depth: number }>;

    function recurse(node: Node, my_depth: number): RecurseResult {
        // Ensure that we're not recursing to a node which is in the process of being visited.
        {
            const facts = fact_map.get(node);

            if (typeof facts === "number") {
                return { facts: null, min_back_depth: 0 };
            } else if (typeof facts === "object") {
                return { facts, min_back_depth: Number.POSITIVE_INFINITY };
            }

            fact_map.set(node, my_depth);
        }

        // Add an entry to the `sccs_to_connect_to` map.
        sccs_to_connect_to.push(new Set());

        // Allow the user to compute facts for this node.
        let min_back_depth = Number.POSITIVE_INFINITY;

        const my_facts = compute_facts(node, node => {
            const facts = recurse(node, my_depth + 1);
            min_back_depth = Math.min(min_back_depth, facts.min_back_depth);
            return facts.facts;
        });

        // If we were self-recursive, add an entry to the `sccs_to_connect_to` map.
        if (min_back_depth !== Number.POSITIVE_INFINITY) {
            sccs_to_connect_to[min_back_depth].add(node);
        }

        // If we're the root of the SCC, let's copy over all our facts to everything in the SCC.
        const my_scc_set = sccs_to_connect_to.pop()!;
        if (min_back_depth === my_depth) {
            for (const node of my_scc_set) {
                fact_map.set(node, my_facts);
            }
        } else {
            // Otherwise, an even earlier function has to take care of unifying the SCC.
            for (const node of my_scc_set) {
                sccs_to_connect_to[-1]!.add(node);
            }
        }

        // TODO
    }

    recurse(start, 0);

    return fact_map as Map<Node, Facts>;
}
