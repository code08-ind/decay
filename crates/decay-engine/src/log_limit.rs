//! Caps for warnings a misbehaving peer can repeat without end (a looping extension
//! mustn't fill the disk with one line): the first [`FIRST`] are logged, then one in
//! [`EVERY`], each saying how many there have been.

use std::sync::atomic::{AtomicU64, Ordering};

const FIRST: u64 = 20;
const EVERY: u64 = 1000;

/// Payloads that don't parse.
pub static BAD_PAYLOAD: AtomicU64 = AtomicU64::new(0);
/// Frames that aren't JSON.
pub static BAD_FRAME: AtomicU64 = AtomicU64::new(0);

/// Whether this occurrence is logged.
pub fn allow(counter: &AtomicU64) -> bool {
    let n = counter.fetch_add(1, Ordering::Relaxed) + 1;
    if n == FIRST + 1 {
        tracing::warn!("more of the same warning follow: logging one in {EVERY} from now on");
    }
    n <= FIRST || n.is_multiple_of(EVERY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_the_first_then_one_in_a_thousand() {
        let c = AtomicU64::new(0);
        let logged = (0..5000).filter(|_| allow(&c)).count();
        assert_eq!(logged, 20 + 5);
    }
}
