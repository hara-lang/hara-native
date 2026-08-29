use std::collections::HashMap;

use super::{namespace_dependencies, split_namespace_form, ModuleSource};

/// Returns a stable dependency-first order. Only eager `:require`/`:use`
/// edges participate because lazy aliases are runtime loader registrations,
/// not compilation prerequisites.
pub(super) fn order_module_sources(sources: &[ModuleSource<'_>]) -> Result<Vec<usize>, String> {
    let positions = sources
        .iter()
        .enumerate()
        .map(|(index, source)| (source.resource, index))
        .collect::<HashMap<_, _>>();
    let dependencies = sources
        .iter()
        .map(|source| {
            let (namespace_form, _) = split_namespace_form(source.source)?;
            namespace_dependencies(namespace_form)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut state = vec![0u8; sources.len()];
    let mut ordered = Vec::with_capacity(sources.len());

    for index in 0..sources.len() {
        visit(
            index,
            sources,
            &positions,
            &dependencies,
            &mut state,
            &mut ordered,
        )?;
    }
    Ok(ordered)
}

fn visit(
    index: usize,
    sources: &[ModuleSource<'_>],
    positions: &HashMap<&str, usize>,
    dependencies: &[Vec<String>],
    state: &mut [u8],
    ordered: &mut Vec<usize>,
) -> Result<(), String> {
    match state[index] {
        2 => return Ok(()),
        1 => {
            return Err(format!(
                "eager namespace dependency cycle at {}",
                sources[index].resource
            ));
        }
        _ => state[index] = 1,
    }
    for dependency in &dependencies[index] {
        if let Some(&dependency_index) = positions.get(dependency.as_str()) {
            visit(
                dependency_index,
                sources,
                positions,
                dependencies,
                state,
                ordered,
            )?;
        }
    }
    state[index] = 2;
    ordered.push(index);
    Ok(())
}
