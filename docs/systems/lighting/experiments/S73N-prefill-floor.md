# S73N — Pre-fill floor inside the texel room

S73G spent pre-fill only from what young and mature left under the base ceiling. `lighting_write_args` then clamped the dispatch to the room, and the pre-fill slots began after the whole mature list. During a turn that list fills the room, so `prefill_taken` went to 0 even while thousands of never-shown texels were waiting.

`lighting_sc_prefill_plan` is shared by the dispatch size and the batch slot:

- Young stays capped at the base room.
- Pre-fill entries are the spare under that ceiling, or `1024 >> budget_shift` when the spare is smaller, and never more than the list or the room.
- Mature entries actually dispatched are what the room has left after young and that pre-fill count. The pre-fill slots start there, not after the full mature list.
- While the boost is on and the list is non-empty, the next round's mature histogram leaves the same floor out, so the carve is not only a cut off a stable tail.

A young list that fills the room dispatches no pre-fill. One event of spare at least the floor is unchanged from S73G.

Deviation: this is still not Lumen's always-spent fixed budget. The floor is a minimum inside the texel room, taken from mature entries, and a larger spare is still spent. The guard band of primary rays and the rotating start on the list stay as they were.
