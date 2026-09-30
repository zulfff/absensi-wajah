//! Measure genuine vs impostor score distributions with the REAL pipeline, so
//! thresholds can be chosen from data instead of guessed (plan Section 6/10).
use face_core::onnx::{build_pipeline, OnnxConfig};
use std::collections::BTreeMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = OnnxConfig::from_env();
    let p = build_pipeline(&cfg, domain::quality::QualityThresholds::default())?;

    // args: label=path...
    let mut by_label: BTreeMap<String, Vec<(String, domain::Embedding)>> = BTreeMap::new();
    for arg in std::env::args().skip(1) {
        let (label, path) = arg.split_once('=').ok_or("usage: label=path ...")?;
        let bytes = std::fs::read(path)?;
        let img = face_core::RgbImage::decode(&bytes)?;
        match p.process(&img) {
            Ok(out) => {
                println!(
                    "OK   {label:8} {path}  liveness={:.4} spoof={} quality={}",
                    out.liveness.score, out.liveness.is_spoof,
                    domain::quality::FrameQuality::evaluate(&out.quality, p.quality_thresholds()).is_passed()
                );
                by_label.entry(label.to_string()).or_default().push((path.to_string(), out.embedding));
            }
            Err(e) => println!("SKIP {label:8} {path}  ({e})"),
        }
    }

    let mut genuine: Vec<f32> = Vec::new();
    let mut impostor: Vec<f32> = Vec::new();
    // genuine: same label, different image
    for v in by_label.values() {
        for i in 0..v.len() {
            for j in i + 1..v.len() {
                genuine.push(domain::cosine_similarity(&v[i].1, &v[j].1));
            }
        }
    }
    // impostor: different labels
    let keys: Vec<&String> = by_label.keys().collect();
    for a in 0..keys.len() {
        for b in a + 1..keys.len() {
            for (_, ea) in &by_label[keys[a]] {
                for (_, eb) in &by_label[keys[b]] {
                    impostor.push(domain::cosine_similarity(ea, eb));
                }
            }
        }
    }

    genuine.sort_by(|a, b| a.partial_cmp(b).unwrap());
    impostor.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let stat = |v: &[f32]| -> (f32, f32, f32) {
        if v.is_empty() { return (0.0, 0.0, 0.0); }
        (v[0], v[v.len() / 2], v[v.len() - 1])
    };
    println!("\n=== SCORE DISTRIBUTION ===");
    println!("genuine  n={:3}  min={:.4} median={:.4} max={:.4}", genuine.len(), stat(&genuine).0, stat(&genuine).1, stat(&genuine).2);
    println!("impostor n={:3}  min={:.4} median={:.4} max={:.4}", impostor.len(), stat(&impostor).0, stat(&impostor).1, stat(&impostor).2);

    println!("\n=== FAR / FRR at candidate T_ACCEPT ===");
    for t in [0.40f32, 0.45, 0.50, 0.55, 0.60, 0.65, 0.70, 0.75, 0.80] {
        let fa = impostor.iter().filter(|s| **s >= t).count();
        let fr = genuine.iter().filter(|s| **s < t).count();
        let far = if impostor.is_empty() { 0.0 } else { fa as f64 / impostor.len() as f64 };
        let frr = if genuine.is_empty() { 0.0 } else { fr as f64 / genuine.len() as f64 };
        println!("T={t:.2}  FAR={far:.4} ({fa}/{})  FRR={frr:.4} ({fr}/{})", impostor.len(), genuine.len());
    }
    Ok(())
}
