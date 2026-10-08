//! Shared validation for workflow stages and batch-local dependencies.

use crate::error::CoreError;
use std::collections::{HashMap, HashSet, VecDeque};

/// Keep input order stable while resolving IDs into batch-local predecessor indices.
pub fn resolve_dependencies(
    ids: &[String],
    dependencies: &[Vec<String>],
) -> Result<Vec<Vec<usize>>, CoreError> {
    let invalid = |message: String| {
        CoreError::InvalidInput(format!("Invalid workflow dependencies: {message}"))
    };
    if ids.len() != dependencies.len() {
        return Err(invalid("stage/dependency length mismatch".into()));
    }
    let mut lookup = HashMap::new();
    for (index, id) in ids.iter().enumerate() {
        if id.trim().is_empty() || id != id.trim() || lookup.insert(id.as_str(), index).is_some() {
            return Err(invalid(format!(
                "stage IDs must be unique and nonempty: {id:?}"
            )));
        }
    }
    let mut resolved = Vec::with_capacity(ids.len());
    let mut outgoing = vec![Vec::new(); ids.len()];
    for (index, edges) in dependencies.iter().enumerate() {
        let mut seen = HashSet::new();
        let mut predecessors = Vec::with_capacity(edges.len());
        for edge in edges {
            let predecessor = *lookup.get(edge.as_str()).ok_or_else(|| {
                invalid(format!("{} references unknown stage {edge:?}", ids[index]))
            })?;
            if predecessor == index || !seen.insert(predecessor) {
                return Err(invalid(format!(
                    "{} has a self-reference or duplicate dependency {edge:?}",
                    ids[index]
                )));
            }
            outgoing[predecessor].push(index);
            predecessors.push(predecessor);
        }
        resolved.push(predecessors);
    }
    let mut remaining = resolved.iter().map(Vec::len).collect::<Vec<_>>();
    let mut ready = remaining
        .iter()
        .enumerate()
        .filter_map(|(index, count)| (*count == 0).then_some(index))
        .collect::<VecDeque<_>>();
    let mut visited = 0;
    while let Some(index) = ready.pop_front() {
        visited += 1;
        for successor in &outgoing[index] {
            remaining[*successor] -= 1;
            if remaining[*successor] == 0 {
                ready.push_back(*successor);
            }
        }
    }
    if visited != ids.len() {
        return Err(invalid("cycle detected; no stages were launched".into()));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_reverse_order_and_rejects_malformed_graphs() {
        let ids = vec!["review".into(), "draft".into(), "research".into()];
        assert_eq!(
            resolve_dependencies(
                &ids,
                &[vec!["draft".into()], vec!["research".into()], vec![]]
            )
            .unwrap(),
            vec![vec![1], vec![2], vec![]]
        );
        for graph in [
            vec![vec!["missing".into()], vec![], vec![]],
            vec![vec!["review".into()], vec![], vec![]],
            vec![vec!["draft".into(), "draft".into()], vec![], vec![]],
            vec![vec!["draft".into()], vec!["review".into()], vec![]],
        ] {
            assert!(resolve_dependencies(&ids, &graph).is_err());
        }
        assert!(resolve_dependencies(&["same".into(), "same".into()], &[vec![], vec![]]).is_err());
    }
}
