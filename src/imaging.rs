//! The cropped selection and the conversions the OCR engines need.

use crate::error::{Error, Result};

/// RGB, 8 bits per channel, rows top to bottom without padding. Screen captures
/// have no meaningful alpha, so it is not stored.
#[derive(Clone)]
pub struct RgbImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl RgbImage {
    /// Copies a rectangle out of a larger RGB buffer. The rectangle must lie inside it.
    pub fn crop_from(source: &[u8], source_width: u32, x: u32, y: u32, width: u32, height: u32) -> Self {
        let stride = source_width as usize * 3;
        let row_bytes = width as usize * 3;
        let mut pixels = Vec::with_capacity(row_bytes * height as usize);
        for row in y..y + height {
            let start = row as usize * stride + x as usize * 3;
            pixels.extend_from_slice(&source[start..start + row_bytes]);
        }
        Self { width, height, pixels }
    }

    pub fn resized(&self, width: u32, height: u32) -> Self {
        Self { width, height, pixels: resample(&self.pixels, self.width, self.height, 3, width, height) }
    }

    /// The image surrounded by `pad` pixels of its border color. Selections are drawn
    /// tight to the text, and glyphs touching the crop edge get cut; a margin colored
    /// like the background around them keeps the shapes intact for recognition.
    pub fn padded(&self, pad: u32) -> Self {
        if pad == 0 {
            return Self { width: self.width, height: self.height, pixels: self.pixels.clone() };
        }
        let [r, g, b] = self.border_color();
        let (width, height) = (self.width as usize, self.height as usize);
        let (new_width, new_height) = (width + pad as usize * 2, height + pad as usize * 2);
        let mut pixels = Vec::with_capacity(new_width * new_height * 3);
        let pad_row = [r, g, b].repeat(new_width);
        let pad_side = [r, g, b].repeat(pad as usize);
        for _ in 0..pad {
            pixels.extend_from_slice(&pad_row);
        }
        for row in self.pixels.chunks_exact(width * 3) {
            pixels.extend_from_slice(&pad_side);
            pixels.extend_from_slice(row);
            pixels.extend_from_slice(&pad_side);
        }
        for _ in 0..pad {
            pixels.extend_from_slice(&pad_row);
        }
        Self { width: new_width as u32, height: new_height as u32, pixels }
    }

    /// Average color of the outermost pixel frame: the background the selection was
    /// drawn on, even when a gradient or a corner glyph touches the edge.
    fn border_color(&self) -> [u8; 3] {
        let (w, h) = (self.width as usize, self.height as usize);
        let at = |x: usize, y: usize| (y * w + x) * 3;
        let mut sum = [0u64; 3];
        let mut samples = 0u64;
        let mut add = |i: usize| {
            for (channel, total) in sum.iter_mut().enumerate() {
                *total += self.pixels[i + channel] as u64;
            }
            samples += 1;
        };
        for x in 0..w {
            add(at(x, 0));
            add(at(x, h - 1));
        }
        for y in 0..h {
            add(at(0, y));
            add(at(w - 1, y));
        }
        let avg = samples.max(1);
        [(sum[0] / avg) as u8, (sum[1] / avg) as u8, (sum[2] / avg) as u8]
    }

    pub fn encode_png(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, self.width, self.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().map_err(|e| Error::new(format!("PNG encoding failed: {e}")))?;
        writer.write_image_data(&self.pixels).map_err(|e| Error::new(format!("PNG encoding failed: {e}")))?;
        writer.finish().map_err(|e| Error::new(format!("PNG encoding failed: {e}")))?;
        Ok(out)
    }
}

/// Resamples interleaved 8-bit pixels with `channels` channels: box average when
/// shrinking, Catmull-Rom bicubic when growing. The cubic kernel is sharp enough to
/// keep upscaled glyphs legible where bilinear washed them out.
pub fn resample(source: &[u8], width: u32, height: u32, channels: usize, new_width: u32, new_height: u32) -> Vec<u8> {
    if (new_width, new_height) == (width, height) {
        return source.to_vec();
    }
    let at = |x: u32, y: u32, c: usize| source[(y as usize * width as usize + x as usize) * channels + c] as f32;
    let sx = width as f32 / new_width as f32;
    let sy = height as f32 / new_height as f32;
    let shrinking = sx > 1.0 || sy > 1.0;
    let mut out = vec![0u8; new_width as usize * new_height as usize * channels];
    for ty in 0..new_height {
        for tx in 0..new_width {
            let base = (ty as usize * new_width as usize + tx as usize) * channels;
            if shrinking {
                let x0 = (tx as f32 * sx) as u32;
                let y0 = (ty as f32 * sy) as u32;
                let x1 = (((tx + 1) as f32 * sx).ceil() as u32).clamp(x0 + 1, width);
                let y1 = (((ty + 1) as f32 * sy).ceil() as u32).clamp(y0 + 1, height);
                let n = ((x1 - x0) * (y1 - y0)) as f32;
                for c in 0..channels {
                    let mut sum = 0.0;
                    for y in y0..y1 {
                        for x in x0..x1 {
                            sum += at(x, y, c);
                        }
                    }
                    out[base + c] = (sum / n).round() as u8;
                }
            } else {
                let fx = ((tx as f32 + 0.5) * sx - 0.5).max(0.0);
                let fy = ((ty as f32 + 0.5) * sy - 0.5).max(0.0);
                let (x0, y0) = (fx.floor() as i32, fy.floor() as i32);
                for c in 0..channels {
                    let mut sum = 0.0;
                    for dy in -1..=2 {
                        let y = (y0 + dy).clamp(0, height as i32 - 1) as u32;
                        let wy = cubic(fy - y0 as f32 - dy as f32);
                        for dx in -1..=2 {
                            let x = (x0 + dx).clamp(0, width as i32 - 1) as u32;
                            sum += at(x, y, c) * cubic(fx - x0 as f32 - dx as f32) * wy;
                        }
                    }
                    out[base + c] = sum.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    out
}

/// Catmull-Rom kernel (B = 0, C = 0.5): interpolating, so a uniform image stays
/// uniform, with just enough negative lobes to keep edges crisp.
fn cubic(t: f32) -> f32 {
    let t = t.abs();
    if t <= 1.0 {
        ((1.5 * t - 2.5) * t) * t + 1.0
    } else if t < 2.0 {
        ((-0.5 * t + 2.5) * t - 4.0) * t + 2.0
    } else {
        0.0
    }
}

/// Size that fits within `max_side` on both axes, keeping the aspect ratio.
pub fn fit_within(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= max_side {
        return (width, height);
    }
    let scale = max_side as f64 / longest as f64;
    (((width as f64 * scale).round() as u32).max(1), ((height as f64 * scale).round() as u32).max(1))
}

pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32) -> RgbImage {
        let pixels = (0..width * height).flat_map(|i| [i as u8, (i * 2) as u8, (i * 3) as u8]).collect();
        RgbImage { width, height, pixels }
    }

    /// A white image with a dark block, standing in for a tight text selection: the
    /// block must stay where it is through padding and upscaling, and the margins
    /// must be uniform on all four sides. Sheared rows (a padded buffer shorter
    /// than width × height) fail the right-margin check.
    #[test]
    fn padding_and_upscale_keep_the_content_in_place() {
        let (w, h, block) = (60u32, 20u32, 8u32);
        let mut pixels = vec![250u8; (w * h * 3) as usize];
        for y in 6..6 + block {
            for x in 20..20 + block {
                pixels[((y * w + x) * 3) as usize..][..3].fill(25);
            }
        }
        let crop = RgbImage { width: w, height: h, pixels };

        let padded = crop.padded(4);
        assert_eq!((padded.width, padded.height), (w + 8, h + 8));
        let px = |img: &RgbImage, x: u32, y: u32| img.pixels[((y * img.width + x) * 3) as usize];
        for y in 0..padded.height {
            for x in 0..padded.width {
                let on_margin = x < 4 || x >= padded.width - 4 || y < 4 || y >= padded.height - 4;
                if on_margin {
                    assert_eq!(px(&padded, x, y), 250, "margin at {x},{y}");
                }
            }
        }
        assert_eq!(px(&padded, 24, 10), 25, "the block moved with the pad");

        let up = padded.resized(padded.width * 3, padded.height * 3);
        for y in 0..up.height {
            for x in 0..up.width {
                let on_margin = x < 12 || x >= up.width - 12 || y < 12 || y >= up.height - 12;
                if on_margin {
                    assert_eq!(px(&up, x, y), 250, "upscaled margin at {x},{y}");
                }
            }
        }
        // The block's heart is still dark after 3x; cubic edges may ring, its center may not.
        assert!(px(&up, 24 * 3 + block * 3 / 2, 10 * 3 + block * 3 / 2) < 60);
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn crops_the_requested_rectangle() {
        let source = image(4, 3);
        let crop = RgbImage::crop_from(&source.pixels, 4, 1, 1, 2, 2);
        assert_eq!((crop.width, crop.height), (2, 2));
        assert_eq!(&crop.pixels[0..3], &source.pixels[(4 + 1) * 3..(4 + 1) * 3 + 3]);
        assert_eq!(&crop.pixels[9..12], &source.pixels[(2 * 4 + 2) * 3..(2 * 4 + 2) * 3 + 3]);
    }

    #[test]
    fn resizes_both_ways() {
        let source = image(10, 6);
        let up = source.resized(30, 18);
        assert_eq!(up.pixels.len(), 30 * 18 * 3);
        let down = source.resized(5, 3);
        assert_eq!(down.pixels.len(), 5 * 3 * 3);
    }

    #[test]
    fn fits_within_longest_side() {
        assert_eq!(fit_within(4000, 1000, 2000), (2000, 500));
        assert_eq!(fit_within(800, 600, 2000), (800, 600));
        assert_eq!(fit_within(1, 5000, 100), (1, 100));
    }

    #[test]
    fn encodes_png() {
        let png = image(3, 2).encode_png().unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn resamples_single_channel() {
        let gray = vec![0u8, 255, 0, 255];
        assert_eq!(resample(&gray, 2, 2, 1, 1, 1), vec![128]);
        assert_eq!(resample(&gray, 2, 2, 1, 4, 4).len(), 16);
    }

    #[test]
    fn upscale_keeps_a_uniform_image_uniform() {
        let flat = vec![90u8; 4 * 4];
        assert!(resample(&flat, 4, 4, 1, 9, 9).iter().all(|&v| v == 90));
    }

    #[test]
    fn pads_a_uniform_image_with_its_own_color() {
        let image = RgbImage { width: 3, height: 2, pixels: vec![7; 18] };
        let padded = image.padded(2);
        assert_eq!((padded.width, padded.height), (7, 6));
        // A short buffer would mean sheared rows: every row must be full width.
        assert_eq!(padded.pixels.len(), 7 * 6 * 3);
        assert!(padded.pixels.chunks_exact(3).all(|px| px == [7, 7, 7]));
    }

    #[test]
    fn pad_color_is_the_average_of_the_border() {
        // Top row of 8s over a row of 0s: the border holds four 8s and four 0s, so
        // the margin color is 4 and the original pixels sit offset by the pad.
        let mut pixels = vec![0u8; 2 * 2 * 3];
        pixels[0..3].fill(8);
        pixels[3..6].fill(8);
        let image = RgbImage { width: 2, height: 2, pixels };
        let padded = image.padded(1);
        assert_eq!(padded.pixels.len(), 4 * 4 * 3);
        assert_eq!(&padded.pixels[0..3], &[4, 4, 4]);
        let top_left = (padded.width as usize + 1) * 3;
        assert_eq!(&padded.pixels[top_left..top_left + 3], &[8, 8, 8]);
        // The right pad of the same content row, and the bottom pad row.
        assert_eq!(&padded.pixels[(padded.width as usize * 2 - 1) * 3..][0..3], &[4, 4, 4]);
        assert_eq!(&padded.pixels[padded.width as usize * 3 * 3..][0..3], &[4, 4, 4]);
    }
}
