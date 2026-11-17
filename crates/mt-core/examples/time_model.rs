//! Time a super-resolution model at several input sizes, to find out what it
//! costs before deciding whether it can be offered.
use rten_tensor::prelude::*;
use rten_tensor::NdTensor;

fn main() {
    let path = std::env::args().nth(1).expect("usage: time_model <model.onnx>");
    let model = rten::Model::load(std::fs::read(&path).unwrap()).unwrap();
    println!("threads available: {}", std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0));

    for side in [64usize, 128, 192, 256] {
        let input: NdTensor<f32, 4> = NdTensor::zeros([1, 3, side, side]);
        // Twice: the first run pays for weight preparation.
        let mut best = std::time::Duration::MAX;
        for _ in 0..2 {
            let started = std::time::Instant::now();
            match model.run_one(input.view().into(), None) {
                Ok(v) => {
                    let out: NdTensor<f32, 4> = v.try_into().unwrap();
                    let took = started.elapsed();
                    best = best.min(took);
                    if side == 64 {
                        println!("  (output {:?})", out.shape());
                    }
                }
                Err(e) => {
                    println!("{side}: failed {e}");
                    break;
                }
            }
        }
        let px = (side * side) as f64;
        println!(
            "{side}x{side}: {:?}  -> {:.1} s per megapixel of input",
            best,
            best.as_secs_f64() / px * 1_000_000.0
        );
    }
}
