use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::marker::PhantomData;
use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;

use super::error::MetricsError;

pub struct JsonlWriter<T> {
    inner: Mutex<BufWriter<File>>,
    _phantom: PhantomData<T>,
}

impl<T: Serialize> JsonlWriter<T> {
    pub fn open(path: &Path) -> Result<Self, MetricsError> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(MetricsError::Io)?;
        Ok(Self {
            inner: Mutex::new(BufWriter::new(file)),
            _phantom: PhantomData,
        })
    }

    pub fn write_record(&self, record: &T) {
        let line = match serde_json::to_string(record) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[metrics] serialization error: {e}");
                return;
            }
        };
        let mut guard = self.inner.lock().expect("metrics writer mutex poisoned");
        if let Err(e) = writeln!(guard, "{line}") {
            eprintln!("[metrics] write error: {e}");
        } else if let Err(e) = guard.flush() {
            eprintln!("[metrics] flush error: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonl_writer_appends_valid_json_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        let writer: JsonlWriter<serde_json::Value> = JsonlWriter::open(&path).unwrap();
        writer.write_record(&serde_json::json!({"a": 1}));
        writer.write_record(&serde_json::json!({"b": 2}));
        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn jsonl_writer_creates_file_if_not_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.jsonl");
        assert!(!path.exists());
        let _writer: JsonlWriter<serde_json::Value> = JsonlWriter::open(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn jsonl_writer_flushes_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flush.jsonl");
        let writer: JsonlWriter<serde_json::Value> = JsonlWriter::open(&path).unwrap();
        writer.write_record(&serde_json::json!({"x": 99}));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.is_empty());
    }

    #[test]
    fn jsonl_writer_rpc_call_roundtrip() {
        use crate::data_model::{Backend, RpcCall};
        use chrono::Utc;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rpc_calls.jsonl");
        let writer = JsonlWriter::<RpcCall>::open(&path).unwrap();
        let call = RpcCall {
            call_id: "c-1".into(),
            run_id: "r-1".into(),
            method: "getblockcount".into(),
            backend: Backend::Zebra,
            params_hash: None,
            request_at: Utc::now(),
            response_at: None,
            latency_ms: Some(42),
            success: true,
            error_code: None,
            error_message: None,
        };
        writer.write_record(&call);
        let content = std::fs::read_to_string(&path).unwrap();
        let back: RpcCall = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(back.call_id, "c-1");
        assert_eq!(back.latency_ms, Some(42));
        assert!(back.success);
    }

    #[test]
    fn jsonl_writer_append_mode_does_not_truncate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("append.jsonl");

        let w1: JsonlWriter<serde_json::Value> = JsonlWriter::open(&path).unwrap();
        w1.write_record(&serde_json::json!({"seq": 1}));
        drop(w1);

        let w2: JsonlWriter<serde_json::Value> = JsonlWriter::open(&path).unwrap();
        w2.write_record(&serde_json::json!({"seq": 2}));
        drop(w2);

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "expected 2 lines after two separate opens");
        let v0: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        let v1: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(v0["seq"], 1);
        assert_eq!(v1["seq"], 2);
    }

    #[test]
    fn jsonl_writer_metric_sample_roundtrip() {
        use crate::data_model::MetricSample;
        use chrono::Utc;
        use std::collections::HashMap;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metrics.jsonl");
        let writer = JsonlWriter::<MetricSample>::open(&path).unwrap();
        let mut labels = HashMap::new();
        labels.insert("backend".to_string(), "Zebra".to_string());
        let sample = MetricSample {
            run_id: "r-1".into(),
            timestamp: Utc::now(),
            metric_name: "mempool_tx_count".into(),
            value: 42.0,
            labels,
        };
        writer.write_record(&sample);
        let content = std::fs::read_to_string(&path).unwrap();
        let back: MetricSample = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(back.metric_name, "mempool_tx_count");
        assert_eq!(back.value, 42.0);
        assert_eq!(back.labels["backend"], "Zebra");
    }
}
