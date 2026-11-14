//! Prints a few OS samples (a manual check of what the platform layer reads).
fn main() {
    let mut p = decay_platform::native();
    for _ in 0..3 {
        let t = std::time::Instant::now();
        let s = p.sample();
        println!("{:?} mic={:?} camera={:?} audio={:?}", t.elapsed(), s.mic_in_use, s.camera_in_use, s.audio_playing);
    }
}
