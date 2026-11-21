//! CPU-native execution and proof-objective candidate preservation.

use super::funnel::FunnelSampler;
use quip_solver_core::families::{Declaration, FamilyContext};
use quip_solver_core::{
    CancelToken, IsingGraph, Lease, LeaseSink, SampleError, SampleParams, Sampler, SamplerResult,
    StreamResult, TopologyView, WarmStart, WarmStreamJob,
};

/// CPU-native sampler with the standard Quip solver protocol.
pub struct NativeSampler(FunnelSampler);

impl Default for NativeSampler {
    fn default() -> Self {
        Self(FunnelSampler::native())
    }
}

impl NativeSampler {
    /// Create a CPU-native sampler. Backend TOML overrides environment defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Sampler for NativeSampler {
    fn sample(
        &self,
        graph: &IsingGraph,
        params: &SampleParams,
    ) -> Result<Vec<SamplerResult>, SampleError> {
        self.0.sample(graph, params)
    }
    fn accepts_family_metadata() -> bool {
        true
    }
    fn generates_locally() -> bool {
        true
    }
    fn declaration() -> Declaration {
        Declaration {
            solver_id: "quip-cpu-native/1",
            ..FunnelSampler::declaration()
        }
    }
    fn stream_width(&self) -> usize {
        self.0.stream_width()
    }
    fn declared_stream_width() -> u32 {
        0
    }
    fn max_reads(&self) -> u32 {
        self.0.max_reads()
    }
    fn apply_config(&self, backend_toml: &str) {
        if let Ok(mut table) = backend_toml.parse::<toml::Table>() {
            let native: Vec<_> = table
                .keys()
                .filter(|k| k.starts_with("native_"))
                .cloned()
                .collect();
            for key in native {
                if let Some(value) = table.remove(&key) {
                    table.insert(key.replacen("native_", "funnel_", 1), value);
                }
            }
            self.0
                .apply_config(&toml::to_string(&table).unwrap_or_default());
        } else {
            self.0.apply_config(backend_toml);
        }
    }
    fn sample_family(
        &self,
        graph: &IsingGraph,
        params: &SampleParams,
        warm: Option<&WarmStart>,
        family: &FamilyContext,
    ) -> Result<Vec<SamplerResult>, SampleError> {
        self.0.sample_family(graph, params, warm, family)
    }
    fn sample_lease(
        &self,
        lease: &Lease,
        topology: &TopologyView,
        params: &SampleParams,
        sink: &LeaseSink,
    ) -> Result<(), SampleError> {
        self.0.sample_lease(lease, topology, params, sink)
    }
    fn sample_stream_warm(
        &self,
        jobs: tokio::sync::mpsc::Receiver<WarmStreamJob>,
        out: tokio::sync::mpsc::Sender<StreamResult>,
        cancel: CancelToken,
    ) {
        self.0.sample_stream_warm(jobs, out, cancel);
    }
}
