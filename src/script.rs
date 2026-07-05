//! The diary's handwriting: a Hershey single-stroke script font turned into
//! pen paths. Single-stroke fonts are exactly what a moving pen produces,
//! so the injected reply genuinely looks written rather than printed.
//!
//! Font data: Hershey "script simplex" (public domain, ASCII 32..127) in
//! JHF format. Each record: 5-char glyph id, 3-char vertex count, then
//! coordinate pairs encoded as (char - 'R'); the pair " R" lifts the pen.
//! Records may wrap across lines, so we parse the flattened stream.

use anyhow::{bail, Result};

const FONT_DATA: &str = include_str!("../assets/scripts.jhf");
/// Hershey glyphs are roughly -16..+16 units tall; this maps font units to
/// pixels for a given nominal size.
const UNITS_PER_EM: f32 = 32.0;

#[derive(Debug, Clone, Default)]
pub struct Glyph {
    left: f32,
    right: f32,
    /// Polylines in font units, y down, x relative to glyph center.
    polylines: Vec<Vec<(f32, f32)>>,
}

impl Glyph {
    fn advance(&self) -> f32 {
        self.right - self.left
    }
}

pub struct Script {
    glyphs: Vec<Glyph>, // indexed by (char as usize - 32), 96 entries
}

impl Script {
    pub fn load() -> Result<Self> {
        let chars: Vec<char> = FONT_DATA
            .chars()
            .filter(|c| *c != '\n' && *c != '\r')
            .collect();
        let mut glyphs = Vec::new();
        let mut i = 0usize;
        while i + 8 <= chars.len() {
            // 5-char glyph id (unused) + 3-char vertex count.
            let count_str: String = chars[i + 5..i + 8].iter().collect();
            let n: usize = match count_str.trim().parse() {
                Ok(n) => n,
                Err(_) => bail!("bad JHF record at offset {i}: count {count_str:?}"),
            };
            i += 8;
            if i + 2 * n > chars.len() {
                bail!("truncated JHF record at offset {i}");
            }
            let mut glyph = Glyph::default();
            let mut current: Vec<(f32, f32)> = Vec::new();
            for p in 0..n {
                let cx = chars[i + 2 * p];
                let cy = chars[i + 2 * p + 1];
                if p == 0 {
                    glyph.left = (cx as i32 - 'R' as i32) as f32;
                    glyph.right = (cy as i32 - 'R' as i32) as f32;
                } else if cx == ' ' && cy == 'R' {
                    if current.len() > 1 {
                        glyph.polylines.push(std::mem::take(&mut current));
                    } else {
                        current.clear();
                    }
                } else {
                    current.push((
                        (cx as i32 - 'R' as i32) as f32,
                        (cy as i32 - 'R' as i32) as f32,
                    ));
                }
            }
            if current.len() > 1 {
                glyph.polylines.push(current);
            }
            i += 2 * n;
            glyphs.push(glyph);
        }
        if glyphs.len() < 96 {
            bail!("expected 96 glyphs in script font, found {}", glyphs.len());
        }
        Ok(Script { glyphs })
    }

    fn glyph(&self, c: char) -> &Glyph {
        let idx = (c as usize).wrapping_sub(32);
        if idx < 96 {
            &self.glyphs[idx]
        } else {
            &self.glyphs['?' as usize - 32]
        }
    }

    fn word_width(&self, word: &str, scale: f32) -> f32 {
        word.chars().map(|c| self.glyph(c).advance() * scale).sum()
    }

    /// Lay text out as screen-space polylines, word-wrapped to `max_width`,
    /// with the top-left of the text block at `origin`. Returns the pen
    /// paths in writing order (left to right, line by line).
    pub fn layout(
        &self,
        text: &str,
        size_px: f32,
        origin: (f32, f32),
        max_width: f32,
    ) -> Vec<Vec<(f32, f32)>> {
        let text = fold_to_ascii(text);
        let scale = size_px / UNITS_PER_EM;
        let line_height = size_px * 1.35;
        // Hershey y=0 is the writing baseline's center; push down so the
        // block starts below `origin.y`.
        let baseline = origin.1 + size_px * 0.6;
        let space_advance = self.glyph(' ').advance() * scale;

        let mut polys: Vec<Vec<(f32, f32)>> = Vec::new();
        let mut cursor_x = 0.0f32;
        let mut line = 0usize;

        for word in text.split_whitespace() {
            let w = self.word_width(word, scale);
            if cursor_x > 0.0 && cursor_x + w > max_width {
                cursor_x = 0.0;
                line += 1;
            }
            for c in word.chars() {
                let g = self.glyph(c);
                let ox = origin.0 + cursor_x - g.left * scale;
                let oy = baseline + line as f32 * line_height;
                for poly in &g.polylines {
                    polys.push(
                        poly.iter()
                            .map(|&(x, y)| (ox + x * scale, oy + y * scale))
                            .collect(),
                    );
                }
                cursor_x += g.advance() * scale;
            }
            cursor_x += space_advance;
        }
        chain_cursive(polys, size_px)
    }
}

/// Join consecutive strokes into cursive runs. Script glyphs carry entry
/// and exit tails near the baseline; when one stroke ends where the next
/// begins, the pen glides on without lifting — real joined-up handwriting,
/// and far fewer lift/touch pauses when the reply is written on the page.
/// Dots on i's, t-crosses and word gaps stay separate strokes: they start
/// too far above the previous exit, or too far to the right.
fn chain_cursive(polys: Vec<Vec<(f32, f32)>>, size_px: f32) -> Vec<Vec<(f32, f32)>> {
    // dy is strict: glyph strokes that start away from the baseline (a
    // t-cross, the top of an s) must stay separate — a connector drawn to
    // them slashes straight through the letter.
    let max_dx = size_px * 0.40;
    let max_dy = size_px * 0.12;
    let ink_len = |p: &[(f32, f32)]| -> f32 {
        p.windows(2)
            .map(|w| {
                let (dx, dy) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
                (dx * dx + dy * dy).sqrt()
            })
            .sum()
    };
    let mut out: Vec<Vec<(f32, f32)>> = Vec::new();
    for poly in polys {
        if poly.len() < 2 {
            continue;
        }
        // Periods, commas and other specks stay separate taps — a chained
        // connector would give them a visible tail.
        if ink_len(&poly) >= size_px * 0.22 {
            if let Some(prev) = out.last_mut() {
            let (ex, ey) = *prev.last().unwrap();
                let (sx, sy) = poly[0];
                let forward = sx - ex > -size_px * 0.10;
                if forward && sx - ex < max_dx && (sy - ey).abs() < max_dy {
                    prev.extend(poly);
                    continue;
                }
            }
        }
        out.push(poly);
    }
    out
}

/// The quill only knows ASCII: fold accents and typographic punctuation so
/// Dutch (and friends) still render.
pub fn fold_to_ascii(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => out.push('a'),
            'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' => out.push('A'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'É' | 'È' | 'Ê' | 'Ë' => out.push('E'),
            'í' | 'ì' | 'î' | 'ï' => out.push('i'),
            'Í' | 'Ì' | 'Î' | 'Ï' => out.push('I'),
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' => out.push('o'),
            'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' | 'Ø' => out.push('O'),
            'ú' | 'ù' | 'û' | 'ü' => out.push('u'),
            'Ú' | 'Ù' | 'Û' | 'Ü' => out.push('U'),
            'ý' | 'ÿ' => out.push('y'),
            'ç' => out.push('c'),
            'Ç' => out.push('C'),
            'ñ' => out.push('n'),
            'Ñ' => out.push('N'),
            'ß' => out.push_str("ss"),
            '\u{2018}' | '\u{2019}' => out.push('\''),
            '\u{201C}' | '\u{201D}' => out.push('"'),
            '\u{2013}' | '\u{2014}' => out.push('-'),
            '\u{2026}' => out.push_str("..."),
            c if c.is_ascii() => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}
