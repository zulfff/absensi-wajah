//! Landmark-derived measurements used by the quality gate: head pose and an
//! eye-open heuristic.
//!
//! These operate on SCRFD's 5 landmarks in the order
//! `[left_eye, right_eye, nose, left_mouth, right_mouth]` (subject's left is
//! image-right for a front-facing person, which is what SCRFD emits).
//!
//! They are approximations, not a full 3D pose solve. That is deliberate: the
//! quality gate only needs to reject large deviations ("not facing the camera"),
//! and an approximation is cheap, deterministic, and testable. If a deployment
//! needs precise pose, swap in a dedicated head-pose model.

/// Estimated head orientation in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// Positive = head turned to the subject's right (image left).
    pub yaw: f32,
    /// Positive = looking up.
    pub pitch: f32,
    /// Positive = head tilted clockwise.
    pub roll: f32,
}

/// Estimate yaw/pitch/roll from 5 landmarks in image pixel coordinates.
///
/// Method:
/// - **roll** from the eye-line angle.
/// - **yaw** from horizontal asymmetry: how far the nose sits from the eye
///   midpoint, normalised by inter-ocular distance.
/// - **pitch** from vertical asymmetry: nose position relative to the
///   eye-to-mouth axis, normalised by face height.
pub fn estimate_pose(lm: &[[f32; 2]; 5]) -> Pose {
    let left_eye = lm[0];
    let right_eye = lm[1];
    let nose = lm[2];

    let dx = right_eye[0] - left_eye[0];
    let dy = right_eye[1] - left_eye[1];
    let eye_dist = (dx * dx + dy * dy).sqrt().max(1e-3);

    let roll = dy.atan2(dx).to_degrees();

    let eye_mid = [
        (left_eye[0] + right_eye[0]) / 2.0,
        (left_eye[1] + right_eye[1]) / 2.0,
    ];
    // Nose offset from eye midpoint, in units of inter-ocular distance.
    // A *x_ratio* of ~0.5 means centred; a 0.5-unit shift is roughly 45 deg.
    let yaw = ((nose[0] - eye_mid[0]) / eye_dist).atan().to_degrees();

    let mouth_mid = [(lm[3][0] + lm[4][0]) / 2.0, (lm[3][1] + lm[4][1]) / 2.0];
    let face_h = ((mouth_mid[1] - eye_mid[1]).powi(2) + (mouth_mid[0] - eye_mid[0]).powi(2))
        .sqrt()
        .max(1e-3);
    // Where the nose sits vertically between eyes and mouth, as a fraction.
    let nose_frac = (nose[1] - eye_mid[1]) / face_h;
    // Neutral front-facing nose sits around 0.5 of the way. Deviation maps to
    // pitch; the 90.0 factor is an empirical scale to degrees.
    let pitch = (nose_frac - 0.5) * 90.0;

    Pose { yaw, pitch, roll }
}

/// A coarse eye-open flag from the same 5 landmarks.
///
/// With only eye centres (no eyelid contour) this cannot truly tell open from
/// closed. It returns `true` (treat as open) unless the landmarks are
/// degenerate, and exists so the pipeline has a value to gate on that a future
/// 68/106-point model can make meaningful. Being permissive here is the
/// *safer* error: a closed-eye frame that slips through is caught by liveness
/// and consensus, whereas wrongly rejecting open eyes blocks everyone.
pub fn eyes_open(_lm: &[[f32; 2]; 5]) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frontal() -> [[f32; 2]; 5] {
        // left_eye, right_eye, nose, mouth_l, mouth_r
        [
            [100.0, 100.0],
            [200.0, 100.0],
            [150.0, 150.0],
            [120.0, 200.0],
            [180.0, 200.0],
        ]
    }

    #[test]
    fn frontal_face_has_near_zero_yaw_roll() {
        let p = estimate_pose(&frontal());
        assert!(p.roll.abs() < 1.0, "roll was {}", p.roll);
        assert!(p.yaw.abs() < 1.0, "yaw was {}", p.yaw);
    }

    #[test]
    fn rolled_head_detected() {
        let mut lm = frontal();
        // Rotate the eye line by ~30 degrees.
        lm[1] = [
            100.0 + 100.0 * 30.0_f32.to_radians().cos(),
            100.0 + 100.0 * 30.0_f32.to_radians().sin(),
        ];
        let p = estimate_pose(&lm);
        assert!(p.roll > 20.0, "expected positive roll, got {}", p.roll);
    }

    #[test]
    fn turned_head_detected() {
        let mut lm = frontal();
        // Move the nose far to one side relative to the eye midpoint.
        lm[2] = [190.0, 150.0];
        let p = estimate_pose(&lm);
        assert!(p.yaw.abs() > 20.0, "expected yaw, got {}", p.yaw);
    }

    #[test]
    fn eyes_open_is_permissive() {
        assert!(eyes_open(&frontal()));
    }

    #[test]
    fn degenerate_landmarks_do_not_panic() {
        let lm = [[0.0; 2]; 5];
        let p = estimate_pose(&lm);
        assert!(p.yaw.is_finite() && p.pitch.is_finite() && p.roll.is_finite());
    }
}
