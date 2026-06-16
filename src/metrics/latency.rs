use std::collections::HashMap;
use std::sync::Mutex;

pub struct LatencyAccumulator {
    samples: Mutex<HashMap<(String, String), Vec<u64>>>,
}

impl LatencyAccumulator {
    pub fn new() -> Self {
        Self {
            samples: Mutex::new(HashMap::new()),
        }
    }

    pub fn record(&self, method: &str, backend_str: &str, latency_ms: u64) {
        self.samples
            .lock()
            .unwrap()
            .entry((method.to_string(), backend_str.to_string()))
            .or_default()
            .push(latency_ms);
    }

    pub fn percentiles(&self, method: &str, backend_str: &str) -> Option<(f64, f64, f64)> {
        let guard = self.samples.lock().unwrap();
        let v = guard.get(&(method.to_string(), backend_str.to_string()))?;
        if v.is_empty() {
            return None;
        }
        let mut sorted = v.clone();
        sorted.sort_unstable();
        Some((
            percentile_value(&sorted, 0.50),
            percentile_value(&sorted, 0.95),
            percentile_value(&sorted, 0.99),
        ))
    }

    pub fn all_keys(&self) -> Vec<(String, String)> {
        self.samples.lock().unwrap().keys().cloned().collect()
    }
}

// Nearest-rank percentile. idx = floor(p * n), clamped to n-1.
// Callers must not pass an empty slice.
pub(crate) fn percentile_value(sorted: &[u64], p: f64) -> f64 {
    let n = sorted.len();
    let idx = ((p * n as f64).floor() as usize).min(n - 1);
    sorted[idx] as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_single_sample() {
        let acc = LatencyAccumulator::new();
        acc.record("foo", "Zebra", 100);
        let (p50, p95, p99) = acc.percentiles("foo", "Zebra").unwrap();
        assert_eq!(p50, 100.0);
        assert_eq!(p95, 100.0);
        assert_eq!(p99, 100.0);
    }

    #[test]
    fn percentiles_ten_samples() {
        let acc = LatencyAccumulator::new();
        for i in 1..=10u64 {
            acc.record("bar", "Zallet", i * 10);
        }
        // Sorted: [10, 20, 30, 40, 50, 60, 70, 80, 90, 100]
        // P50: idx floor(0.50 * 10) = 5 → 60
        // P95: idx floor(0.95 * 10) = 9 → 100
        let (p50, _, p99) = acc.percentiles("bar", "Zallet").unwrap();
        assert_eq!(p50, 60.0);
        assert_eq!(p99, 100.0);
    }

    #[test]
    fn percentiles_unknown_method_returns_none() {
        let acc = LatencyAccumulator::new();
        assert!(acc.percentiles("nonexistent", "Zebra").is_none());
    }

    #[test]
    fn all_keys_returns_recorded_method_backend_pairs() {
        let acc = LatencyAccumulator::new();
        acc.record("a", "Zebra", 1);
        acc.record("b", "Zallet", 2);
        acc.record("a", "Zebra", 3);
        let mut keys = acc.all_keys();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                ("a".to_string(), "Zebra".to_string()),
                ("b".to_string(), "Zallet".to_string()),
            ]
        );
    }

    #[test]
    fn percentile_value_five_element_slice() {
        // sorted = [10, 20, 30, 40, 50], n=5
        // p50: floor(0.5 * 5) = 2 → 30
        // p95: floor(0.95 * 5) = floor(4.75) = 4 → 50
        // p99: floor(0.99 * 5) = floor(4.95) = 4 → 50
        let s = &[10u64, 20, 30, 40, 50];
        assert_eq!(percentile_value(s, 0.50), 30.0);
        assert_eq!(percentile_value(s, 0.95), 50.0);
        assert_eq!(percentile_value(s, 0.99), 50.0);
    }

    #[test]
    fn percentile_value_two_element_slice() {
        // sorted = [10, 90], n=2
        // p50: floor(0.5 * 2) = 1 → 90
        // p99: floor(0.99 * 2) = floor(1.98) = 1 → 90
        let s = &[10u64, 90];
        assert_eq!(percentile_value(s, 0.50), 90.0);
        assert_eq!(percentile_value(s, 0.99), 90.0);
    }

    #[test]
    fn backend_isolation_different_keys_do_not_interfere() {
        let acc = LatencyAccumulator::new();
        for ms in [10u64, 20, 30] {
            acc.record("method", "Zebra", ms);
        }
        for ms in [100u64, 200, 300] {
            acc.record("method", "Zallet", ms);
        }
        // Zebra p50: floor(0.5*3)=1 → sorted[1]=20
        // Zallet p50: floor(0.5*3)=1 → sorted[1]=200
        let (z_p50, _, _) = acc.percentiles("method", "Zebra").unwrap();
        let (za_p50, _, _) = acc.percentiles("method", "Zallet").unwrap();
        assert_eq!(z_p50, 20.0);
        assert_eq!(za_p50, 200.0);
    }
}
