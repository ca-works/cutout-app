//! Run with rustc -O --edition=2021 app/benches/conventional.rs -o <output>.
#[path = "../src/background.rs"]
mod background;
fn main() {
    for (w, h) in [(512, 512), (1024, 1024), (1920, 1080), (3840, 2160)] {
        let mut p = [255; 4].repeat(w * h);
        for y in h / 4..3 * h / 4 {
            for x in w / 4..3 * w / 4 {
                p[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[20, 80, 120, 255]);
            }
        }
        let mut times = Vec::new();
        for _ in 0..11 {
            let start = std::time::Instant::now();
            std::hint::black_box(
                background::mask(std::hint::black_box(&p), w as u32, h as u32).unwrap(),
            );
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(f64::total_cmp);
        println!(
            "{w}x{h}: median {:.2} ms; min {:.2}; max {:.2} (11 runs)",
            times[5], times[0], times[10]
        );
    }
}
