//! The Windows notification-area icon (F16, UI Design Guidelines §6): the leaf glyph in
//! the taskbar's colour with the stale count drawn on it. macOS uses a template image and
//! puts the count beside it as the status item's title; Windows has no title, so the
//! count goes into the icon. Pure pixel work on RGBA buffers, tested on every platform.

/// The icon's side in pixels: the notification area shows 16 px at 100 % scaling and 32 px
/// at 200 %; Windows scales this one down for the smaller sizes.
pub const SIZE: u32 = 32;

/// The glyph on a light taskbar (near-black, like the system icons) and on a dark one.
const INK_LIGHT: [u8; 3] = [0x1d, 0x22, 0x19];
const INK_DARK: [u8; 3] = [0xff, 0xff, 0xff];
/// The badge: Decay's accent with white figures; danger red for "!" (the engine failed).
const BADGE: [u8; 3] = [0x4f, 0x8a, 0x4b];
const BADGE_ALERT: [u8; 3] = [0xa5, 0x47, 0x31];
const FIGURE: [u8; 3] = [0xff, 0xff, 0xff];

/// A square RGBA image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    pub side: u32,
    pub pixels: Vec<u8>,
}

impl Rgba {
    fn idx(&self, x: u32, y: u32) -> usize {
        ((y * self.side + x) * 4) as usize
    }

    /// The alpha at a pixel.
    #[cfg(test)]
    pub fn alpha(&self, x: u32, y: u32) -> u8 {
        self.pixels[self.idx(x, y) + 3]
    }

    /// The colour at a pixel.
    #[cfg(test)]
    pub fn rgb(&self, x: u32, y: u32) -> [u8; 3] {
        let i = self.idx(x, y);
        [self.pixels[i], self.pixels[i + 1], self.pixels[i + 2]]
    }
}

/// The glyph's coverage (its alpha channel) resampled from `src_side` to `side` by area
/// averaging (good for shrinking a line glyph without losing its strokes).
fn coverage(src: &[u8], src_side: u32, side: u32) -> Vec<f32> {
    let ratio = f64::from(src_side) / f64::from(side);
    let mut out = Vec::with_capacity((side * side) as usize);
    for y in 0..side {
        for x in 0..side {
            let (x0, x1) = (f64::from(x) * ratio, f64::from(x + 1) * ratio);
            let (y0, y1) = (f64::from(y) * ratio, f64::from(y + 1) * ratio);
            let (mut sum, mut area) = (0.0_f64, 0.0_f64);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            for sy in (y0.floor() as u32)..(y1.ceil() as u32).min(src_side) {
                let h = (y1.min(f64::from(sy + 1)) - y0.max(f64::from(sy))).max(0.0);
                for sx in (x0.floor() as u32)..(x1.ceil() as u32).min(src_side) {
                    let w = (x1.min(f64::from(sx + 1)) - x0.max(f64::from(sx))).max(0.0);
                    let a = f64::from(src[((sy * src_side + sx) * 4 + 3) as usize]);
                    sum += a * w * h;
                    area += w * h;
                }
            }
            #[allow(clippy::cast_possible_truncation)]
            out.push(if area > 0.0 { (sum / area / 255.0) as f32 } else { 0.0 });
        }
    }
    out
}

/// The figures of a badge (3×5 bitmaps, rows top to bottom, bit 2 = left column).
fn glyph(c: char) -> Option<[u8; 5]> {
    Some(match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        '+' => [0b000, 0b010, 0b111, 0b010, 0b000],
        '!' => [0b010, 0b010, 0b010, 0b000, 0b010],
        _ => return None,
    })
}

/// Over-paints `rgb` at `(x, y)` with coverage `a` (0–1), straight alpha.
fn blend(img: &mut Rgba, x: u32, y: u32, rgb: [u8; 3], a: f32) {
    if x >= img.side || y >= img.side || a <= 0.0 {
        return;
    }
    let i = img.idx(x, y);
    let dst_a = f32::from(img.pixels[i + 3]) / 255.0;
    let out_a = a + dst_a * (1.0 - a);
    for (k, c) in rgb.iter().enumerate() {
        let src = f32::from(*c) * a;
        let dst = f32::from(img.pixels[i + k]) * dst_a * (1.0 - a);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            img.pixels[i + k] = ((src + dst) / out_a).round().clamp(0.0, 255.0) as u8;
        }
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        img.pixels[i + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
    }
}

/// Draws the badge: a pill in the bottom-right corner holding `text` (figures, "+", "!";
/// anything else is skipped), with a one-pixel transparent ring so it reads on the glyph.
fn draw_badge(img: &mut Rgba, text: &str) {
    let glyphs: Vec<[u8; 5]> = text.chars().filter_map(glyph).collect();
    if glyphs.is_empty() {
        return;
    }
    // Each bitmap pixel is 2×2; 2 px between figures.
    let scale = 2_u32;
    #[allow(clippy::cast_possible_truncation)]
    let count = glyphs.len() as u32;
    let text_w = count * 3 * scale + (count - 1) * scale;
    let text_h = 5 * scale;
    let pill_h = text_h + 6; // 3 px padding above and below
    let pill_w = (text_w + 6).max(pill_h).min(img.side);
    let (left, top) = (img.side - pill_w, img.side - pill_h);
    let radius = pill_h as f32 / 2.0;
    let fill = if text == "!" { BADGE_ALERT } else { BADGE };

    // Signed distance to the pill's edge, sampled at the pixel centre (anti-aliased).
    let dist = |x: u32, y: u32| {
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let cx = px.clamp(left as f32 + radius, (left + pill_w) as f32 - radius);
        let cy = top as f32 + radius;
        ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() - radius
    };
    for y in top.saturating_sub(1)..img.side {
        for x in left.saturating_sub(1)..img.side {
            let edge = dist(x, y);
            // Clear a 1-px ring around the pill, then paint the pill.
            let ring = (1.5 - edge).clamp(0.0, 1.0);
            if ring > 0.0 {
                let i = img.idx(x, y);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    img.pixels[i + 3] = (f32::from(img.pixels[i + 3]) * (1.0 - ring)) as u8;
                }
            }
            blend(img, x, y, fill, (0.5 - edge).clamp(0.0, 1.0));
        }
    }
    let (tx, ty) = (left + (pill_w - text_w) / 2, top + (pill_h - text_h) / 2);
    for (gi, g) in glyphs.iter().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let gx = tx + gi as u32 * 4 * scale;
        for (row, bits) in g.iter().enumerate() {
            for col in 0..3_u32 {
                if bits & (0b100 >> col) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        #[allow(clippy::cast_possible_truncation)]
                        let y = ty + row as u32 * scale + dy;
                        blend(img, gx + col * scale + dx, y, FIGURE, 1.0);
                    }
                }
            }
        }
    }
}

/// The icon for a taskbar: the template glyph (`template` RGBA, `template_side` square)
/// in the taskbar's ink at [`SIZE`] px, with `badge` drawn on it when there is one.
pub fn render(
    template: &[u8],
    template_side: u32,
    light_taskbar: bool,
    badge: Option<&str>,
) -> Rgba {
    let ink = if light_taskbar { INK_LIGHT } else { INK_DARK };
    let cov = coverage(template, template_side, SIZE);
    let mut img = Rgba { side: SIZE, pixels: Vec::with_capacity(cov.len() * 4) };
    for a in cov {
        img.pixels.extend_from_slice(&ink);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        img.pixels.push((a * 255.0).round().clamp(0.0, 255.0) as u8);
    }
    if let Some(text) = badge {
        draw_badge(&mut img, text);
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 44-px template: an opaque square (10..34), transparent elsewhere.
    fn template() -> Vec<u8> {
        let mut px = vec![0_u8; 44 * 44 * 4];
        for y in 10..34 {
            for x in 10..34 {
                px[(y * 44 + x) * 4 + 3] = 255;
            }
        }
        px
    }

    #[test]
    fn scales_the_glyph_and_colours_it_for_the_taskbar() {
        let dark = render(&template(), 44, false, None);
        assert_eq!(dark.side, SIZE);
        assert_eq!(dark.pixels.len(), (SIZE * SIZE * 4) as usize);
        assert_eq!(dark.alpha(16, 16), 255);
        assert_eq!(dark.alpha(1, 1), 0);
        assert_eq!(dark.rgb(16, 16), INK_DARK);
        let light = render(&template(), 44, true, None);
        assert_eq!(light.rgb(16, 16), INK_LIGHT);
        // Edges are anti-aliased, not dropped: 10/44 of 32 = 7.3, so pixel 7 is partly covered.
        let edge = render(&template(), 44, true, None).alpha(7, 16);
        assert!(edge > 0 && edge < 255, "edge alpha {edge}");
    }

    #[test]
    fn draws_the_count_in_the_corner() {
        let plain = render(&template(), 44, false, None);
        let one = render(&template(), 44, false, Some("7"));
        // The pill is opaque accent in the bottom-right corner, with white figures.
        assert_eq!(one.alpha(SIZE - 3, SIZE - 8), 255);
        assert_eq!(one.rgb(SIZE - 3, SIZE - 8), BADGE);
        assert!((0..SIZE).any(|x| (0..SIZE).any(|y| one.rgb(x, y) == FIGURE
            && one.alpha(x, y) == 255
            && x > SIZE / 2
            && y > SIZE / 2)));
        // The top-left stays as it was.
        assert_eq!(plain.alpha(16, 12), one.alpha(16, 12));
        assert_eq!(plain.rgb(4, 4), one.rgb(4, 4));
    }

    #[test]
    fn longer_counts_make_a_wider_pill() {
        let width = |text: &str| {
            let img = render(&vec![0; 44 * 44 * 4], 44, false, Some(text));
            (0..SIZE).filter(|x| img.alpha(*x, SIZE - 8) > 128).count()
        };
        assert!(width("7") < width("42"));
        assert!(width("42") < width("99+"));
        assert!(width("99+") <= SIZE as usize);
    }

    #[test]
    fn a_failure_badge_is_red_and_unknown_text_draws_nothing() {
        let alert = render(&template(), 44, true, Some("!"));
        assert_eq!(alert.rgb(SIZE - 3, SIZE - 8), BADGE_ALERT);
        let plain = render(&template(), 44, true, None);
        assert_eq!(render(&template(), 44, true, Some("?")), plain);
        assert_eq!(render(&template(), 44, true, Some("")), plain);
    }
}
