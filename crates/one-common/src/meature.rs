use std::time::Duration;
use tokio::time::Instant;

pub fn measure_start(label: &str) -> (String, Instant) {
    (label.to_string(), Instant::now())
}

pub fn measure_end(start: (String, Instant)) -> Duration {
    let elapsed = start.1.elapsed();
    info!("[time]: {:.2?} for '{}'", elapsed, start.0);
    elapsed
}

mod tests {
    use super::*;
    use crate::init_logs;

    #[test]
    fn test_time_measure() {
        init_logs();

        let start = measure_start("test");

        std::thread::sleep(std::time::Duration::from_secs(1));

        measure_end(start);
    }
}
