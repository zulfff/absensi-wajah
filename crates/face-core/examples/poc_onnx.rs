//! Real-model PoC (plan Section 14, step 2): load SCRFD + ArcFace via `ort`,
//! detect faces, embed them, and print cosine similarities.
//!
//! Run with:
//!   ORT_DYLIB_PATH=/path/to/libonnxruntime.so \
//!   FACE_DETECTOR_MODEL=models/det_10g.onnx \
//!   FACE_EMBEDDER_MODEL=models/w600k_r50.onnx \
//!   cargo run -p face-core --features onnx --example poc_onnx -- img_a.jpg img_b.jpg
//!
//! It prints the detected box per image and the cosine similarity between the
//! two embeddings. Two photos of the same person should score far higher than
//! two different people.

use face_core::{FaceDetector, FaceEmbedder, RgbImage};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: poc_onnx <image_a> <image_b>");
        std::process::exit(2);
    }

    let config = face_core::onnx::OnnxConfig::from_env();
    println!("detector: {}", config.detector_path);
    println!("embedder: {}", config.embedder_path);

    // Detect + embed without the liveness stage, so the PoC runs without a
    // MiniFASNet file. We use the detector and embedder directly.
    let detector = face_core::onnx::ScrfdDetector::load(&config)?;
    let embedder = face_core::onnx::ArcFaceEmbedder::load(&config)?;

    let mut embeddings = Vec::new();
    for path in &args[1..3] {
        let bytes = std::fs::read(path)?;
        let image = RgbImage::decode(&bytes)?;
        let faces = detector.detect(&image)?;
        println!(
            "\n{path}: {}x{} -> {} face(s)",
            image.width,
            image.height,
            faces.len()
        );
        if faces.is_empty() {
            eprintln!("no face detected in {path}");
            std::process::exit(1);
        }
        let face = &faces[0];
        println!(
            "  bbox=[{:.0},{:.0},{:.0},{:.0}] score={:.3} yaw={:.1} pitch={:.1} roll={:.1}",
            face.bbox[0],
            face.bbox[1],
            face.bbox[2],
            face.bbox[3],
            face.confidence,
            face.yaw,
            face.pitch,
            face.roll
        );
        // Align using the landmark similarity transform — the same path the
        // production pipeline uses. A plain bbox crop would inflate the
        // impostor scores and hide a real defect.
        let aligned = face_core::align::warp_to_aligned(&image, &face.landmarks)?;
        let emb = embedder.embed(&aligned)?;
        embeddings.push(emb);
    }

    let sim = domain::cosine_similarity(&embeddings[0], &embeddings[1]);
    println!("\ncosine similarity: {sim:.4}");
    println!(
        "interpretation: {}",
        if sim >= 0.5 {
            "same person (above the default T_ACCEPT)"
        } else {
            "different people (below the default T_ACCEPT)"
        }
    );
    Ok(())
}
