use async_trait::async_trait;

use super::{
    blocked_by_challenge, fetch_search_html, parse_results, ParserConfig, SearchProvider,
    SearchProviderContext,
};
use crate::web_search::model::{
    SearchEngine, SearchProviderFailure, SearchRequest, SearchResultItem,
};

pub struct BingProvider;

const RESULT_SELECTORS: &[&str] = &["li.b_algo", ".b_algo"];
const TITLE_SELECTORS: &[&str] = &["h2 a[href]", "a[href]"];
const SNIPPET_SELECTORS: &[&str] = &[".b_caption p", ".b_snippet", "p"];
const CAPTCHA_NEEDLES: &[&str] = &["unusual traffic", "captcha", "verify you are human"];

pub(crate) fn parse_bing_results(html: &str) -> Vec<SearchResultItem> {
    parse_results(
        html,
        ParserConfig {
            engine: SearchEngine::Bing,
            base_url: "https://www.bing.com/",
            result_selectors: RESULT_SELECTORS,
            title_selectors: TITLE_SELECTORS,
            snippet_selectors: SNIPPET_SELECTORS,
        },
    )
}

#[async_trait]
impl SearchProvider for BingProvider {
    fn engine(&self) -> SearchEngine {
        SearchEngine::Bing
    }

    async fn search(
        &self,
        request: &SearchRequest,
        ctx: &SearchProviderContext<'_>,
    ) -> Result<Vec<SearchResultItem>, SearchProviderFailure> {
        let mut url = reqwest::Url::parse("https://www.bing.com/search").map_err(|e| {
            SearchProviderFailure::new(SearchEngine::Bing, "invalid_provider_url", e.to_string())
        })?;
        url.query_pairs_mut()
            .append_pair("q", &request.effective_query)
            .append_pair("count", &request.limit.min(10).to_string());

        let html = fetch_search_html(
            ctx.client,
            SearchEngine::Bing,
            url,
            Some("https://www.bing.com/"),
            "en-US,en;q=0.9,zh-CN;q=0.6",
        )
        .await?;
        if blocked_by_challenge(&html, CAPTCHA_NEEDLES) {
            return Err(SearchProviderFailure::new(
                SearchEngine::Bing,
                "captcha",
                "Bing returned a verification page",
            ));
        }
        Ok(parse_bing_results(&html))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bing_fixture() {
        let html = include_str!("fixtures/bing_normal.html");

        let results = parse_bing_results(html);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Tauri Documentation");
        assert_eq!(results[0].source, "tauri.app");
        assert_eq!(results[0].engine, SearchEngine::Bing);
    }

    #[test]
    fn detects_bing_challenge_fixture() {
        let html = include_str!("fixtures/bing_captcha.html");

        assert!(blocked_by_challenge(html, CAPTCHA_NEEDLES));
    }

    #[test]
    fn reads_bing_destinations_without_network_and_keeps_distinct_pages() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        let links = [
            "https://example.org/view?id=A",
            "https://example.org/view?id=B",
        ];
        let html: String = links.iter().map(|url| format!(
            "<li class='b_algo'><h2><a href='https://www.bing.com/ck/a?u=a1{}'>Document</a></h2></li>",
            URL_SAFE_NO_PAD.encode(url)
        )).collect();
        let results = parse_bing_results(&html);
        assert_eq!(
            results
                .iter()
                .map(|result| result.url.as_str())
                .collect::<Vec<_>>(),
            links
        );
        assert!(results.iter().all(|result| result.resolved));
    }
}
