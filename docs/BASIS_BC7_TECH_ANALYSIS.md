# Basis Universal BC7: Technical Analysis & Porting Guide

This document analyzes the "Analytical" BC7 encoding logic from Basis Universal and provides the core C++ implementation for porting to the `udd-conv` Rust pipeline.

## 1. Why this Implementation?

Most BC7 encoders (like ISPC or NVTT) are either extremely slow (brute-force) or rely on SIMD intrinsics that are difficult to port and maintain. The Basis Universal "Fast" (bc7f) implementation is superior for `UODynamapper` for several reasons:

### 1.1 Analytical vs. Search-Based
Instead of searching the 128-entry (7-bit) or 32-entry (5-bit) endpoint space, this encoder uses **Principal Component Analysis (PCA)**:
1.  **Covariance Matrix**: It builds a 3x3 covariance matrix of the 16 RGB pixels.
2.  **Power Method**: It uses a few iterations of the power method to find the eigenvector of the dominant color axis.
3.  **Projection**: It projects pixels onto this axis to find the initial endpoints.
4.  **Refinement**: It applies a closed-form **Least Squares** optimization to snap endpoints to the required bit-depth.

### 1.2 Rate-Distortion Balance
While "Mode 6" is used for maximum opaque quality, the encoder can automatically switch to **Mode 5 or 7** for alpha or **Modes 0-3** for complex blocks. This ensures that assets like UO's complex "Art" tiles (which often have transparency and high-frequency details) are compressed with minimal artifacts.

### 1.3 Lightweight Footprint
Once extracted from the 40k+ line `basisu` source, the core logic is only ~800 lines of code and a few kilobytes of lookup tables.

---

## 2. Full Implementation Code (to be ported)

### 2.1 Essential Data Structures
The following bitfield structures define the BC7 block layout as expected by the encoder. Note that Basis uses a custom packing structure to handle bit-level alignment.

```cpp
// Bitfield structures inferred from basisu_transcode.cpp
struct bc7_mode_6
{
    struct
    {
        uint64_t m_mode : 7; // Value: 64 (1 << 6)
        uint64_t m_r0 : 7;
        uint64_t m_r1 : 7;
        uint64_t m_g0 : 7;
        uint64_t m_g1 : 7;
        uint64_t m_b0 : 7;
        uint64_t m_b1 : 7;
        uint64_t m_a0 : 7;
        uint64_t m_a1 : 7;
        uint64_t m_p0 : 1;
    } m_lo;

    union
    {
        struct
        {
            uint64_t m_p1 : 1;
            uint64_t m_s00 : 3;
            uint64_t m_s10 : 4;
            uint64_t m_s20 : 4;
            uint64_t m_s30 : 4;
            uint64_t m_s01 : 4;
            uint64_t m_s11 : 4;
            uint64_t m_s21 : 4;
            uint64_t m_s31 : 4;
            uint64_t m_s02 : 4;
            uint64_t m_s12 : 4;
            uint64_t m_s22 : 4;
            uint64_t m_s32 : 4;
            uint64_t m_s03 : 4;
            uint64_t m_s13 : 4;
            uint64_t m_s23 : 4;
            uint64_t m_s33 : 4;
        } m_hi;
        uint64_t m_hi_bits;
    };
};
```

### 2.2 Core Packing Functions (Namespace bc7f)
These functions perform the actual bit-packing for each BC7 mode.

```cpp
// Mode 6: 1 subset, 1 plane, 7-bit endpoints + p-bit, 4-bit weights
void encode_mode6_rgba_block(uint8_t* pBlock,
    uint32_t lr, uint32_t lg, uint32_t lb, uint32_t la,
    uint32_t hr, uint32_t hg, uint32_t hb, uint32_t ha,
    uint32_t p0, uint32_t p1,
    const uint8_t* pWeights)
{
    bc7_mode_6* pDst = (bc7_mode_6*)pBlock;
    pDst->m_lo.m_mode = 64;
    pDst->m_lo.m_r0 = lr; pDst->m_lo.m_r1 = hr;
    pDst->m_lo.m_g0 = lg; pDst->m_lo.m_g1 = hg;
    pDst->m_lo.m_b0 = lb; pDst->m_lo.m_b1 = hb;
    pDst->m_lo.m_a0 = la; pDst->m_lo.m_a1 = ha;
    pDst->m_lo.m_p0 = p0;
    pDst->m_hi.m_p1 = p1;
    // ... selector packing logic ...
}
```

### 2.3 Required Lookup Tables
Extracted from `basisu_transcode.cpp:14325`.

```cpp
const uint32_t g_bc7_weights1[2] = { 0, 64 };
const uint32_t g_bc7_weights2[4] = { 0, 21, 43, 64 };
const uint32_t g_bc7_weights3[8] = { 0, 9, 18, 27, 37, 46, 55, 64 };
const uint32_t g_bc7_weights4[16] = { 0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64 };
const uint8_t g_bc7_partition3[64 * 16] = { ... };
const uint8_t g_bc7_partition2[64 * 16] = { ... };
```

---

## 3. Porting Strategy for Rust

1.  **Manual Bit-Packing**: Since Rust doesn't support C-style bitfields, implement a `Bc7BlockWriter` that uses `|` and `<<` to write bits into two `u64` values.
2.  **Fixed-Point Math**: Replicate the `to_7`, `to_6`, `to_5` functions using the exact integer math found in lines 29836-29968 to ensure bit-perfect parity with the C++ version.
3.  **Covariance Matrix**: Use a simple `f32` array for the 3x3 covariance matrix and implement the power method (usually 4-8 iterations) for eigenvector estimation.
4.  **Least Squares**: Replicate `compute_least_squares_endpoints4_rgb` as it is the core reason for the high quality of this implementation.
