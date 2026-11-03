use super::*;
use crate::conversation::ImageAttachment;

pub struct ImageEditInputs {
    pub original: String,
    pub mask: String,
    pub guide: String,
}

/// Only the current turn's explicitly submitted edit is eligible. No history
/// scan, model-supplied URL, or other conversation can supply these pixels.
pub fn current_image_edit(
    context: &super::super::ToolExecutionContext<'_>,
) -> Result<ImageEditInputs, CoreError> {
    let conversation = context
        .conversation_id
        .ok_or_else(|| CoreError::InvalidInput("Image editing requires a conversation".into()))?;
    let turn = context
        .turn_id
        .ok_or_else(|| CoreError::InvalidInput("Image editing requires the current turn".into()))?;
    let (artifacts, attachments): (String, String) = context.db.conn().query_row(
        "SELECT m.artifacts_json, m.image_attachments_json FROM conversation_turns t JOIN messages m ON m.id=t.user_message_id WHERE t.id=?1 AND t.conversation_id=?2 AND m.conversation_id=?2 AND m.role='user'",
        rusqlite::params![turn, conversation], |row| Ok((row.get(0)?,row.get(1)?)),
    ).map_err(|_| CoreError::InvalidInput("This turn has no image edit selection. Open the image editor and submit a selection.".into()))?;
    let value: Value = serde_json::from_str(&artifacts)?;
    let edit = if value["kind"] == "chatSendContext" {
        &value["slashCommand"]
    } else {
        &value
    };
    if edit["kind"] != "imageEdit" || edit["version"] != 1 {
        return Err(CoreError::InvalidInput(
            "This turn is not an image edit request".into(),
        ));
    }
    let attachments: Vec<ImageAttachment> = serde_json::from_str(&attachments)?;
    let mut names = std::collections::HashSet::new();
    let mut total = 0usize;
    let mut images = Vec::new();
    for role in ["original", "mask", "guide"] {
        let name = edit[role]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| CoreError::InvalidInput(format!("Image edit is missing {role}")))?;
        if !names.insert(name) {
            return Err(CoreError::InvalidInput(
                "Image edit inputs must be distinct".into(),
            ));
        }
        let matches = attachments
            .iter()
            .filter(|image| image.original_name == name)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(CoreError::InvalidInput(format!(
                "Image edit {role} attachment is missing or ambiguous"
            )));
        }
        let image = matches[0];
        if image.base64_data.len() > crate::media::MAX_IMAGE_SIZE.div_ceil(3) * 4 {
            return Err(CoreError::InvalidInput(
                "Image edit attachment exceeds 12 MiB".into(),
            ));
        }
        let bytes = STANDARD
            .decode(&image.base64_data)
            .map_err(|_| CoreError::InvalidInput("Invalid image edit data".into()))?;
        total += bytes.len();
        if total > 32 * 1024 * 1024 {
            return Err(CoreError::InvalidInput(
                "Image edit inputs exceed 32 MiB".into(),
            ));
        }
        let format =
            image::guess_format(&bytes).map_err(|e| CoreError::InvalidInput(e.to_string()))?;
        if format != image::ImageFormat::Png {
            return Err(CoreError::InvalidInput(
                "Image editor inputs must be normalized PNG images".into(),
            ));
        }
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(crate::media::MAX_BROWSER_NATIVE_CAPTURE_EDGE);
        limits.max_image_height = Some(crate::media::MAX_BROWSER_NATIVE_CAPTURE_EDGE);
        limits.max_alloc = Some(crate::media::MAX_BROWSER_NATIVE_CAPTURE_PIXELS * 8);
        let mut reader = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format);
        reader.limits(limits);
        let decoded = reader
            .decode()
            .map_err(|e| CoreError::InvalidInput(format!("Invalid edit pixels: {e}")))?;
        crate::media::validate_browser_capture_dimensions(decoded.width(), decoded.height())?;
        if role == "mask" && !decoded.to_rgba8().pixels().any(|pixel| pixel[3] < 255) {
            return Err(CoreError::InvalidInput(
                "Select an area to edit; the mask has no transparent pixels".into(),
            ));
        }
        images.push((
            format!("data:image/png;base64,{}", image.base64_data),
            (decoded.width(), decoded.height()),
        ));
    }
    if images.iter().any(|(_, size)| *size != images[0].1) {
        return Err(CoreError::InvalidInput(
            "Image edit dimensions must match the original".into(),
        ));
    }
    let mut images = images.into_iter().map(|(url, _)| url);
    Ok(ImageEditInputs {
        original: images.next().unwrap(),
        mask: images.next().unwrap(),
        guide: images.next().unwrap(),
    })
}

pub const GUIDE_INSTRUCTION: &str = "The first input is the original image. The second is a selection guide: magenta marks the area to change. Apply the requested edit to that area, preserve the rest as closely as possible, and do not reproduce the magenta marking.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::CreateConversationInput;
    use crate::db::Database;
    use crate::tools::ToolExecutionContext;

    #[test]
    fn editor_inputs_are_bound_to_the_current_turn_and_keep_mask_alpha() {
        let db = Database::open_memory().unwrap();
        let conversation = db
            .create_conversation(&CreateConversationInput {
                provider: "open_ai".into(),
                model: "test".into(),
                system_prompt: None,
                collection_context: None,
                project_id: None,
                persona_id: None,
            })
            .unwrap();
        let mut pixels = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255]));
        pixels.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        pixels
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let encoded = STANDARD.encode(bytes.into_inner());
        let attachments = ["original", "mask", "guide"]
            .map(|name| json!({"base64Data":encoded,"mediaType":"image/png","originalName":name}));
        let artifacts = json!({"kind":"imageEdit","version":1,"original":"original","mask":"mask","guide":"guide"});
        db.conn().execute("INSERT INTO messages(id,conversation_id,role,content,artifacts_json,image_attachments_json) VALUES('edit-message',?1,'user','change selected pixels',?2,?3)", rusqlite::params![&conversation.id,artifacts.to_string(),json!(attachments).to_string()]).unwrap();
        let turn = db
            .create_conversation_turn(&conversation.id, "edit-message", None)
            .unwrap();
        let context = ToolExecutionContext::new("edit", "{}", &db, &[])
            .with_conversation_id(Some(&conversation.id))
            .with_turn_id(Some(&turn.id));
        let inputs = current_image_edit(&context).unwrap();
        assert_eq!(inputs.mask, format!("data:image/png;base64,{encoded}"));
        let wrong = ToolExecutionContext::new("edit", "{}", &db, &[])
            .with_conversation_id(Some("another-chat"))
            .with_turn_id(Some(&turn.id));
        assert!(current_image_edit(&wrong).is_err());
        db.conn()
            .execute(
                "UPDATE messages SET artifacts_json='{}' WHERE id='edit-message'",
                [],
            )
            .unwrap();
        assert!(current_image_edit(&context).is_err());
    }
}
