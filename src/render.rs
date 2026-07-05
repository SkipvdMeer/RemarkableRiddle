//! Renders stroke polylines to a PNG — this is what the vision model "sees"
//! instead of a framebuffer screenshot, which keeps us independent of
//! xochitl internals across firmware versions.

use crate::capture::Stroke;
use crate::geom::{wacom_to_screen, SCREEN_H, SCREEN_W};
use anyhow::{anyhow, Result};
use tiny_skia::{
    Color, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke as SkStroke, Transform,
};

/// Half-resolution keeps the image tokens cheap while staying very legible.
const RENDER_SCALE: f32 = 0.5;

/// Captured wacom-space strokes → screen-space polylines.
pub fn strokes_to_polys(strokes: &[Stroke]) -> Vec<Vec<(f32, f32)>> {
    strokes
        .iter()
        .map(|s| s.points.iter().map(|p| wacom_to_screen(p.x, p.y)).collect())
        .collect()
}

pub fn polylines_to_png(polys: &[Vec<(f32, f32)>]) -> Result<Vec<u8>> {
    let w = (SCREEN_W * RENDER_SCALE) as u32;
    let h = (SCREEN_H * RENDER_SCALE) as u32;
    let mut pixmap = Pixmap::new(w, h).ok_or_else(|| anyhow!("pixmap alloc failed"))?;
    pixmap.fill(Color::WHITE);

    let mut paint = Paint::default();
    paint.set_color_rgba8(20, 20, 30, 255);
    paint.anti_alias = true;

    let sk_stroke = SkStroke {
        width: 2.0,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Default::default()
    };

    for poly in polys {
        if poly.is_empty() {
            continue;
        }
        let mut pb = PathBuilder::new();
        pb.move_to(poly[0].0 * RENDER_SCALE, poly[0].1 * RENDER_SCALE);
        if poly.len() == 1 {
            // Single-point tap: a sub-pixel segment + round cap draws a dot.
            pb.line_to(poly[0].0 * RENDER_SCALE + 0.5, poly[0].1 * RENDER_SCALE);
        }
        for &(x, y) in &poly[1..] {
            pb.line_to(x * RENDER_SCALE, y * RENDER_SCALE);
        }
        if let Some(path) = pb.finish() {
            pixmap.stroke_path(&path, &paint, &sk_stroke, Transform::identity(), None);
        }
    }

    Ok(pixmap.encode_png()?)
}
