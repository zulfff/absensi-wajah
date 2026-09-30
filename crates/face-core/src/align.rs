//! Face alignment: the 5-point similarity transform ArcFace expects.
//!
//! ArcFace was trained on faces warped so that five landmarks (both eyes, nose,
//! mouth corners) land at fixed positions in a 112x112 crop. Feeding it a plain
//! bounding-box crop — or a crop from a differently-normed detector — measurably
//! collapses the margin between genuine and impostor scores. This module
//! implements the standard transform:
//!
//! 1. Target landmarks, scaled to a 112x112 output (the ArcFace reference
//!    points, originally defined for 96x112 and re-scaled).
//! 2. A least-squares similarity transform (rotation + uniform scale +
//!    translation, no shear) mapping detected landmarks to targets.
//! 3. Bilinear sampling of the source image through that transform.
//!
//! Pure maths, no model, no ORT — unit-tested with synthetic points.

use crate::{FaceError, RgbImage};

pub const ALIGNED_SIZE: u32 = 112;

/// The canonical ArcFace target landmarks for a 112x112 crop, in the SCRFD
/// landmark order: left eye, right eye, nose, left mouth corner, right mouth
/// corner. These are the widely used reference points (from the InsightFace /
/// ArcFace alignment), scaled from the 96x112 reference to 112x112.
pub const TARGET_LANDMARKS: [[f32; 2]; 5] = [
    [38.2946, 51.6963], // left eye
    [73.5318, 51.5014], // right eye
    [56.0252, 71.7366], // nose
    [41.5493, 92.3655], // left mouth
    [70.7299, 92.2041], // right mouth
];

/// Solve for a similarity transform `(a, b, tx, ty)` such that
/// `[x'] = [a -b][x] + [tx]`
/// `[y']   [b  a][y]   [ty]`
/// i.e. `dst ≈ A · src`, using least squares over 5 point pairs.
///
/// Returns `(a, b, tx, ty)`. Uses the standard closed-form Umeyama/least-squares
/// solution for similarity (not full affine) so the face is not sheared.
pub fn similarity_transform(src: &[[f32; 2]; 5], dst: &[[f32; 2]; 5]) -> (f32, f32, f32, f32) {
    // Centre both point sets.
    let (mut sx, mut sy, mut dx, mut dy) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for i in 0..5 {
        sx += src[i][0];
        sy += src[i][1];
        dx += dst[i][0];
        dy += dst[i][1];
    }
    sx /= 5.0;
    sy /= 5.0;
    dx /= 5.0;
    dy /= 5.0;

    // Accumulate the covariance terms.
    let mut sxx = 0.0f32;
    let mut sxy = 0.0f32;
    let mut syx = 0.0f32;
    let mut syy = 0.0f32;
    let mut src_var = 0.0f32;
    for i in 0..5 {
        let x = src[i][0] - sx;
        let y = src[i][1] - sy;
        let u = dst[i][0] - dx;
        let v = dst[i][1] - dy;
        sxx += x * u;
        sxy += x * v;
        syx += y * u;
        syy += y * v;
        src_var += x * x + y * y;
    }
    if src_var <= f32::EPSILON {
        return (1.0, 0.0, dx - sx, dy - sy);
    }

    // Least-squares similarity: a = (Σ(x·u + y·v)) / Σ(x²+y²),
    //                          b = (Σ(x·v - y·u)) / Σ(x²+y²)
    let a = (sxx + syy) / src_var;
    let b = (sxy - syx) / src_var;
    let tx = dx - (a * sx - b * sy);
    let ty = dy - (b * sx + a * sy);
    (a, b, tx, ty)
}

/// Warp `image` so its `landmarks` land on [`TARGET_LANDMARKS`], producing a
/// 112x112 RGB crop. Uses bilinear sampling.
pub fn warp_to_aligned(image: &RgbImage, landmarks: &[[f32; 2]; 5]) -> Result<RgbImage, FaceError> {
    let (a, b, tx, ty) = similarity_transform(landmarks, &TARGET_LANDMARKS);

    // The forward transform maps source -> target. To fill the target, invert
    // it: for each output pixel, find the source pixel.
    // forward: [x'] = a·x - b·y + tx ; [y'] = b·x + a·y + ty
    // inverse: [x] = ( a·(x'-tx) + b·(y'-ty)) / det
    //          [y] = (-b·(x'-tx) + a·(y'-ty)) / det,  det = a² + b²
    let det = a * a + b * b;
    if det.abs() <= f32::EPSILON {
        return Err(FaceError::OutputShape("degenerate alignment".into()));
    }

    let size = ALIGNED_SIZE as usize;
    let mut pixels = Vec::with_capacity(size * size * 3);
    for oy in 0..size {
        for ox in 0..size {
            let fx = ox as f32;
            let fy = oy as f32;
            let px = fx - tx;
            let py = fy - ty;
            let src_x = (a * px + b * py) / det;
            let src_y = (-b * px + a * py) / det;

            let (r, g, bl) = bilinear(image, src_x, src_y);
            pixels.push(r);
            pixels.push(g);
            pixels.push(bl);
        }
    }
    RgbImage::new(ALIGNED_SIZE, ALIGNED_SIZE, pixels)
}

/// Bilinear sample of an RGB image at floating-point coordinates, clamped to
/// the border.
fn bilinear(image: &RgbImage, x: f32, y: f32) -> (u8, u8, u8) {
    let w = image.width as i64;
    let h = image.height as i64;
    let x0 = x.floor() as i64;
    let y0 = y.floor() as i64;
    let dx = x - x0 as f32;
    let dy = y - y0 as f32;

    let clamp = |vx: i64, vy: i64| -> (f32, f32, f32) {
        let cx = vx.clamp(0, w - 1) as usize;
        let cy = vy.clamp(0, h - 1) as usize;
        let idx = (cy * image.width as usize + cx) * 3;
        (
            image.pixels[idx] as f32,
            image.pixels[idx + 1] as f32,
            image.pixels[idx + 2] as f32,
        )
    };

    let p00 = clamp(x0, y0);
    let p10 = clamp(x0 + 1, y0);
    let p01 = clamp(x0, y0 + 1);
    let p11 = clamp(x0 + 1, y0 + 1);

    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let mix = |p0: (f32, f32, f32), p1: (f32, f32, f32), t: f32| {
        (
            lerp(p0.0, p1.0, t),
            lerp(p0.1, p1.1, t),
            lerp(p0.2, p1.2, t),
        )
    };
    let top_row = mix(p00, p10, dx);
    let bot_row = mix(p01, p11, dx);
    let out = mix(top_row, bot_row, dy);
    (
        out.0.round().clamp(0.0, 255.0) as u8,
        out.1.round().clamp(0.0, 255.0) as u8,
        out.2.round().clamp(0.0, 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_transform_is_identity() {
        let (a, b, tx, ty) = similarity_transform(&TARGET_LANDMARKS, &TARGET_LANDMARKS);
        assert!((a - 1.0).abs() < 1e-4, "a={a}");
        assert!(b.abs() < 1e-4, "b={b}");
        assert!(tx.abs() < 1e-3, "tx={tx}");
        assert!(ty.abs() < 1e-3, "ty={ty}");
    }

    #[test]
    fn pure_translation_recovered() {
        let mut src = TARGET_LANDMARKS;
        for p in &mut src {
            p[0] += 10.0;
            p[1] += 20.0;
        }
        let (a, b, tx, ty) = similarity_transform(&src, &TARGET_LANDMARKS);
        assert!((a - 1.0).abs() < 1e-3);
        assert!(b.abs() < 1e-3);
        assert!((tx + 10.0).abs() < 1e-2, "tx={tx}");
        assert!((ty + 20.0).abs() < 1e-2, "ty={ty}");
    }

    #[test]
    fn warp_of_target_landmarks_reproduces_landmarks() {
        // Build a synthetic image and check the warp does not error and has the
        // right size.
        let img = RgbImage::solid(200, 200, 100, 120, 140);
        let aligned = warp_to_aligned(&img, &TARGET_LANDMARKS).unwrap();
        assert_eq!(aligned.width, ALIGNED_SIZE);
        assert_eq!(aligned.height, ALIGNED_SIZE);
        assert_eq!(
            aligned.pixels.len(),
            (ALIGNED_SIZE * ALIGNED_SIZE * 3) as usize
        );
    }

    #[test]
    fn uniform_scale_is_recovered() {
        // Source at 2x scale around origin -> transform should shrink by 0.5.
        let mut src = TARGET_LANDMARKS;
        for p in &mut src {
            p[0] *= 2.0;
            p[1] *= 2.0;
        }
        let (a, b, _, _) = similarity_transform(&src, &TARGET_LANDMARKS);
        assert!((a - 0.5).abs() < 1e-3, "a={a}");
        assert!(b.abs() < 1e-3);
    }

    #[test]
    fn bilinear_clamps_at_border() {
        let img = RgbImage::solid(4, 4, 10, 20, 30);
        let (r, g, b) = bilinear(&img, -5.0, -5.0);
        assert_eq!((r, g, b), (10, 20, 30));
        let (r2, _, _) = bilinear(&img, 100.0, 100.0);
        assert_eq!(r2, 10);
    }
}
