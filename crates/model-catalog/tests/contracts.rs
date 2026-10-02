use nexa_model_catalog::*;

fn model(id: &str) -> ModelDescriptor {
    let mut model = ModelDescriptor::new(id, "provider", id);
    model.endpoint_ids = vec!["text:one".into()];
    model
}

#[test]
fn discovery_is_scoped_and_cannot_resurrect_removed_aliases() {
    let mut removed = model("old");
    removed.aliases = vec!["legacy".into()];
    removed.lifecycle = ModelLifecycle::Removed;
    let curated = [model("known"), removed];
    let live = [
        DiscoveredModel::new("legacy", "text:one", ""),
        DiscoveredModel::new("new", "text:one", ""),
        DiscoveredModel::new("foreign", "text:two", ""),
    ];
    let snapshot = merge_catalog(CatalogMergeInput {
        provider_id: "provider",
        endpoint_id: "text:one",
        curated: &curated,
        discovered: Some(&live),
        probes: &[],
        refreshed_at: "now",
    });
    assert_eq!(
        snapshot
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ["known", "new"]
    );
    assert_eq!(snapshot.models[0].available_to_credential, Some(false));
    assert_eq!(
        snapshot.models[1].product_readiness,
        ProductReadiness::Discoverable
    );
    assert!(!snapshot.models[1].capabilities.vision);
    assert_eq!(snapshot.tombstones[0].id, "old");
}

#[test]
fn failed_refresh_does_not_replace_last_good_account_snapshot() {
    let key = CatalogCacheKey::new("provider", "text:one", "credential-a");
    let other = CatalogCacheKey::new("provider", "text:one", "credential-b");
    let curated = [model("known")];
    let mut cache = LastGoodCatalogCache::default();
    let success = merge_catalog(CatalogMergeInput {
        provider_id: "provider",
        endpoint_id: "text:one",
        curated: &curated,
        discovered: Some(&[]),
        probes: &[],
        refreshed_at: "success",
    });
    let failed = merge_catalog(CatalogMergeInput {
        provider_id: "provider",
        endpoint_id: "text:one",
        curated: &[],
        discovered: None,
        probes: &[],
        refreshed_at: "failed",
    });
    cache.store_if_success(key.clone(), success);
    cache.store_if_success(key.clone(), failed);
    assert_eq!(cache.get(&key).unwrap().refreshed_at, "success");
    assert!(cache.get(&other).is_none());
    assert!(!format!("{key}").contains("credential-a"));
}

#[test]
fn selection_does_not_borrow_an_alias_from_another_endpoint() {
    let mut descriptor = model("current");
    descriptor.aliases = vec!["alias".into()];
    let saved = SavedModelSelection::new("provider", Some("text:two".into()), "alias");
    assert_eq!(
        resolve_saved_selection(&saved, &[descriptor.clone()]).kind,
        SelectionResolutionKind::Unverified
    );
    let own = SavedModelSelection::new("provider", Some("text:one".into()), "alias");
    let resolved = resolve_saved_selection(&own, &[descriptor]);
    assert_eq!(resolved.kind, SelectionResolutionKind::Alias);
    assert_eq!(resolved.model_id, "current");
}

#[test]
fn documented_endpoint_lookup_does_not_trust_a_lookalike_or_private_path() {
    assert!(
        resolve_builtin_endpoint_id("text", "open_ai", Some("https://api.openai.com/v1")).is_some()
    );
    for route in [
        "http://api.openai.com/v1",
        "https://api.openai.com:8443/v1",
        "https://api.openai.com.evil.example/v1",
        "https://api.openai.com/private/v1",
    ] {
        assert!(
            resolve_builtin_endpoint_id("text", "open_ai", Some(route)).is_none(),
            "{route}"
        );
    }
}
