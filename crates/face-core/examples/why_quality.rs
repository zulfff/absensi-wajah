//! Print the quality signals + the exact rejection reason per frame, so a
//! too-strict gate is visible instead of guessed.
use face_core::onnx::{build_pipeline, OnnxConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = OnnxConfig::from_env();
    let p = build_pipeline(&cfg, domain::quality::QualityThresholds::default())?;
    for arg in std::env::args().skip(1) {
        let (label, path) = arg.split_once('=').ok_or("label=path")?;
        let bytes = std::fs::read(path)?;
        let img = face_core::RgbImage::decode(&bytes)?;
        match p.process(&img) {
            Ok(out) => {
                let s = &out.quality;
                let qt = p.quality_thresholds();
                let q = domain::quality::FrameQuality::evaluate(s, qt);
                println!(
                    "{label:8} {}x{} face_px={} lap={:.1} lum={:.1} bg={:.1} yaw={:.1} pitch={:.1} roll={:.1} eyes={} -> {:?}",
                    img.width, img.height, s.face_px, s.laplacian_variance, s.mean_luminance,
                    s.background_luminance, s.yaw, s.pitch, s.roll, s.eyes_open, q
                );
            }
            Err(e) => println!("{label}: {e}"),
        }
    }
    Ok(())
}
