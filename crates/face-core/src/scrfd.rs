//! SCRFD detection post-processing: anchor generation, distance decoding, and
//! non-maximum suppression.
//!
//! Kept separate from `onnx.rs` and **free of ORT** so the decoding maths can be
//! unit-tested without any model or native library compiled in.
//!
//! SCRFD pins (verified against `det_10g.onnx`, buffalo_l, InsightFace v0.7):
//! - input `input.1`, NCHW, 640x640, RGB, normalised `(x - 127.5) / 128`.
//! - three strides: 8, 16, 32.
//! - each stride emits, in this output order:
//!   score `[N_cells*2, 1]`, bbox `[N_cells*2, 4]`, kps `[N_cells*2, 10]`
//!   where `N_cells = (640/stride)^2` and 2 = anchors per cell.
//!   (outputs 448/471/494 = scores; 451/474/497 = bboxes; 454/477/500 = kps)
//! - bbox is a *distance* regression (left, top, right, bottom) from the anchor
//!   centre, multiplied by the stride.
//! - kps is 5 points (x, y), each multiplied by the stride.

/// One SCRFD output stride: its scale and the cell count.
#[derive(Clone, Copy, Debug)]
pub struct Stride {
    pub stride: u32,
    /// cells per side = input_size / stride.
    pub cells: u32,
    /// anchors per cell (SCRFD uses 2 at every stride).
    pub anchors: u32,
}

impl Stride {
    pub fn num_anchors(&self) -> usize {
        (self.cells as usize) * (self.cells as usize) * (self.anchors as usize)
    }
}

/// The three SCRFD strides for a 640x640 input.
pub const STRIDES: [Stride; 3] = [
    Stride {
        stride: 8,
        cells: 80,
        anchors: 2,
    },
    Stride {
        stride: 16,
        cells: 40,
        anchors: 2,
    },
    Stride {
        stride: 32,
        cells: 20,
        anchors: 2,
    },
];

/// A decoded detection: bounding box, 5 landmarks, confidence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Detection {
    /// x1, y1, x2, y2 in input-image pixels.
    pub bbox: [f32; 4],
    /// 5 landmarks as (x, y) in input-image pixels.
    pub landmarks: [[f32; 2]; 5],
    pub score: f32,
}

/// Generate the anchor centres for a stride, in the same order the model emits
/// them: for each cell (row-major), then each anchor.
pub fn anchor_centers(stride: &Stride) -> Vec<[f32; 2]> {
    let mut centers = Vec::with_capacity(stride.num_anchors());
    for y in 0..stride.cells {
        for x in 0..stride.cells {
            for _ in 0..stride.anchors {
                centers.push([
                    (x as f32) * stride.stride as f32,
                    (y as f32) * stride.stride as f32,
                ]);
            }
        }
    }
    centers
}

/// Decode one stride's raw head outputs into detections.
///
/// `scores[i]` is the confidence of anchor `i` (already squeezed to a scalar),
/// `bboxes[i]` the 4 distances, `kps[i]` the 10 landmark coordinates. Vectors
/// must be the same length as the anchor count.
pub fn decode_stride(
    stride: &Stride,
    scores: &[f32],
    bboxes: &[f32],
    kps: &[f32],
    score_threshold: f32,
) -> Vec<Detection> {
    let centers = anchor_centers(stride);
    let n = centers.len();
    debug_assert_eq!(scores.len(), n);
    debug_assert_eq!(bboxes.len(), n * 4);
    debug_assert_eq!(kps.len(), n * 10);

    let mut out = Vec::new();
    for i in 0..n {
        let score = scores[i];
        if score < score_threshold {
            continue;
        }
        let cx = centers[i][0];
        let cy = centers[i][1];
        let s = stride.stride as f32;
        let b = &bboxes[i * 4..i * 4 + 4];
        let bbox = [cx - b[0] * s, cy - b[1] * s, cx + b[2] * s, cy + b[3] * s];
        let mut landmarks = [[0.0f32; 2]; 5];
        for (p, lm) in landmarks.iter_mut().enumerate() {
            lm[0] = cx + kps[i * 10 + p * 2] * s;
            lm[1] = cy + kps[i * 10 + p * 2 + 1] * s;
        }
        out.push(Detection {
            bbox,
            landmarks,
            score,
        });
    }
    out
}

/// Intersection-over-union of two `[x1,y1,x2,y2]` boxes.
pub fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let x1 = a[0].max(b[0]);
    let y1 = a[1].max(b[1]);
    let x2 = a[2].min(b[2]);
    let y2 = a[3].min(b[3]);
    let iw = (x2 - x1).max(0.0);
    let ih = (y2 - y1).max(0.0);
    let inter = iw * ih;
    let area_a = (a[2] - a[0]).max(0.0) * (a[3] - a[1]).max(0.0);
    let area_b = (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0);
    let union = area_a + area_b - inter;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// Greedy non-maximum suppression, highest score first.
pub fn nms(mut detections: Vec<Detection>, iou_threshold: f32, max_out: usize) -> Vec<Detection> {
    detections.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut kept: Vec<Detection> = Vec::new();
    'outer: for det in detections {
        for k in &kept {
            if iou(&det.bbox, &k.bbox) > iou_threshold {
                continue 'outer;
            }
        }
        kept.push(det);
        if kept.len() >= max_out {
            break;
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_counts_match_model_outputs() {
        // The model emits 12800 / 3200 / 800 rows per head.
        assert_eq!(STRIDES[0].num_anchors(), 12800);
        assert_eq!(STRIDES[1].num_anchors(), 3200);
        assert_eq!(STRIDES[2].num_anchors(), 800);
    }

    #[test]
    fn anchor_centers_start_at_origin() {
        let c = anchor_centers(&STRIDES[0]);
        assert_eq!(c[0], [0.0, 0.0]);
        assert_eq!(c[1], [0.0, 0.0]); // second anchor same cell
        assert_eq!(c[2], [8.0, 0.0]); // next cell
    }

    #[test]
    fn decode_maps_distances_to_boxes() {
        let s = Stride {
            stride: 8,
            cells: 1,
            anchors: 1,
        };
        // one anchor at (0,0), distances (1,1,1,1) * stride 8 -> box (-8,-8)-(8,8)
        let dets = decode_stride(&s, &[0.9], &[1.0, 1.0, 1.0, 1.0], &[0.0; 10], 0.5);
        assert_eq!(dets.len(), 1);
        assert_eq!(dets[0].bbox, [0.0 - 8.0, 0.0 - 8.0, 0.0 + 8.0, 0.0 + 8.0]);
    }

    #[test]
    fn decode_thresholds_low_scores() {
        let s = Stride {
            stride: 8,
            cells: 1,
            anchors: 1,
        };
        let dets = decode_stride(&s, &[0.1], &[0.0; 4], &[0.0; 10], 0.5);
        assert!(dets.is_empty());
    }

    #[test]
    fn decode_landmarks_scale_by_stride() {
        let s = Stride {
            stride: 16,
            cells: 1,
            anchors: 1,
        };
        let mut kps = [0.0f32; 10];
        kps[0] = 1.0; // first point x
        kps[1] = 2.0; // first point y
        let dets = decode_stride(&s, &[0.9], &[0.0; 4], &kps, 0.5);
        assert_eq!(dets[0].landmarks[0], [16.0, 32.0]);
    }

    #[test]
    fn iou_identical_is_one() {
        let a = [0.0, 0.0, 10.0, 10.0];
        assert!((iou(&a, &a) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn iou_disjoint_is_zero() {
        let a = [0.0, 0.0, 10.0, 10.0];
        let b = [20.0, 20.0, 30.0, 30.0];
        assert_eq!(iou(&a, &b), 0.0);
    }

    #[test]
    fn nms_keeps_highest_and_suppresses_overlap() {
        let d1 = Detection {
            bbox: [0.0, 0.0, 10.0, 10.0],
            landmarks: [[0.0; 2]; 5],
            score: 0.9,
        };
        let d2 = Detection {
            bbox: [1.0, 1.0, 11.0, 11.0],
            landmarks: [[0.0; 2]; 5],
            score: 0.8,
        };
        let d3 = Detection {
            bbox: [50.0, 50.0, 60.0, 60.0],
            landmarks: [[0.0; 2]; 5],
            score: 0.7,
        };
        let kept = nms(vec![d1, d2, d3], 0.4, 10);
        assert_eq!(kept.len(), 2, "overlapping box suppressed, disjoint kept");
        assert_eq!(kept[0].score, 0.9);
        assert_eq!(kept[1].score, 0.7);
    }

    #[test]
    fn nms_respects_max_out() {
        let dets: Vec<_> = (0..10)
            .map(|i| Detection {
                bbox: [(i * 20) as f32, 0.0, (i * 20 + 10) as f32, 10.0],
                landmarks: [[0.0; 2]; 5],
                score: 0.9 - i as f32 * 0.01,
            })
            .collect();
        assert_eq!(nms(dets, 0.4, 3).len(), 3);
    }
}
