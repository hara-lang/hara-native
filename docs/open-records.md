# Opt-in open records

The `:open true` metadata on a `defstruct` declaration enables persistent
extension entries while retaining the named type and its protocol methods:

```clojure
(defstruct ^{:open true} Snapshot [])
(map->Snapshot {:lua {:id :lua :book {:parent :xtalk}}})
```

Ordinary structs remain closed. Open records retain arbitrary map keys in their
map constructor, association, lookup, membership, iteration, and count.
Positional construction still initializes only the declared fields. The map
constructor preserves input metadata and supplies nil for absent declared fields.

Removing extension keys preserves the record type and protocol dispatch. Removing
a declared field returns an ordinary ordered map, as for existing closed structs.
`empty` preserves the record type, clears extensions, and resets declared fields
to nil. These operations are persistent and do not mutate the input.

## Serialization boundary

The current HTA structure encoding and component structure representation cannot
describe open entries. They reject open records instead of silently dropping
entries or reconstructing a closed record. Convert entries to an ordinary map
explicitly and retain the owning constructor to reconstruct the type. HTA does
not preserve ordinary collection metadata implicitly; carry metadata as explicit
data alongside the map when it is required for reconstruction.

This capability enables Foundation-style Snapshot contents. It does not itself
install or complete the Book, Snapshot, Library, or runtime migrations.
