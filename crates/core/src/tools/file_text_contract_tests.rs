//! End-to-end byte contracts shared by all supported host platforms.
use super::{
    create_file_tool::CreateFileTool, edit_file_tool::EditFileTool, multi_edit_tool::MultiEditTool,
    Tool, ToolExecutionContext,
};
use crate::{db::Database, sources::CreateSourceInput};
use serde_json::json;

fn database(root: &std::path::Path) -> Database {
    let db = Database::open_memory().unwrap();
    db.add_source(CreateSourceInput {
        root_path: root.to_string_lossy().into_owned(),
        include_globs: vec![],
        exclude_globs: vec![],
        watch_enabled: false,
    })
    .unwrap();
    db
}

async fn execute(tool: &dyn Tool, db: &Database, args: serde_json::Value) {
    let arguments =
        super::normalize_tool_arguments(tool.name(), &args.to_string(), &tool.parameters_schema())
            .unwrap();
    let result = tool
        .execute(ToolExecutionContext::new(
            "byte-contract",
            &arguments,
            db,
            &[],
        ))
        .await
        .unwrap();
    assert!(!result.is_error, "{}: {}", tool.name(), result.content);
}

#[tokio::test]
async fn create_append_and_edits_preserve_literal_escapes_and_utf8_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path());
    let path = dir.path().join("中文 space [1].txt");
    let chunks = [
        "first\nsecond\r\n",
        r"C:\new\test\file.txt \\server\share",
        "\n中文🙂\t",
        r#"literal \n \r \t \u4e2d \" $HOME `echo` $(value) %PATH%"#,
    ];
    let mut expected = String::new();
    for (index, chunk) in chunks.iter().enumerate() {
        execute(
            &CreateFileTool,
            &db,
            json!({"path": path, "content": chunk,
            "mode": if index == 0 { "create" } else { "append" },
            "expected_bytes": expected.len()}),
        )
        .await;
        expected.push_str(chunk);
        assert_eq!(std::fs::read(&path).unwrap(), expected.as_bytes());
    }
    execute(
        &EditFileTool,
        &db,
        json!({"path": path, "old_str": "first", "new_str": r"literal\nfirst"}),
    )
    .await;
    expected = expected.replacen("first", r"literal\nfirst", 1);
    assert_eq!(std::fs::read(&path).unwrap(), expected.as_bytes());
    execute(
        &MultiEditTool,
        &db,
        json!({"path": path, "edits": [
            {"old_str": "second", "new_str": "two\nlines"},
            {"old_str": "中文🙂", "new_str": r"中文🙂\t"}
        ]}),
    )
    .await;
    expected = expected
        .replacen("second", "two\nlines", 1)
        .replacen("中文🙂", r"中文🙂\t", 1);
    assert_eq!(std::fs::read(&path).unwrap(), expected.as_bytes());
}

fn encoded(text: &str, encoding: &str) -> Vec<u8> {
    match encoding {
        "utf8-bom" => [b"\xef\xbb\xbf".as_slice(), text.as_bytes()].concat(),
        "utf16-le" => [
            vec![0xff, 0xfe],
            text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        ]
        .concat(),
        "utf16-be" => [
            vec![0xfe, 0xff],
            text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        ]
        .concat(),
        _ => text.as_bytes().to_vec(),
    }
}

#[tokio::test]
async fn append_refuses_mixed_encoding_and_accepts_utf8_across_read_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path());
    let path = dir.path().join("append.txt");
    let bytes = encoded("before\r\n", "utf16-le");
    std::fs::write(&path, &bytes).unwrap();
    let args = json!({"path": path, "mode": "append", "expected_bytes": bytes.len(), "content": "after\n"});
    let result = CreateFileTool
        .execute(ToolExecutionContext::new(
            "append",
            &args.to_string(),
            &db,
            &[],
        ))
        .await
        .unwrap();
    assert!(result.is_error);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let text = format!("{}🙂end", "a".repeat(65539));
    std::fs::write(&path, text.as_bytes()).unwrap();
    execute(
        &CreateFileTool,
        &db,
        json!({"path": path, "mode": "append", "expected_bytes": text.len(), "content": "\nnext"}),
    )
    .await;
    assert_eq!(
        std::fs::read(&path).unwrap(),
        format!("{text}\nnext").as_bytes()
    );
}

#[tokio::test]
async fn write_note_preserves_literals_and_refuses_mixed_encoding() {
    use super::write_note_tool::WriteNoteTool;
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path());
    execute(
        &WriteNoteTool,
        &db,
        json!({"filename":"literal.md", "content":"中文\n\\n"}),
    )
    .await;
    execute(
        &WriteNoteTool,
        &db,
        json!({"filename":"literal.md", "content":"\r\n\\t", "mode":"append"}),
    )
    .await;
    let path = dir.path().join("notes/literal.md");
    assert_eq!(std::fs::read(&path).unwrap(), "中文\n\\n\r\n\\t".as_bytes());
    let original = encoded("before", "utf16-le");
    std::fs::write(&path, &original).unwrap();
    let result = WriteNoteTool
        .execute(ToolExecutionContext::new(
            "append-note",
            &json!({"filename":"literal.md", "content":"after", "mode":"append"}).to_string(),
            &db,
            &[],
        ))
        .await
        .unwrap();
    assert!(result.is_error);
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[tokio::test]
async fn malformed_text_encoding_is_rejected_without_modification() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path());
    for bytes in [
        vec![0xff, 0xfe, 0x41],
        vec![0xff, 0xfe, 0x00, 0xd8],
        vec![0x80, 0x81],
        vec![0, b'a'],
    ] {
        for tool in [&EditFileTool as &dyn Tool, &MultiEditTool as &dyn Tool] {
            let path = dir.path().join("invalid.txt");
            std::fs::write(&path, &bytes).unwrap();
            let args = if tool.name() == "multi_edit" {
                json!({"path": path, "edits": [{"old_str": "a", "new_str": "b"}]})
            } else {
                json!({"path": path, "old_str": "a", "new_str": "b"})
            };
            let result = tool
                .execute(ToolExecutionContext::new(
                    "invalid",
                    &args.to_string(),
                    &db,
                    &[],
                ))
                .await
                .unwrap();
            assert!(result.is_error);
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
}

#[tokio::test]
async fn edits_preserve_bom_and_utf16_encoding() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path());
    for encoding in ["utf8", "utf8-bom", "utf16-le", "utf16-be"] {
        for tool in [&EditFileTool as &dyn Tool, &MultiEditTool as &dyn Tool] {
            let path = dir.path().join(format!("{encoding}-{}.txt", tool.name()));
            let original = "before\r\n中文🙂 literal \\n\r\nafter\r\n";
            std::fs::write(&path, encoded(original, encoding)).unwrap();
            let edit = json!({"old_str": "before", "new_str": "updated"});
            let args = if tool.name() == "multi_edit" {
                json!({"path": path, "edits": [edit]})
            } else {
                json!({"path": path, "old_str": "before", "new_str": "updated"})
            };
            execute(tool, &db, args).await;
            assert_eq!(
                std::fs::read(&path).unwrap(),
                encoded(&original.replacen("before", "updated", 1), encoding),
                "{encoding}: {}",
                tool.name()
            );
        }
    }
}
