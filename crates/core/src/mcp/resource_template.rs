use crate::error::CoreError;
use std::collections::{BTreeMap, BTreeSet};
use uri_template_system::{Template, Value, Values};

/// Expand an exact advertised template. The caller must first bind it to the
/// current connector catalog; no URI supplied by the caller is substituted here.
pub(super) fn expand(raw: &str, arguments: &BTreeMap<String, String>) -> Result<String, CoreError> {
    let invalid = |message: &str| CoreError::InvalidInput(message.into());
    if raw.is_empty()
        || raw.len() > 16_384
        || arguments.len() > 128
        || serde_json::to_vec(arguments)?.len() > 64 * 1024
    {
        return Err(invalid(
            "Resource template and arguments exceed the content budget.",
        ));
    }
    let template = Template::parse(raw)
        .map_err(|_| invalid("The connector advertised an invalid RFC 6570 URI template."))?;
    // Parsing above establishes the grammar. Enumerate references only to reject
    // unknown parameters and bound allocation before the RFC expansion runs.
    let mut declared = BTreeSet::new();
    let mut maximum = raw.len();
    for expression in raw.split('{').skip(1) {
        let expression = expression.split_once('}').expect("validated expression").0;
        for variable in expression
            .trim_start_matches(['+', '#', '.', '/', ';', '?', '&'])
            .split(',')
        {
            let variable = variable
                .split([':', '*'])
                .next()
                .expect("validated variable");
            declared.insert(variable);
            maximum = maximum.saturating_add(
                arguments
                    .get(variable)
                    .map_or(0, |value| value.len().saturating_mul(3)),
            );
        }
    }
    if maximum > 64 * 1024 || arguments.keys().any(|key| !declared.contains(key.as_str())) {
        return Err(invalid("Resource arguments must match the advertised URI template and fit within the expansion budget."));
    }
    let values: Values = arguments
        .iter()
        .map(|(key, value)| (key.clone(), Value::item(value.clone())))
        .collect();
    let uri = template
        .expand(&values)
        .map_err(|_| invalid("The resource URI template could not be expanded."))?;
    if uri.is_empty() || uri.len() > 16_384 || reqwest::Url::parse(&uri).is_err() {
        return Err(invalid(
            "The expanded resource URI must be absolute and fit within 16 KiB.",
        ));
    }
    Ok(uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_encodes_scalar_values_and_preserves_rfc_operators() {
        let arguments = BTreeMap::from([
            ("id".into(), "private/file?x=1".into()),
            ("query".into(), "a&b".into()),
        ]);
        assert_eq!(
            expand("notes://catalog/{id}{?query}", &arguments).unwrap(),
            "notes://catalog/private%2Ffile%3Fx%3D1?query=a%26b"
        );
        assert_eq!(
            expand(
                "notes://catalog/{id:7}/{id:7}",
                &BTreeMap::from([("id".into(), "private-more".into())])
            )
            .unwrap(),
            "notes://catalog/private/private"
        );
        assert_eq!(
            expand(
                "notes://catalog/{+path}",
                &BTreeMap::from([("path".into(), "folder/item".into())])
            )
            .unwrap(),
            "notes://catalog/folder/item"
        );
        assert!(expand("notes://catalog/{id}", &arguments).is_err());
        assert!(expand("notes://catalog/{broken", &BTreeMap::new()).is_err());
        assert!(expand(
            &format!("notes://catalog/{}", "{id}".repeat(100)),
            &BTreeMap::from([("id".into(), "x".repeat(1000))])
        )
        .is_err());
    }
}
