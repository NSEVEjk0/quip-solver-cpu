# CPU baseline compatibility

The earlier `quip-cpu-funnel` binary remains available as a reference CPU
implementation. The current implementation and technical contribution are
presented in [Quip CPU Native](README.md).

The baseline uses the same canonical family generator, population sampler,
and original-energy polishing path. CPU-native execution options are off by
default in this compatibility binary. Its `funnel_*` configuration keys and
`QUIP_CPU_FUNNEL_*` environment variables remain supported.

The native binary uses `native_*` keys and `QUIP_CPU_NATIVE_*` variables.
See the [operator guide](docs/cpu-native/OPERATOR_GUIDE.md) for the evaluated
configuration and [technical report](docs/cpu-native/TECHNICAL_REPORT.md) for
execution and candidate-preservation changes.

The [20 November baseline records](docs/cpu-funnel/RESULTS.md) retain the
original stock/tuned and screening comparisons. Current measurements are in
[the native experiment report](docs/cpu-native/EXPERIMENTS.md).

[Source provenance and license](ACKNOWLEDGMENTS.md) apply to both binaries.
