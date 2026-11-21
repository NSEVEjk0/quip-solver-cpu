# Source provenance and license

The CPU-native implementation is released under **AGPL-3.0-or-later**.
The full license is in [LICENSE](LICENSE).

| Component | Source |
|---|---|
| CPU solver framework and base cluster sampler | [QuipNetwork/quip-solver-cpu](https://github.com/QuipNetwork/quip-solver-cpu), baseline `de51fd4fd31fef2f50ff3a343961796dbb5ca49f`, v0.3.6 |
| Canonical family generation, scoring, and solver session | [QuipNetwork/quip-solver-core](https://github.com/QuipNetwork/quip-solver-core), published crates pinned in Cargo.lock |
| Cached family generator and legacy structural screen modules | [ntllinh2511/quip-solver-cuda](https://github.com/ntllinh2511/quip-solver-cuda/commit/77bfd8210616174e2cd9325a77220f1e122adafd), `fast_dimer.rs`, `stage0.rs`, `stage0v2.rs` |

The legacy screen implementations retain their original comments and fitted
coefficients. Their source is credited here; CPU-native execution, resampling,
candidate preservation, diagnostics, and the current-recipe research harness
are implemented in this branch.

The underlying auxiliary-bond method follows Otsuka, *Physical Review B*
90, 220406 (2014), as documented in the base kernel. Population annealing,
Metropolis updates, and isoenergetic cluster moves are established methods.
This project's contribution is the CPU implementation and measured execution
and search improvements, with reproducible evaluation on Quip instances.

Corresponding source, build instructions, and dependency pins accompany
the released binary. Preserve this provenance and the license when redistributing.
