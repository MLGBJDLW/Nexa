//! Explicit GitHub.com read through the user's existing gh authentication.
use super::invalid;
use crate::error::CoreError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub name: String,
    pub state: String,
    pub url: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    pub path: String,
    pub line: Option<u64>,
    pub outdated: bool,
    pub body: String,
    pub url: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub url: String,
    pub title: String,
    pub state: String,
    pub draft: bool,
    pub head_sha: String,
    pub base_sha: String,
    pub review_decision: Option<String>,
    pub observed_at: String,
    pub checks: Vec<Check>,
    pub unresolved_threads: Vec<Thread>,
    pub partial: bool,
}
fn parse_url(input: &str) -> Result<(String, String, u64), CoreError> {
    let url = reqwest::Url::parse(input).map_err(|_| invalid("Enter a GitHub pull request URL"))?;
    let parts = url
        .path()
        .trim_end_matches('/')
        .split('/')
        .collect::<Vec<_>>();
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || parts.len() != 5
        || parts[3] != "pull"
        || !parts[1..3].iter().all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        })
    {
        return Err(invalid(
            "Use https://github.com/owner/repository/pull/number",
        ));
    }
    let number = parts[4]
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0 && *n <= i32::MAX as u64)
        .ok_or_else(|| invalid("Invalid pull request number"))?;
    Ok((parts[1].into(), parts[2].into(), number))
}
const QUERY: &str = r#"query($owner:String!,$repo:String!,$number:Int!) {
  repository(owner:$owner,name:$repo) { pullRequest(number:$number) {
    url title state isDraft headRefOid baseRefOid reviewDecision
    commits(last:1) { nodes { commit { oid statusCheckRollup { contexts(first:100) {
      pageInfo { hasNextPage } nodes { __typename
        ... on CheckRun { name status conclusion detailsUrl }
        ... on StatusContext { context state targetUrl }
      }
    } } } } }
    reviewThreads(first:100) { pageInfo { hasNextPage } nodes {
      id isResolved isOutdated path line comments(last:1) { nodes { body url } }
    } }
  } }
}"#;

fn string(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().into()
}
fn optional(value: &Value, key: &str) -> Option<String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
fn parse(value: Value, expected_url: &str) -> Result<PullRequest, CoreError> {
    if value
        .get("errors")
        .is_some_and(|errors| !errors.as_array().is_some_and(Vec::is_empty))
    {
        return Err(invalid("GitHub could not return a complete PR observation; check gh authentication and repository access"));
    }
    let pr = &value["data"]["repository"]["pullRequest"];
    let head_sha = string(pr, "headRefOid");
    let base_sha = string(pr, "baseRefOid");
    if head_sha.len() != 40 || base_sha.len() != 40 || string(pr, "url") != expected_url {
        return Err(invalid(
            "GitHub PR response did not match the requested identity",
        ));
    }
    let commit = &pr["commits"]["nodes"][0]["commit"];
    if string(commit, "oid") != head_sha {
        return Err(invalid(
            "The PR head changed during observation; refresh again",
        ));
    }
    let contexts = &commit["statusCheckRollup"]["contexts"];
    let checks = contexts["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|node| {
            if node["__typename"] == "CheckRun" {
                Check {
                    name: string(node, "name"),
                    state: optional(node, "conclusion").unwrap_or_else(|| string(node, "status")),
                    url: optional(node, "detailsUrl"),
                }
            } else {
                Check {
                    name: string(node, "context"),
                    state: string(node, "state"),
                    url: optional(node, "targetUrl"),
                }
            }
        })
        .collect();
    let threads = &pr["reviewThreads"];
    let unresolved_threads = threads["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|node| node["isResolved"] != true)
        .map(|node| {
            let comment = &node["comments"]["nodes"][0];
            Thread {
                id: string(node, "id"),
                path: string(node, "path"),
                line: node["line"].as_u64(),
                outdated: node["isOutdated"] == true,
                body: string(comment, "body"),
                url: optional(comment, "url"),
            }
        })
        .collect();
    Ok(PullRequest {
        url: expected_url.into(),
        title: string(pr, "title"),
        state: string(pr, "state"),
        draft: pr["isDraft"] == true,
        head_sha,
        base_sha,
        review_decision: optional(pr, "reviewDecision"),
        observed_at: chrono::Utc::now().to_rfc3339(),
        checks,
        unresolved_threads,
        partial: contexts["pageInfo"]["hasNextPage"] == true
            || threads["pageInfo"]["hasNextPage"] == true,
    })
}
pub async fn read(url: &str) -> Result<PullRequest, CoreError> {
    let (owner, repo, number) = parse_url(url)?;
    let canonical = format!("https://github.com/{owner}/{repo}/pull/{number}");
    let payload = serde_json::to_vec(
        &json!({"query":QUERY,"variables":{"owner":owner,"repo":repo,"number":number}}),
    )?;
    let mut command = Command::new("gh");
    command
        .args(["api", "graphql", "--hostname", "github.com", "--input", "-"])
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GH_DEBUG")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn().map_err(|_| {
        invalid(
            "GitHub CLI is unavailable. Install gh and sign in using gh auth login, then refresh.",
        )
    })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| invalid("GitHub CLI input unavailable"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| invalid("GitHub CLI output unavailable"))?
        .take(2 * 1024 * 1024 + 1);
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| invalid("GitHub CLI diagnostics unavailable"))?
        .take(64 * 1024 + 1);
    let mut bytes = Vec::new();
    let mut errors = Vec::new();
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        tokio::try_join!(
            async {
                stdin.write_all(&payload).await?;
                drop(stdin);
                Ok::<_, std::io::Error>(())
            },
            async { stdout.read_to_end(&mut bytes).await.map(|_| ()) },
            async { stderr.read_to_end(&mut errors).await.map(|_| ()) },
            async {
                let status = child.wait().await?;
                Ok::<_, std::io::Error>(status)
            }
        )
    })
    .await;
    let success = match result {
        Ok(Ok((_, _, _, status))) => status.success(),
        _ => {
            let _ = child.kill().await;
            return Err(invalid(
                "GitHub observation timed out or was interrupted; refresh to retry",
            ));
        }
    };
    if !success {
        return Err(invalid(
            "GitHub PR read failed. Verify gh auth status and access to this repository.",
        ));
    }
    if bytes.len() > 2 * 1024 * 1024 || errors.len() > 64 * 1024 {
        return Err(invalid("GitHub response exceeds the observation limit"));
    }
    parse(serde_json::from_slice(&bytes)?, &canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_ambiguous_hosts_and_keeps_checks_bound_to_head() {
        assert!(parse_url("https://github.com/owner/repo/pull/42").is_ok());
        for url in [
            "https://github.com.evil.test/o/r/pull/1",
            "https://user@github.com/o/r/pull/1",
            "https://github.com/o/r/pull/1?x=1",
            "http://github.com/o/r/pull/1",
        ] {
            assert!(parse_url(url).is_err());
        }
        let sha = "a".repeat(40);
        let value = json!({"data":{"repository":{"pullRequest":{"url":"https://github.com/o/r/pull/1","headRefOid":sha,"baseRefOid":"b".repeat(40),"commits":{"nodes":[{"commit":{"oid":"c".repeat(40)}}]}}}}});
        assert!(parse(value, "https://github.com/o/r/pull/1").is_err());
    }
    #[tokio::test]
    #[ignore = "read-only live GitHub integration; requires gh authentication"]
    async fn live_pr_read() {
        let pr = read("https://github.com/MLGBJDLW/Nexa/pull/442")
            .await
            .unwrap();
        assert_eq!(pr.head_sha.len(), 40);
        assert!(!pr.title.is_empty());
    }
}
