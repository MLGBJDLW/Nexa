//! One Copilot account/catalog route for Settings, Chat and execution.
use github_copilot_sdk::{CliProgram, ClientOptions};
use std::path::PathBuf;

pub(crate) fn client_options(binary: PathBuf) -> ClientOptions {
    // Copilot distinguishes editor and CLI model entitlements by integration.
    // Use the editor API contract (also used by pi/Hermes) throughout Nexa:
    // mixing a CLI catalog with editor requests drops models and effort levels.
    // The official CLI still owns authentication, inference and tool callbacks.
    ClientOptions::default()
        .with_program(CliProgram::Path(binary))
        .with_env([("GITHUB_COPILOT_INTEGRATION_ID", "vscode-chat")])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_catalog_and_execution_share_one_integration() {
        let options = client_options(PathBuf::from("copilot"));
        assert!(options.env.iter().any(|(key, value)| {
            key == "GITHUB_COPILOT_INTEGRATION_ID" && value == "vscode-chat"
        }));
        assert!(options.github_token.is_none(), "the CLI owns credentials");
    }
}
