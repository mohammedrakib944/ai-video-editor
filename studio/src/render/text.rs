//! Renders text layers to bitmaps with cosmic-text (full shaping, so Bangla
//! conjuncts render correctly), using the same fonts the app bundles.

use std::sync::{Arc, Mutex, OnceLock};

use iced::advanced::graphics::text::cosmic_text::{self as ct, Attrs, Buffer, Family, Metrics, Shaping, SwashCache, Weight};
use iced::advanced::graphics::text::font_system;

use super::Frame;
use crate::clip::{Align, TextLayer};

fn swash_cache() -> &'static Mutex<SwashCache> {
    static CACHE: OnceLock<Mutex<SwashCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(SwashCache::new()))
}

/// Cache key: everything that affects the bitmap.
pub fn key(layer: &TextLayer, px_per_unit: f32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    layer.text.hash(&mut h);
    layer.font.hash(&mut h);
    layer.weight.hash(&mut h);
    for v in [layer.size, layer.line_height, layer.letter_spacing, layer.padding, px_per_unit as f64] {
        v.to_bits().hash(&mut h);
    }
    for c in [Some(layer.color), layer.background].into_iter().flatten() {
        for v in [c.0, c.1, c.2, c.3] {
            v.to_bits().hash(&mut h);
        }
    }
    (layer.align as u8).hash(&mut h);
    layer.background.is_some().hash(&mut h);
    h.finish()
}

pub fn rasterize(layer: &TextLayer, px_per_unit: f32) -> Arc<Frame> {
    let size = (layer.size as f32 * px_per_unit).max(1.0);
    let metrics = Metrics::new(size, size * layer.line_height as f32);
    let attrs = Attrs::new()
        .family(Family::Name(&layer.font))
        .weight(Weight(layer.weight))
        .letter_spacing(layer.letter_spacing as f32 / layer.size.max(1.0) as f32);
    let align = match layer.align {
        Align::Left => ct::Align::Left,
        Align::Center => ct::Align::Center,
        Align::Right => ct::Align::Right,
    };
    let text = if layer.text.is_empty() { " " } else { layer.text.as_str() };

    let mut fs_guard = font_system().write().expect("font system");
    let fs = fs_guard.raw();
    let mut buffer = Buffer::new(fs, metrics);
    buffer.set_size(fs, None, None);
    buffer.set_text(fs, text, &attrs, Shaping::Advanced, Some(align));
    buffer.shape_until_scroll(fs, false);
    // Measure, then lay out again at that width so alignment works across lines.
    let width = buffer.layout_runs().map(|r| r.line_w).fold(0.0f32, f32::max).ceil().max(1.0);
    buffer.set_size(fs, Some(width), None);
    buffer.shape_until_scroll(fs, false);
    let height = buffer.layout_runs().map(|r| r.line_top + r.line_height).fold(0.0f32, f32::max).ceil().max(1.0);

    let pad = if layer.background.is_some() { (layer.padding as f32 * px_per_unit).round() } else { (size * 0.08).ceil() + 2.0 };
    let (w, h) = ((width + pad * 2.0) as u32, (height + pad * 2.0) as u32);
    let mut rgba = vec![0u8; (w * h * 4) as usize];

    if let Some(bg) = layer.background {
        fill_rounded(&mut rgba, w, h, (pad * 0.7).min(h as f32 / 2.0), [bg.0, bg.1, bg.2, bg.3]);
    }

    let c = layer.color;
    let color = ct::Color::rgba((c.0 * 255.0) as u8, (c.1 * 255.0) as u8, (c.2 * 255.0) as u8, (c.3 * 255.0) as u8);
    let mut cache = swash_cache().lock().expect("swash cache");
    let (ox, oy) = (pad as i32, pad as i32);
    buffer.draw(fs, &mut cache, color, |x, y, gw, gh, col| {
        let a = col.a() as f32 / 255.0;
        if a <= 0.0 {
            return;
        }
        for yy in 0..gh as i32 {
            for xx in 0..gw as i32 {
                let (px, py) = (x + xx + ox, y + yy + oy);
                if px < 0 || py < 0 || px >= w as i32 || py >= h as i32 {
                    continue;
                }
                let i = ((py as u32 * w + px as u32) * 4) as usize;
                blend(&mut rgba[i..i + 4], [col.r(), col.g(), col.b()], a);
            }
        }
    });
    Arc::new(Frame::new(w, h, rgba))
}

/// Source-over blend of one color onto a straight-alpha pixel.
fn blend(dst: &mut [u8], rgb: [u8; 3], a: f32) {
    let da = dst[3] as f32 / 255.0;
    let out_a = a + da * (1.0 - a);
    if out_a <= 0.0 {
        return;
    }
    for k in 0..3 {
        let s = rgb[k] as f32;
        let d = dst[k] as f32;
        dst[k] = ((s * a + d * da * (1.0 - a)) / out_a).round().clamp(0.0, 255.0) as u8;
    }
    dst[3] = (out_a * 255.0).round() as u8;
}

fn fill_rounded(rgba: &mut [u8], w: u32, h: u32, r: f32, c: [f32; 4]) {
    let rgb = [(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8];
    for y in 0..h {
        for x in 0..w {
            // Signed distance to the rounded rectangle, for an anti-aliased edge.
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let qx = (px - w as f32 / 2.0).abs() - (w as f32 / 2.0 - r);
            let qy = (py - h as f32 / 2.0).abs() - (h as f32 / 2.0 - r);
            let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
            let cover = (0.5 - d).clamp(0.0, 1.0);
            if cover > 0.0 {
                let i = ((y * w + x) * 4) as usize;
                blend(&mut rgba[i..i + 4], rgb, c[3] * cover);
            }
        }
    }
}
