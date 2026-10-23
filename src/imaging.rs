//! The cropped selection and the conversions the OCR engines need.

use crate::error::{Error, Result};

/// RGB, 8 bits per channel, rows top to bottom without padding. Screen captures
/// have no meaningful alpha, so it is not stored.
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
/// shrinking, bilinear when growing.
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
                let (x0, y0) = (fx as u32, fy as u32);
                let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
                let (wx, wy) = (fx - x0 as f32, fy - y0 as f32);
                for c in 0..channels {
                    let top = at(x0, y0, c) + (at(x1, y0, c) - at(x0, y0, c)) * wx;
                    let bottom = at(x0, y1, c) + (at(x1, y1, c) - at(x0, y1, c)) * wx;
                    out[base + c] = (top + (bottom - top) * wy).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    out
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
}
