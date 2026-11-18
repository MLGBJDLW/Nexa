use std::io::{Read, Seek};

use super::model::{PreviewBlock, StructuredPreview};
use crate::document_structure::{presentation_slide_text, presentation_slides};

/// Interactive previews keep a bounded overview. A citation reads its exact
/// slide directly from the archive, regardless of its position in the deck.
pub(super) fn preview_pptx(
    reader: impl Read + Seek,
    focus_slide: Option<u32>,
) -> Result<StructuredPreview, String> {
    let mut archive = zip::ZipArchive::new(reader).map_err(|error| error.to_string())?;
    let slides = presentation_slides(&mut archive).map_err(|error| error.to_string())?;
    let mut blocks = Vec::new();
    let start = focus_slide.unwrap_or(1).saturating_sub(1) as usize;
    let count = if focus_slide.is_some() { 1 } else { 80 };
    let mut remaining = 512 * 1024;
    let mut truncated = false;
    for (index, part) in slides.iter().enumerate().skip(start).take(count) {
        let mut text = String::new();
        let mut notes = String::new();
        presentation_slide_text(&mut archive, part, |source, is_notes| {
            let value = source.trim();
            let dest = if is_notes { &mut notes } else { &mut text };
            let end = value.floor_char_boundary(value.len().min(remaining));
            dest.push_str(&value[..end]);
            remaining -= end;
            truncated |= end < value.len();
        })
        .map_err(|error| error.to_string())?;
        blocks.push(PreviewBlock::Slide {
            number: index as u32 + 1,
            text,
            notes,
        });
        if remaining == 0 {
            truncated |= focus_slide.is_none() && index + 1 < slides.len();
            break;
        }
    }
    if truncated || (focus_slide.is_none() && slides.len() > count) {
        blocks.push(PreviewBlock::Unsupported {
            message: "This presentation preview is truncated. A slide citation opens its own text."
                .into(),
        });
    }
    Ok(StructuredPreview::Document {
        blocks,
        assets: Vec::new(),
    })
}
