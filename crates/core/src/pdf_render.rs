//! CPU-only page rasterization for OCR, including text converted to vector paths.
//! Kept inside the OCR feature; no external executable or platform DLL is needed.
use std::collections::BTreeSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};

use crate::error::CoreError;

pub(crate) struct PdfRenderer(Pdf);

pub(crate) struct RenderedPage {
    pub png: Vec<u8>,
    pub warnings: Vec<String>,
}

impl PdfRenderer {
    pub(crate) fn new(bytes: &[u8]) -> Result<Self, CoreError> {
        catch_unwind(AssertUnwindSafe(|| Pdf::new(bytes.to_vec())))
            .map_err(|_| CoreError::Parse("PDF rendering parser failed".into()))?
            .map(Self)
            .map_err(|error| CoreError::Parse(format!("PDF rendering parser: {error:?}")))
    }

    pub(crate) fn page_count(&self) -> usize {
        self.0.pages().len()
    }

    pub(crate) fn page(&self, index: usize) -> Result<RenderedPage, CoreError> {
        catch_unwind(AssertUnwindSafe(|| self.render_page(index))).map_err(|_| {
            CoreError::Parse("PDF page rendering failed; try a structured parser".into())
        })?
    }

    fn render_page(&self, index: usize) -> Result<RenderedPage, CoreError> {
        let page =
            self.0.pages().get(index).ok_or_else(|| {
                CoreError::Parse("PDF page mapping changed during rendering".into())
            })?;
        let (width, height) = page.render_dimensions();
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err(CoreError::Parse("PDF page has invalid dimensions".into()));
        }
        // 144 DPI, bounded to a 2048-pixel edge (at most 16 MiB RGBA).
        // The bound is applied before the renderer's internal u16 conversion.
        let scale = 2.0_f32.min(2048.0 / width.max(height));
        if width * scale < 1.0 || height * scale < 1.0 {
            return Err(CoreError::Parse(
                "PDF page aspect ratio cannot be rendered safely".into(),
            ));
        }
        let warnings = Arc::new(Mutex::new(BTreeSet::new()));
        let sink = Arc::clone(&warnings);
        let settings = InterpreterSettings {
            warning_sink: Arc::new(move |warning| {
                sink.lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .insert(format!("PDF rendering incomplete: {warning:?}"));
            }),
            ..Default::default()
        };
        // Release decoded images and glyph caches after each page, so a long
        // scanned document does not retain every full-resolution image in RAM.
        let pixmap = hayro::render(
            page,
            &RenderCache::new(),
            &settings,
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: WHITE,
            },
        );
        let png = pixmap
            .into_png()
            .map_err(|error| CoreError::Parse(format!("PDF raster encoding: {error}")))?;
        let warnings = warnings
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .cloned()
            .collect();
        Ok(RenderedPage { png, warnings })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Document, Object, Stream};

    #[test]
    fn vector_only_pages_are_rendered_on_white_with_distinct_page_geometry() {
        let mut document = Document::new();
        let pages = document.new_object_id();
        let mut kids: Vec<Object> = Vec::new();
        for content in [b"0 g 0 0 50 100 re f".as_slice(), b"0 g 50 0 50 100 re f"] {
            let stream = document.add_object(Stream::new(dictionary! {}, content.to_vec()));
            kids.push(document.add_object(dictionary! {"Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),100.into(),100.into()],"Contents"=>stream}).into());
        }
        document.objects.insert(
            pages,
            Object::Dictionary(dictionary! {"Type"=>"Pages","Kids"=>kids,"Count"=>2}),
        );
        let catalog = document.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
        document.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        let renderer = PdfRenderer::new(&bytes).unwrap();
        assert_eq!(renderer.page_count(), 2);
        for index in 0..2 {
            let rendered = renderer.page(index).unwrap();
            assert!(rendered.warnings.is_empty());
            let pixels = image::load_from_memory(&rendered.png).unwrap().to_rgba8();
            assert_eq!(pixels.dimensions(), (200, 200));
            assert_eq!(
                pixels.get_pixel(if index == 0 { 25 } else { 175 }, 100).0,
                [0, 0, 0, 255]
            );
            assert_eq!(
                pixels.get_pixel(if index == 0 { 175 } else { 25 }, 100).0,
                [255, 255, 255, 255]
            );
        }
    }
}
