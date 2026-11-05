use quick_xml::{events::Event, Reader};

use crate::error::CoreError;

pub(super) fn validate(bytes: &[u8]) -> Result<(), CoreError> {
    const ELEMENTS: &[&[u8]] = &[
        b"svg",
        b"g",
        b"defs",
        b"path",
        b"rect",
        b"circle",
        b"ellipse",
        b"line",
        b"polyline",
        b"polygon",
        b"linearGradient",
        b"radialGradient",
        b"stop",
        b"clipPath",
        b"mask",
        b"symbol",
        b"use",
        b"title",
        b"desc",
    ];
    let invalid = || CoreError::Parse("Image provider returned unsupported or unsafe SVG.".into());
    if bytes.len() > 2 * 1024 * 1024 || std::str::from_utf8(bytes).is_err() {
        return Err(invalid());
    }
    let mut xml = Reader::from_reader(bytes);
    let mut depth = 0usize;
    let mut seen_root = false;
    let mut references = 0;
    loop {
        let event = xml.read_event().map_err(|_| invalid())?;
        match event {
            Event::Start(ref tag) | Event::Empty(ref tag) => {
                if !ELEMENTS.contains(&tag.name().as_ref()) || depth >= 64 {
                    return Err(invalid());
                }
                if depth == 0 {
                    if seen_root || tag.name().as_ref() != b"svg" {
                        return Err(invalid());
                    }
                    seen_root = true;
                }
                for attr in tag.attributes() {
                    let attr = attr.map_err(|_| invalid())?;
                    let key = attr.key.as_ref();
                    let value = attr
                        .decoded_and_normalized_value(
                            quick_xml::XmlVersion::Implicit1_0,
                            xml.decoder(),
                        )
                        .map_err(|_| invalid())?;
                    match key {
                        b"xmlns" if value == "http://www.w3.org/2000/svg" => {}
                        b"xmlns:xlink" if value == "http://www.w3.org/1999/xlink" => {}
                        b"href" | b"xlink:href"
                            if fragment(&value) && tag.name().as_ref() != b"use" => {}
                        b"fill" | b"stroke" | b"clip-path" | b"mask"
                            if value.starts_with("url(") =>
                        {
                            if !value
                                .strip_prefix("url(")
                                .and_then(|v| v.strip_suffix(')'))
                                .is_some_and(fragment)
                            {
                                return Err(invalid());
                            }
                            references += 1;
                            if references > 2048 {
                                return Err(invalid());
                            }
                        }
                        b"id"
                        | b"version"
                        | b"viewBox"
                        | b"width"
                        | b"height"
                        | b"x"
                        | b"y"
                        | b"x1"
                        | b"y1"
                        | b"x2"
                        | b"y2"
                        | b"cx"
                        | b"cy"
                        | b"r"
                        | b"rx"
                        | b"ry"
                        | b"fx"
                        | b"fy"
                        | b"fr"
                        | b"points"
                        | b"d"
                        | b"transform"
                        | b"fill"
                        | b"fill-rule"
                        | b"fill-opacity"
                        | b"stroke"
                        | b"stroke-width"
                        | b"stroke-linecap"
                        | b"stroke-linejoin"
                        | b"stroke-miterlimit"
                        | b"stroke-opacity"
                        | b"stroke-dasharray"
                        | b"stroke-dashoffset"
                        | b"clip-path"
                        | b"clip-rule"
                        | b"clipPathUnits"
                        | b"mask"
                        | b"maskUnits"
                        | b"maskContentUnits"
                        | b"opacity"
                        | b"offset"
                        | b"stop-color"
                        | b"stop-opacity"
                        | b"gradientUnits"
                        | b"gradientTransform"
                        | b"spreadMethod"
                        | b"preserveAspectRatio"
                        | b"vector-effect" => {
                            if value.contains(['\\', '<', '&'])
                                || value.to_ascii_lowercase().contains("url")
                            {
                                return Err(invalid());
                            }
                        }
                        _ => return Err(invalid()),
                    }
                }
                if matches!(event, Event::Start(_)) {
                    depth += 1;
                }
            }
            Event::End(_) => depth = depth.checked_sub(1).ok_or_else(invalid)?,
            Event::Text(text) if depth > 0 || text.iter().all(u8::is_ascii_whitespace) => {}
            Event::Decl(decl)
                if !seen_root
                    && decl.encoding().is_none_or(|encoding| {
                        encoding.is_ok_and(|value| value.eq_ignore_ascii_case(b"utf-8"))
                    }) => {}
            Event::Comment(_) => {}
            Event::Eof if seen_root && depth == 0 => return Ok(()),
            _ => return Err(invalid()),
        }
    }
}

fn fragment(value: &str) -> bool {
    value.strip_prefix('#').is_some_and(|id| {
        !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    })
}

#[cfg(test)]
mod tests {
    use super::validate;

    #[test]
    fn keeps_editable_vector_paths_and_rejects_active_or_external_content() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><defs><linearGradient id="a"><stop offset="1" stop-color="#fff"/></linearGradient></defs><path d="M0 0L20 20" fill="url(#a)"/></svg>"##;
        assert!(validate(svg).is_ok());
        for payload in [
            "<script>alert(1)</script>",
            "<path onload='alert(1)'/>",
            "<path fill='url(https://example.com/remote.svg#x)'/>",
            "<path style='fill:url(https://example.com)'/>",
            "<use href='https://example.com/test.svg#x'/>",
            "<foreignObject><p>hi</p></foreignObject>",
        ] {
            assert!(validate(format!("<svg>{payload}</svg>").as_bytes()).is_err());
        }
        assert!(validate(b"<!DOCTYPE svg><svg/>").is_err());
        assert!(validate(b"<svg/><svg/>").is_err());
    }
}
