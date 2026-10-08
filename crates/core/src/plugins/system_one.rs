use super::{
    CapabilityCheckSeverity, CapabilityPackageView, CapabilityProviderCatalog,
    CapabilityRuntimeCheck, CapabilityRuntimeStatus,
};

pub(super) fn enrich_manifest(
    mut view: CapabilityPackageView,
    config: Option<&crate::system_one::SystemOneConfig>,
) -> CapabilityPackageView {
    view.provider_catalogs = vec![CapabilityProviderCatalog {
        id: "systemOneProviders".into(),
        label: "Structured decision providers".into(),
        item_kind: "providerPreset".into(),
        items: serde_json::from_str(include_str!(
            "../../../../shared/system-one-provider-presets.json"
        ))
        .expect("System One catalog"),
    }];
    let (status, message) = match config {
        None => (CapabilityRuntimeStatus::Unknown, "Structured decision settings have not been loaded."),
        Some(config) if !config.enabled => (CapabilityRuntimeStatus::Warning, "Structured decisions are disabled. Enable a provider in Settings before invoking the tool."),
        Some(config) if config.is_configured() => (CapabilityRuntimeStatus::Pass, "Provider, credential and model are configured. Account access and inference have not been probed."),
        Some(_) => (CapabilityRuntimeStatus::Error, "Configure the provider credential, model and correct regional endpoint."),
    };
    view.runtime_checks = vec![CapabilityRuntimeCheck {
        id: "configuration".into(),
        label: "Decision provider configuration".into(),
        status,
        severity: CapabilityCheckSeverity::Warning,
        message: message.into(),
    }];
    view
}
