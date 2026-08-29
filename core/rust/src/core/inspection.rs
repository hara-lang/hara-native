/// Returns cloned values for every persistent set-like runtime representation.
///
/// Embedding hosts should use this instead of matching a concrete [`Value`]
/// variant. HAL set literals currently evaluate to ordered sets, while explicit
/// constructors may produce hash, ordered, or sorted sets. The returned order
/// follows the representation's iterator and is not part of set semantics.
pub fn set_values(value: &Value) -> Option<Vec<Value>> {
    set_items(value).map(|values| values.into_iter().cloned().collect())
}
