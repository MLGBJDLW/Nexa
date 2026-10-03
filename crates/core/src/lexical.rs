//! Versioned, deterministic bilingual FTS projection. Raw evidence stays in
//! `chunks`; FTS sees the same projection on insert, delete, rebuild and read.
use rusqlite::{functions::FunctionFlags, Connection};

fn is_han(ch: char) -> bool {
    matches!(ch as u32, 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x323af)
}

fn han_runs(text: &str) -> impl Iterator<Item = &str> {
    text.split(|ch| !is_han(ch)).filter(|run| !run.is_empty())
}

fn bigrams(run: &str) -> Vec<String> {
    let chars = run.chars().collect::<Vec<_>>();
    chars.windows(2).map(|pair| pair.iter().collect()).collect()
}

pub(crate) fn index_text(text: &str) -> String {
    // unicode61 treats adjacent Han/Latin letters as one word. Separate script
    // boundaries so identifiers such as PKCE and AUTH-204 remain searchable in
    // Chinese prose, while retaining the ordinary Latin punctuation tokenizer.
    let mut result = String::with_capacity(text.len());
    let mut previous = None;
    for ch in text.chars() {
        if previous.is_some_and(|han| han != is_han(ch)) {
            result.push(' ');
        }
        result.push(ch);
        previous = Some(is_han(ch));
    }
    for run in han_runs(text) {
        // A boundary token prevents phrase matches spanning separate runs.
        result.push_str("\n nxcjkboundary ");
        result.push_str(&bigrams(run).join(" "));
        result.push_str(" nxcjkboundary ");
        for ch in run.chars() {
            result.push(ch);
            result.push(' ');
        }
    }
    result
}

pub(crate) fn register(conn: &Connection) -> rusqlite::Result<()> {
    conn.create_scalar_function(
        "nexa_lexical_v1",
        1,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |context| Ok(index_text(&context.get::<String>(0)?)),
    )
}

fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}

/// Preserve existing whitespace-OR and final-prefix behavior for Latin terms.
/// Inside one mixed-language token, every segment must match. Chinese phrases
/// match consecutive bigrams, including two-character terms; single Han terms
/// use the unigram lane instead of a trigram-only blind spot.
pub(crate) fn query(input: &str) -> String {
    let tokens = input.split_whitespace().collect::<Vec<_>>();
    let mut clauses = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let prefix = index + 1 == tokens.len() && token.ends_with('*');
        let token = if prefix {
            &token[..token.len() - 1]
        } else {
            token
        };
        if token.is_empty() {
            continue;
        }
        if !token.chars().any(is_han) {
            clauses.push(format!("{}{}", quote(token), if prefix { "*" } else { "" }));
            continue;
        }
        let mut segments = Vec::new();
        let mut start = 0;
        let mut previous_han = token.chars().next().is_some_and(is_han);
        for (offset, ch) in token.char_indices().skip(1) {
            if is_han(ch) != previous_han {
                segments.push((&token[start..offset], previous_han));
                start = offset;
                previous_han = is_han(ch);
            }
        }
        segments.push((&token[start..], previous_han));
        let terms = segments
            .into_iter()
            .filter_map(|(segment, han)| {
                if !han && !segment.chars().any(char::is_alphanumeric) {
                    return None;
                }
                let grams = if han { bigrams(segment) } else { Vec::new() };
                Some(if grams.is_empty() {
                    quote(segment)
                } else {
                    quote(&grams.join(" "))
                })
            })
            .collect::<Vec<_>>();
        if !terms.is_empty() {
            clauses.push(format!("({})", terms.join(" AND ")));
        }
    }
    clauses.join(" OR ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_queries_keep_phrases_prefixes_and_operators_literal() {
        assert_eq!(query("报销标准"), "(\"报销 销标 标准\")");
        assert_eq!(query("Qwen报销"), "(\"Qwen\" AND \"报销\")");
        assert_eq!(
            query("retry_guard ERR-429 backo*"),
            "\"retry_guard\" OR \"ERR-429\" OR \"backo\"*"
        );
        assert_eq!(query("a\"b OR *"), "\"a\"\"b\" OR \"OR\"");
    }
    #[test]
    fn latin_identifiers_remain_tokens_inside_continuous_chinese() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE fixture USING fts5(content, tokenize='unicode61')",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO fixture(content) VALUES(?1)",
            [index_text(
                "使用PKCE保护OAuth登录。错误代码AUTH-204表示令牌过期。",
            )],
        )
        .unwrap();
        for text in ["PKCE", "OAuth", "AUTH-204", "令牌过期", "PKCE保护"] {
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(*) FROM fixture WHERE fixture MATCH ?1",
                    [query(text)],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
                1,
                "{text}"
            );
        }
    }
}
