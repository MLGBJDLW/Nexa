//! Shared PDF image extraction for OCR and visual evidence. This layer has no
//! inference dependency so format compatibility can be tested without models.
use std::collections::HashSet;

use image::{DynamicImage, GrayImage, RgbImage};
use lopdf::{content::Content, Dictionary, Document, Object, ObjectId, Stream};

const MAX_IMAGE_PIXELS: usize = 32 * 1024 * 1024;

fn dictionary<'a>(doc: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    doc.dereference(object).ok()?.1.as_dict().ok()
}

pub(crate) fn extract_images_from_pdf_page(doc: &Document, page_id: ObjectId) -> Vec<DynamicImage> {
    let mut node = page_id;
    let mut parents = HashSet::new();
    let mut resources = None;
    // Resources are inherited from the nearest page-tree ancestor, not merged
    // from every ancestor (which could include another page's images).
    while parents.insert(node) {
        let Ok(page) = doc.get_dictionary(node) else {
            break;
        };
        if let Ok(value) = page.get(b"Resources") {
            resources = dictionary(doc, value);
            break;
        }
        let Ok(parent) = page.get(b"Parent").and_then(Object::as_reference) else {
            break;
        };
        node = parent;
    }
    let mut pending = resources
        .into_iter()
        .map(|resources| (doc.get_page_content(page_id), resources))
        .collect::<Vec<_>>();
    let mut visited = HashSet::new();
    let mut images = Vec::new();
    while let Some((content, resources)) = pending.pop() {
        let Some(xobjects) = resources
            .get(b"XObject")
            .ok()
            .and_then(|value| dictionary(doc, value))
        else {
            continue;
        };
        let Ok(content) = Content::decode(&content) else {
            continue;
        };
        // Resource dictionaries are lookup scopes. Only Do operations paint
        // an XObject on this page; shared, unused images belong to other pages.
        for operation in content
            .operations
            .iter()
            .filter(|operation| operation.operator == "Do")
        {
            let Some(name) = operation
                .operands
                .first()
                .and_then(|operand| operand.as_name().ok())
            else {
                continue;
            };
            let Ok(object) = xobjects.get(name) else {
                continue;
            };
            let Ok((id, object)) = doc.dereference(object) else {
                continue;
            };
            if id.is_some_and(|id| !visited.insert(id)) {
                continue;
            }
            let Ok(stream) = object.as_stream() else {
                continue;
            };
            match stream.dict.get(b"Subtype").and_then(Object::as_name).ok() {
                Some(b"Form") => {
                    let form_resources = stream
                        .dict
                        .get(b"Resources")
                        .ok()
                        .map(|value| dictionary(doc, value))
                        .unwrap_or(Some(resources));
                    if let Some(resources) = form_resources {
                        if let Ok(content) = stream.get_plain_content() {
                            pending.push((content, resources));
                        }
                    }
                }
                Some(b"Image") => {
                    if let Some(image) = decode_image(doc, stream) {
                        images.push(image);
                    }
                }
                _ => (),
            }
        }
    }
    images
}

fn color_components(doc: &Document, object: &Object) -> Option<usize> {
    let (_, object) = doc.dereference(object).ok()?;
    match object {
        Object::Name(name) => match name.as_slice() {
            b"DeviceGray" => Some(1),
            b"DeviceRGB" => Some(3),
            b"DeviceCMYK" => Some(4),
            _ => None,
        },
        Object::Array(values) if values.first()?.as_name().ok()? == b"ICCBased" => {
            let profile = doc.dereference(values.get(1)?).ok()?.1.as_stream().ok()?;
            usize::try_from(profile.dict.get(b"N").ok()?.as_i64().ok()?).ok()
        }
        _ => None,
    }
}

fn decode_image(doc: &Document, stream: &Stream) -> Option<DynamicImage> {
    let width = u32::try_from(stream.dict.get(b"Width").ok()?.as_i64().ok()?).ok()?;
    let height = u32::try_from(stream.dict.get(b"Height").ok()?.as_i64().ok()?).ok()?;
    let pixels = (width as usize).checked_mul(height as usize)?;
    if pixels == 0 || pixels > MAX_IMAGE_PIXELS {
        return None;
    }
    let filters = stream.filters().unwrap_or_default();
    if filters
        .last()
        .is_some_and(|filter| *filter == b"DCTDecode" || *filter == b"JPXDecode")
    {
        // image handles JPEG; unsupported JPEG2000 remains an explicit OCR
        // coverage gap rather than being interpreted as raw pixel bytes.
        return image::load_from_memory(&stream.content).ok();
    }
    let raw = if filters.is_empty() {
        stream.content.clone()
    } else {
        stream.decompressed_content().ok()?
    };
    let components = match stream.dict.get(b"ColorSpace") {
        Ok(value) => color_components(doc, value)?,
        Err(_)
            if stream
                .dict
                .get(b"ImageMask")
                .and_then(Object::as_bool)
                .unwrap_or(false) =>
        {
            1
        }
        Err(_) => return None,
    };
    let bpc = stream
        .dict
        .get(b"BitsPerComponent")
        .and_then(Object::as_i64)
        .unwrap_or(8);
    let decode = stream
        .dict
        .get(b"Decode")
        .ok()
        .and_then(|value| value.as_array().ok());
    let inverted = |component: usize| {
        decode.is_some_and(|values| {
        let number = |index: usize| values.get(index).and_then(|value| value.as_float().ok());
        matches!((number(component * 2), number(component * 2 + 1)), (Some(a), Some(b)) if a > b)
    })
    };
    if components == 1 && bpc == 1 {
        let stride = (width as usize).div_ceil(8);
        if raw.len() < stride.checked_mul(height as usize)? {
            return None;
        }
        let mut gray = Vec::with_capacity(pixels);
        for y in 0..height as usize {
            for x in 0..width as usize {
                let bit = raw[y * stride + x / 8] & (0x80 >> (x % 8)) != 0;
                gray.push(if bit ^ inverted(0) { 255 } else { 0 });
            }
        }
        return GrayImage::from_raw(width, height, gray).map(DynamicImage::ImageLuma8);
    }
    if bpc != 8 || !matches!(components, 1 | 3 | 4) || raw.len() < pixels.checked_mul(components)? {
        return None;
    }
    let channel = |value: u8, index| if inverted(index) { 255 - value } else { value };
    if components == 1 {
        let gray = raw[..pixels]
            .iter()
            .map(|value| channel(*value, 0))
            .collect();
        return GrayImage::from_raw(width, height, gray).map(DynamicImage::ImageLuma8);
    }
    let mut rgb = Vec::with_capacity(pixels * 3);
    for pixel in raw[..pixels * components].chunks_exact(components) {
        if components == 4 {
            let black = u16::from(channel(pixel[3], 3));
            for (index, ink) in pixel[..3].iter().enumerate() {
                rgb.push(((255 - u16::from(channel(*ink, index))) * (255 - black) / 255) as u8);
            }
        } else {
            rgb.extend(
                pixel
                    .iter()
                    .enumerate()
                    .map(|(index, value)| channel(*value, index)),
            );
        }
    }
    RgbImage::from_raw(width, height, rgb).map(DynamicImage::ImageRgb8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    #[test]
    #[ignore = "requires an explicitly supplied local PDF fixture"]
    fn supplied_scanned_pdf_has_decodable_images_on_every_page() {
        let path = std::env::var("NEXA_SCAN_FIXTURE_PATH").expect("set fixture path");
        let document = Document::load(path).unwrap();
        let counts: Vec<_> = document
            .get_pages()
            .values()
            .map(|id| extract_images_from_pdf_page(&document, *id).len())
            .collect();
        assert!(
            !counts.is_empty() && counts.iter().all(|count| *count > 0),
            "images per page: {counts:?}"
        );
        println!(
            "Verified {} pages; decoded images per page: {counts:?}",
            counts.len()
        );
    }

    #[test]
    fn inherited_nested_cmyk_images_are_decoded_once_even_with_form_cycles() {
        let mut doc = Document::new();
        let image = doc.add_object(Stream::new(dictionary! {"Subtype"=>"Image", "Width"=>2, "Height"=>1, "ColorSpace"=>"DeviceCMYK", "BitsPerComponent"=>8}, vec![0,0,0,0,0,0,0,255]));
        let form_id = doc.new_object_id();
        let resources =
            doc.add_object(dictionary! {"XObject"=>dictionary! {"Image"=>image,"Cycle"=>form_id}});
        doc.objects.insert(
            form_id,
            Object::Stream(Stream::new(
                dictionary! {"Subtype"=>"Form","Resources"=>resources},
                b"/Image Do /Cycle Do /Image Do".to_vec(),
            )),
        );
        let parent = doc.add_object(
            dictionary! {"Resources"=>dictionary! {"XObject"=>dictionary! {"Form"=>form_id}}},
        );
        let content = doc.add_object(Stream::new(dictionary! {}, b"/Form Do".to_vec()));
        let page = doc.add_object(dictionary! {"Parent"=>parent,"Contents"=>content});
        let images = extract_images_from_pdf_page(&doc, page);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].to_rgb8().into_raw(), [255, 255, 255, 0, 0, 0]);
    }

    #[test]
    fn shared_resources_only_extract_images_invoked_on_the_current_page() {
        let mut doc = Document::new();
        let a = doc.add_object(Stream::new(dictionary! {"Subtype"=>"Image","Width"=>1,"Height"=>1,"ColorSpace"=>"DeviceGray","BitsPerComponent"=>8}, vec![10]));
        let b = doc.add_object(Stream::new(dictionary! {"Subtype"=>"Image","Width"=>1,"Height"=>1,"ColorSpace"=>"DeviceGray","BitsPerComponent"=>8}, vec![200]));
        let parent = doc.add_object(
            dictionary! {"Resources"=>dictionary! {"XObject"=>dictionary! {"A"=>a,"B"=>b}}},
        );
        for (name, pixel) in [("A", 10), ("B", 200)] {
            let content = doc.add_object(Stream::new(
                dictionary! {},
                format!("/{name} Do").into_bytes(),
            ));
            let page = doc.add_object(dictionary! {"Parent"=>parent,"Contents"=>content});
            let images = extract_images_from_pdf_page(&doc, page);
            assert_eq!(images.len(), 1);
            assert_eq!(images[0].to_luma8().into_raw(), [pixel]);
        }
    }

    #[test]
    fn monochrome_rows_respect_padding_and_decode_inversion() {
        let doc = Document::new();
        let stream = Stream::new(
            dictionary! {"Width"=>3,"Height"=>2,"BitsPerComponent"=>1,"ColorSpace"=>"DeviceGray","Decode"=>vec![1.into(),0.into()]},
            vec![0b10100000, 0b01000000],
        );
        assert_eq!(
            decode_image(&doc, &stream).unwrap().to_luma8().into_raw(),
            [0, 255, 0, 255, 0, 255]
        );
    }
}
