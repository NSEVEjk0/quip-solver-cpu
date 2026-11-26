//! Eight-link AVX2 residual scan for an already labeled component.

#![expect(
    unsafe_code,
    reason = "AVX2 intrinsics with checked CPU and slice preconditions"
)]

use std::arch::x86_64::*;

use super::graph::Link;

/// # Safety
/// The current CPU must support AVX2. Each link's decoded node must be in `packed`.
pub(super) unsafe fn interior_avx2(links: &[Link], packed: &[u32], label: u32) -> i32 {
    debug_assert!(links.len().is_multiple_of(8));
    debug_assert!(std::arch::is_x86_feature_detected!("avx2"));
    // The caller guarantees the target feature and gather indices.
    unsafe { interior(links, packed, label) }
}

#[target_feature(enable = "avx2")]
unsafe fn interior(links: &[Link], packed: &[u32], label: u32) -> i32 {
    let labels = _mm256_set1_epi32(label as i32);
    let label_mask = _mm256_set1_epi32(i32::MAX);
    let coupling_mask = _mm256_set1_epi32(2047);
    let bias = _mm256_set1_epi32(1024);
    let mut total = _mm256_setzero_si256();
    for chunk in links.chunks_exact(8) {
        // Link is a transparent u32. The unaligned load reads this chunk;
        // the decoded 21-bit indices address only the validated node array.
        let word = unsafe { _mm256_loadu_si256(chunk.as_ptr().cast()) };
        let nodes = _mm256_srli_epi32::<11>(word);
        let values = unsafe { _mm256_i32gather_epi32::<4>(packed.as_ptr().cast(), nodes) };
        let equal = _mm256_cmpeq_epi32(_mm256_and_si256(values, label_mask), labels);
        let coupling = _mm256_sub_epi32(_mm256_and_si256(word, coupling_mask), bias);
        let sign = _mm256_srai_epi32::<31>(values);
        let signed = _mm256_sub_epi32(_mm256_xor_si256(coupling, sign), sign);
        total = _mm256_add_epi32(total, _mm256_and_si256(equal, signed));
    }
    let mut lanes = [0_i32; 8];
    // The result array has a full 32-byte destination for the unaligned store.
    unsafe { _mm256_storeu_si256(lanes.as_mut_ptr().cast(), total) };
    lanes.into_iter().sum()
}
