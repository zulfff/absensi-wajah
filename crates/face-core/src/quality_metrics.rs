//! Quality metrics computed from pixel data.
//!
//! These produce the raw signals that [`domain::quality`] gates on. Kept as
//! plain functions over pixel buffers so they are unit-testable without any
//! model or image file.

/// Mean luminance (0.0..=255.0) using Rec. 601 luma weights.
pub fn mean_luminance(pixels: &[u8], channels: usize) -> f32 {
    if pixels.is_empty() || channels == 0 {
        return 0.0;
    }
    let mut sum = 0.0f64;
    let mut count = 0u64;
    for chunk in pixels.chunks_exact(channels) {
        let (r, g, b) = match channels {
            1 => (chunk[0] as f32, chunk[0] as f32, chunk[0] as f32),
            2 => (chunk[0] as f32, chunk[0] as f32, chunk[0] as f32),
            _ => (chunk[0] as f32, chunk[1] as f32, chunk[2] as f32),
        };
        let luma = 0.299 * r + 0.587 * g + 0.114 * b;
        sum += luma as f64;
        count += 1;
    }
    if count == 0 {
        0.0
    } else {
        (sum / count as f64) as f32
    }
}

/// Variance of the Laplacian, a standard sharpness proxy.
///
/// A greyscale image is convolved with the 4-neighbour Laplacian kernel
/// `[[0,1,0],[1,-4,1],[0,1,0]]`; its variance is returned. A flat or blurred
/// image yields a low value; a sharp, detailed image yields a high one.
///
/// `width`/`height` are the grey-image dimensions. `pixels` must be
/// `width * height` bytes of greyscale.
pub fn laplacian_variance(pixels: &[u8], width: u32, height: u32) -> f32 {
    let (w, h) = (width as usize, height as usize);
    if w < 3 || h < 3 || pixels.len() != w * h {
        return 0.0;
    }
    let at = |x: usize, y: usize| pixels[y * w + x] as f32;

    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    let mut count = 0u64;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let lap = at(x, y - 1) + at(x - 1, y) + at(x + 1, y) + at(x, y + 1) - 4.0 * at(x, y);
            sum += lap as f64;
            sum_sq += (lap * lap) as f64;
            count += 1;
        }
    }
    if count == 0 {
        return 0.0;
    }
    let mean = sum / count as f64;
    let variance = (sum_sq / count as f64) - mean * mean;
    variance.max(0.0) as f32
}

/// Extract a rectangular region as a greyscale buffer, for quality metrics on
/// the face area specifically.
///
/// `pixels` is row-major RGB (3 channels), `img_w`/`img_h` its dimensions.
pub fn of_region(
    pixels: &[u8],
    img_w: u32,
    img_h: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> Option<(Vec<u8>, u32, u32)> {
    if w == 0 || h == 0 {
        return None;
    }
    if x + w > img_w || y + h > img_h {
        return None;
    }
    let mut out = Vec::with_capacity((w * h) as usize);
    for row in 0..h {
        for col in 0..w {
            let px = ((y + row) as usize) * (img_w as usize) + ((x + col) as usize);
            let idx = px * 3;
            if idx + 2 >= pixels.len() {
                return None;
            }
            let luma = 0.299 * pixels[idx] as f32
                + 0.587 * pixels[idx + 1] as f32
                + 0.114 * pixels[idx + 2] as f32;
            out.push(luma.round().clamp(0.0, 255.0) as u8);
        }
    }
    Some((out, w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_image_has_zero_laplacian_variance() {
        let grey = vec![128u8; 64 * 64];
        assert_eq!(laplacian_variance(&grey, 64, 64), 0.0);
    }

    #[test]
    fn checkerboard_has_high_laplacian_variance() {
        let (w, h) = (64u32, 64u32);
        let mut grey = vec![0u8; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let v = if (x + y) % 2 == 0 { 0 } else { 255 };
                grey[(y * w + x) as usize] = v;
            }
        }
        let var = laplacian_variance(&grey, w, h);
        assert!(var > 1000.0, "expected sharp image, variance was {var}");
    }

    #[test]
    fn tiny_image_returns_zero() {
        assert_eq!(laplacian_variance(&[1, 2, 3], 3, 1), 0.0);
    }

    #[test]
    fn mean_luminance_black_and_white() {
        assert_eq!(mean_luminance(&[0, 0, 0, 0, 0, 0], 3), 0.0);
        assert!((mean_luminance(&[255, 255, 255, 255, 255, 255], 3) - 255.0).abs() < 0.01);
    }

    #[test]
    fn mean_luminance_grey_midpoint() {
        let grey = vec![128u8; 30];
        let lum = mean_luminance(&grey, 3);
        assert!((lum - 128.0).abs() < 0.5, "was {lum}");
    }

    #[test]
    fn region_extraction_out_of_bounds_returns_none() {
        let px = vec![0u8; 10 * 10 * 3];
        assert!(of_region(&px, 10, 10, 5, 5, 10, 10).is_none());
        assert!(of_region(&px, 10, 10, 0, 0, 4, 4).is_some());
    }
}
