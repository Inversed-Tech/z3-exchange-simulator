use std::collections::HashMap;

use chrono::Utc;

use crate::data_model::{IntentRecord, MetricSample, RpcCall};

use super::error::MetricsError;
use super::latency::LatencyAccumulator;
use super::run_dir::RunDir;
use super::writers::JsonlWriter;
use super::MetricsRecorder;

pub struct JsonlRecorder {
    rpc_writer: JsonlWriter<RpcCall>,
    metric_writer: JsonlWriter<MetricSample>,
    intent_writer: JsonlWriter<IntentRecord>,
    pub(crate) latency: LatencyAccumulator,
}

impl JsonlRecorder {
    pub fn new(run_dir: &RunDir) -> Result<Self, MetricsError> {
        Ok(Self {
            rpc_writer: JsonlWriter::open(&run_dir.rpc_calls_path())?,
            metric_writer: JsonlWriter::open(&run_dir.metrics_path())?,
            intent_writer: JsonlWriter::open(&run_dir.intents_path())?,
            latency: LatencyAccumulator::new(),
        })
    }

    /// Persist one intent's final outcome. Called once per dispatched intent
    /// after the load phase completes — see `docs/architecture/observability.md`.
    pub fn record_intent(&self, record: IntentRecord) {
        self.intent_writer.write_record(&record);
    }

    pub fn flush_latency_samples(&self, run_id: &str) {
        let now = Utc::now();
        for (method, backend) in self.latency.all_keys() {
            if let Some((p50, p95, p99)) = self.latency.percentiles(&method, &backend) {
                for (label, value) in [("p50", p50), ("p95", p95), ("p99", p99)] {
                    let mut labels = HashMap::new();
                    labels.insert("method".into(), method.clone());
                    labels.insert("backend".into(), backend.clone());
                    labels.insert("percentile".into(), label.into());
                    self.metric_writer.write_record(&MetricSample {
                        run_id: run_id.to_string(),
                        timestamp: now,
                        metric_name: "rpc_latency_ms".into(),
                        value,
                        labels,
                    });
                }
            }
        }
    }
}

impl MetricsRecorder for JsonlRecorder {
    fn record_rpc_call(&self, call: RpcCall) {
        if let Some(ms) = call.latency_ms {
            let backend_str = format!("{:?}", call.backend);
            self.latency.record(&call.method, &backend_str, ms);
        }
        self.rpc_writer.write_record(&call);
    }

    fn record_metric(&self, sample: MetricSample) {
        self.metric_writer.write_record(&sample);
    }
}

pub struct NullRecorder;

impl MetricsRecorder for NullRecorder {
    fn record_rpc_call(&self, _: RpcCall) {}
    fn record_metric(&self, _: MetricSample) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_recorder_creates_jsonl_files() {
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "test").unwrap();
        let _rec = JsonlRecorder::new(&rd).unwrap();
        assert!(rd.rpc_calls_path().exists());
        assert!(rd.metrics_path().exists());
        assert!(rd.intents_path().exists());
    }

    #[test]
    fn record_intent_writes_to_intents_file() {
        use crate::data_model::{FlowType, IntentRecord};
        use chrono::Utc;
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "intent-test").unwrap();
        let rec = JsonlRecorder::new(&rd).unwrap();
        rec.record_intent(IntentRecord {
            run_id: "r-1".into(),
            intent_id: "i-1".into(),
            flow_type: FlowType::TToT,
            outcome: "timed_out".into(),
            error: None,
            timeout_context: Some("tx abc did not reach 3 confirmations".into()),
            recorded_at: Utc::now(),
            failure_class: Some(crate::data_model::IntentFailureClass::Timeout),
        });
        let content = std::fs::read_to_string(rd.intents_path()).unwrap();
        assert!(content.contains("timed_out"));
        assert!(content.contains("did not reach 3 confirmations"));
    }

    #[test]
    fn record_rpc_call_writes_to_file() {
        use crate::data_model::{Backend, RpcCall};
        use chrono::Utc;
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "rec-test").unwrap();
        let rec = JsonlRecorder::new(&rd).unwrap();
        let call = RpcCall {
            call_id: "c-1".into(),
            run_id: "r-1".into(),
            method: "getblockcount".into(),
            backend: Backend::Zebra,
            params_hash: None,
            request_at: Utc::now(),
            response_at: None,
            latency_ms: Some(20),
            success: true,
            error_code: None,
            error_message: None,
            phase: crate::data_model::Phase::Unknown,
            intent_id: None,
            attempt_number: 1,
        };
        rec.record_rpc_call(call);
        let content = std::fs::read_to_string(rd.rpc_calls_path()).unwrap();
        assert!(content.contains("getblockcount"));
    }

    #[test]
    fn record_rpc_call_populates_latency_accumulator() {
        use crate::data_model::{Backend, RpcCall};
        use chrono::Utc;
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "lat-test").unwrap();
        let rec = JsonlRecorder::new(&rd).unwrap();
        let call = RpcCall {
            call_id: "c-2".into(),
            run_id: "r-1".into(),
            method: "z_getbalances".into(),
            backend: Backend::Zallet,
            params_hash: None,
            request_at: Utc::now(),
            response_at: None,
            latency_ms: Some(75),
            success: true,
            error_code: None,
            error_message: None,
            phase: crate::data_model::Phase::Unknown,
            intent_id: None,
            attempt_number: 1,
        };
        rec.record_rpc_call(call);
        let (p50, _, _) = rec.latency.percentiles("z_getbalances", "Zallet").unwrap();
        assert_eq!(p50, 75.0);
    }

    #[test]
    fn flush_latency_samples_writes_rpc_latency_ms_metrics() {
        use crate::data_model::{Backend, RpcCall};
        use chrono::Utc;
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "flush-test").unwrap();
        let rec = JsonlRecorder::new(&rd).unwrap();
        let call = RpcCall {
            call_id: "c-3".into(),
            run_id: "r-1".into(),
            method: "getblockcount".into(),
            backend: Backend::Zebra,
            params_hash: None,
            request_at: Utc::now(),
            response_at: None,
            latency_ms: Some(15),
            success: true,
            error_code: None,
            error_message: None,
            phase: crate::data_model::Phase::Unknown,
            intent_id: None,
            attempt_number: 1,
        };
        rec.record_rpc_call(call);
        rec.flush_latency_samples("r-1");
        let content = std::fs::read_to_string(rd.metrics_path()).unwrap();
        assert!(content.contains("rpc_latency_ms"));
        assert!(content.contains("getblockcount"));
        assert!(
            content.contains("\"backend\""),
            "backend label must be present in rpc_latency_ms sample"
        );
        assert!(
            content.contains("\"Zebra\""),
            "backend value must be the PascalCase Backend string"
        );
    }

    #[test]
    fn null_recorder_does_not_panic() {
        use crate::data_model::{Backend, RpcCall};
        use chrono::Utc;
        let rec = NullRecorder;
        rec.record_rpc_call(RpcCall {
            call_id: "c".into(),
            run_id: "r".into(),
            method: "getblockcount".into(),
            backend: Backend::Zebra,
            params_hash: None,
            request_at: Utc::now(),
            response_at: None,
            latency_ms: None,
            success: true,
            error_code: None,
            error_message: None,
            phase: crate::data_model::Phase::Unknown,
            intent_id: None,
            attempt_number: 1,
        });
        rec.record_metric(MetricSample {
            run_id: "r".into(),
            timestamp: Utc::now(),
            metric_name: "test".into(),
            value: 1.0,
            labels: HashMap::new(),
        });
    }

    #[test]
    fn record_rpc_call_none_latency_writes_to_file_but_not_accumulator() {
        use crate::data_model::{Backend, RpcCall};
        use chrono::Utc;
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "nolat").unwrap();
        let rec = JsonlRecorder::new(&rd).unwrap();
        let call = RpcCall {
            call_id: "c-none".into(),
            run_id: "r-1".into(),
            method: "getinfo".into(),
            backend: Backend::Zebra,
            params_hash: None,
            request_at: Utc::now(),
            response_at: None,
            latency_ms: None,
            success: true,
            error_code: None,
            error_message: None,
            phase: crate::data_model::Phase::Unknown,
            intent_id: None,
            attempt_number: 1,
        };
        rec.record_rpc_call(call);
        let content = std::fs::read_to_string(rd.rpc_calls_path()).unwrap();
        assert!(
            content.contains("getinfo"),
            "call must be written to rpc_calls.jsonl even without latency"
        );
        assert!(
            rec.latency.all_keys().is_empty(),
            "latency accumulator must stay empty when latency_ms is None"
        );
    }

    #[test]
    fn record_metric_writes_to_metrics_file() {
        use chrono::Utc;
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "recmet").unwrap();
        let rec = JsonlRecorder::new(&rd).unwrap();
        rec.record_metric(MetricSample {
            run_id: "r-1".into(),
            timestamp: Utc::now(),
            metric_name: "mempool_tx_count".into(),
            value: 77.0,
            labels: HashMap::new(),
        });
        let content = std::fs::read_to_string(rd.metrics_path()).unwrap();
        assert!(content.contains("mempool_tx_count"));
        assert!(content.contains("77"));
    }

    #[test]
    fn flush_latency_samples_with_empty_accumulator_writes_nothing() {
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "emptyflush").unwrap();
        let rec = JsonlRecorder::new(&rd).unwrap();
        rec.flush_latency_samples("r-1");
        let content = std::fs::read_to_string(rd.metrics_path()).unwrap();
        assert!(
            content.is_empty(),
            "metrics file must stay empty after flush with no data"
        );
    }

    #[test]
    fn flush_latency_samples_is_cumulative() {
        use crate::data_model::{Backend, RpcCall};
        use chrono::Utc;
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "cumulative").unwrap();
        let rec = JsonlRecorder::new(&rd).unwrap();

        let make_call = |ms: u64| RpcCall {
            call_id: "c".into(),
            run_id: "r-1".into(),
            method: "getblockcount".into(),
            backend: Backend::Zebra,
            params_hash: None,
            request_at: Utc::now(),
            response_at: None,
            latency_ms: Some(ms),
            success: true,
            error_code: None,
            error_message: None,
            phase: crate::data_model::Phase::Unknown,
            intent_id: None,
            attempt_number: 1,
        };

        rec.record_rpc_call(make_call(100));
        rec.flush_latency_samples("r-1");

        let after_first = std::fs::read_to_string(rd.metrics_path()).unwrap();
        let first_count = after_first.lines().count();
        assert!(first_count > 0, "first flush must produce metric records");

        rec.record_rpc_call(make_call(200));
        rec.flush_latency_samples("r-1");

        let after_second = std::fs::read_to_string(rd.metrics_path()).unwrap();
        let second_count = after_second.lines().count();
        assert!(
            second_count > first_count,
            "second flush must append more records (accumulator is cumulative)"
        );

        // p50 after second flush should reflect both 100 and 200 ms.
        let (p50, _, _) = rec.latency.percentiles("getblockcount", "Zebra").unwrap();
        assert!(
            p50 == 100.0 || p50 == 200.0,
            "p50 should come from [100, 200]; got {p50}"
        );
    }
}
