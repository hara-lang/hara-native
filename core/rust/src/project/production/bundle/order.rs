use super::model::RenderedModule;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn module_order(modules: &[RenderedModule]) -> Vec<usize> {
    let positions = modules
        .iter()
        .enumerate()
        .map(|(index, module)| (module.resource.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut remaining = (0..modules.len()).collect::<BTreeSet<_>>();
    let mut ordered = Vec::with_capacity(modules.len());

    while !remaining.is_empty() {
        let next = remaining.iter().copied().find(|index| {
            modules[*index].dependencies.iter().all(|dependency| {
                match positions.get(dependency.as_str()) {
                    Some(dependency_index) => !remaining.contains(dependency_index),
                    None => true,
                }
            })
        });
        let Some(next) = next else {
            // Cyclic namespace declarations are deterministic too. Global
            // providers are predeclared before compilation, so retain lexical
            // order and let runtime evaluation enforce ordinary cycle safety.
            ordered.extend(remaining.iter().copied());
            break;
        };
        remaining.remove(&next);
        ordered.push(next);
    }
    ordered
}
