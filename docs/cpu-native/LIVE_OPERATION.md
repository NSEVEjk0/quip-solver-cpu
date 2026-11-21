# CPU-native deployment and operation

The native solver integrates through the existing Quip coordinator as a
CPU backend. Its deployment configuration uses two lease workers on CPUs
0 and 1, a 1 GiB service memory cap, 16 reads × 8,192 sweeps, β=12,
polish β=20, and the six documented CPU-native options. Structural screening
is disabled in this operating configuration.

The deployment procedure keeps the coordinator image and operator identity,
installs the native executable under `/data/cpu-native/bin/`, validates Compose,
and recreates the CPU service. Corresponding AGPL source accompanies the binary.
A private snapshot and CPU-service rollback script retain the working baseline.

The final build passes 30 family-library tests, 21 kernel tests, and native
family/plain protocol conformance, including expired deadlines, stale leases,
generation cancellation, canonical witness validation, and credit accounting.
The [validation records](data/validation/) identify the commands and results.

Operational observations record the activated binary and image hashes, actual
CPU affinity and memory use, coordinator candidate-validation counters,
rejections, qblock transitions, and subsequent work. Counts are labeled by
their measurement source; a validated salt candidate is distinct from a
winning proof included on-chain.
