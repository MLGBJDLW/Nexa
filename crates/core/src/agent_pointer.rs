//! One Computer Use pointer design and resolved theme for native and browser input.
use std::sync::atomic::{AtomicU32, Ordering};

pub const SIZE: f64 = 36.0;
pub const HOTSPOT: f64 = 8.0;

// Neutral only until the main renderer projects the current CSS theme.
static APPEARANCE: AtomicU32 = AtomicU32::new(0x0080_8080);

pub fn configure(accent: [u8; 3], reduced_motion: bool) {
    APPEARANCE.store(
        u32::from_be_bytes([u8::from(reduced_motion), accent[0], accent[1], accent[2]]),
        Ordering::Release,
    );
}

pub fn accent() -> [u8; 3] {
    let bytes = APPEARANCE.load(Ordering::Acquire).to_be_bytes();
    [bytes[1], bytes[2], bytes[3]]
}

pub fn reduced_motion() -> bool {
    APPEARANCE.load(Ordering::Acquire) >> 24 != 0
}

fn rounded_triangle(vertices: [(f64, f64); 3], corner: f64) -> Vec<(f64, f64)> {
    let mut outline = Vec::with_capacity(21);
    for index in 0..3 {
        let vertex = vertices[index];
        let inset = |neighbor: (f64, f64)| {
            let dx = neighbor.0 - vertex.0;
            let dy = neighbor.1 - vertex.1;
            let fraction = (corner / dx.hypot(dy)).min(0.4);
            (vertex.0 + dx * fraction, vertex.1 + dy * fraction)
        };
        let from = inset(vertices[(index + 2) % 3]);
        let to = inset(vertices[(index + 1) % 3]);
        for step in 0..=6 {
            let t = f64::from(step) / 6.0;
            let u = 1.0 - t;
            outline.push((
                u * u * from.0 + 2.0 * u * t * vertex.0 + t * t * to.0,
                u * u * from.1 + 2.0 * u * t * vertex.1 + t * t * to.1,
            ));
        }
    }
    outline
}

pub fn outer_outline() -> Vec<(f64, f64)> {
    rounded_triangle([(8.0, 8.0), (11.0, 28.0), (26.0, 21.0)], 3.4)
}

pub fn inner_outline() -> Vec<(f64, f64)> {
    rounded_triangle([(9.3, 10.2), (11.8, 26.6), (24.0, 20.8)], 2.8)
}

/// SVG uses the same polygon samples and gradient as the native BGRA rasterizer.
pub fn browser_appearance() -> serde_json::Value {
    let color = accent();
    let points = |outline: Vec<(f64, f64)>| {
        outline
            .into_iter()
            .map(|(x, y)| format!("{x:.4},{y:.4}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let outer = points(outer_outline());
    let inner = points(inner_outline());
    let css = |rgb: [u8; 3]| format!("rgb({} {} {})", rgb[0], rgb[1], rgb[2]);
    let light =
        css(color.map(|channel| (f64::from(channel) + (255.0 - f64::from(channel)) * 0.22) as u8));
    let dark = css(color.map(|channel| (f64::from(channel) * 0.88) as u8));
    let shadow = css(color.map(|channel| channel / 4));
    let svg = format!(
        r#"<svg viewBox="0 0 36 36" width="36" height="36" xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="nexa-pointer-fill" gradientUnits="userSpaceOnUse" x1="0" y1="10" x2="0" y2="28"><stop stop-color="{light}"/><stop offset="1" stop-color="{dark}"/></linearGradient></defs><polygon points="{outer}" transform="translate(.5 1)" fill="{shadow}" opacity=".2745"/><polygon points="{outer}" fill="white"/><polygon points="{inner}" fill="url(#nexa-pointer-fill)"/></svg>"#
    );
    serde_json::json!({ "svg": svg, "size": SIZE, "hotspot": HOTSPOT, "accent": css(color), "reducedMotion": reduced_motion() })
}
