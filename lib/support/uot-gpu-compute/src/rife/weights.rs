//! The RIFE v4.25 weights: the fp16 safetensors file bundled with the binary, read and laid out again for the
//! convolution shader, which reads each kernel tap's weights for consecutive output channels together.

use std::collections::HashMap;
use serde::Deserialize;

/// The 158 tensors the network uses at inference, fp16, named as in Practical-RIFE's IFNet_HDv3.py.
pub(super) const WEIGHTS_FILE: &[u8] = include_bytes!("../../models/rife_v4_25_f16.safetensors");

/// Bytes of the little-endian header length that starts a safetensors file.
const HEADER_LENGTH_BYTES: usize = 8;
/// The optional header entry that is not a tensor.
const METADATA_KEY: &str = "__metadata__";
const F16_DTYPE: &str = "F16";
const F16_BYTES: usize = 2;
/// Kernel sides: the 3x3 convolutions and the 4x4 stride 2 transposed ones.
const CONV_KERNEL: usize = 3;
const TRANSPOSED_KERNEL: usize = 4;
/// A transposed convolution's output pixels split into 2x2 phases by parity; each phase sees 2x2 of the kernel.
const PHASE_SIDE: usize = 2;

#[derive(Deserialize)]
struct TensorInfo {
    dtype: String,
    shape: Vec<usize>,
    data_offsets: [usize; 2],
}

/// Every tensor as fp16 bits in one array, each starting at an even index (a whole 32-bit word on the GPU).
pub(super) struct PackedParams {
    pub halves: Vec<u16>,
    offsets: HashMap<String, u32>,
}

impl PackedParams {
    /// Reads a safetensors file. A 3x3 convolution's weight `[out, in, 3, 3]` becomes `[in * 9 + tap][out]`; a
    /// transposed one's `[in, out, 4, 4]` becomes `[phase][in * 4 + tap][out]`, phase `py * 2 + px` covering the
    /// output pixels of row parity `py` and column parity `px`, tap `a * 2 + b` the kernel element
    /// `(2a + 1 - py, 2b + 1 - px)`, which reads input pixel `(y + py - a, x + px - b)` for output `(2y + py, 2x + px)`.
    /// Other tensors (biases, ResConv betas) stay as they are.
    pub fn parse(file: &[u8]) -> Result<Self, String> {
        let length_bytes = file.get(..HEADER_LENGTH_BYTES).ok_or("RIFE weights: file shorter than its header length")?;
        let header_length = u64::from_le_bytes(length_bytes.try_into().unwrap()) as usize;
        let data_start = HEADER_LENGTH_BYTES + header_length;
        let header = file.get(HEADER_LENGTH_BYTES..data_start).ok_or("RIFE weights: header past the end of the file")?;
        let mut entries: HashMap<String, serde_json::Value> = serde_json::from_slice(header)
            .map_err(|error| format!("RIFE weights: header is not valid JSON: {error}"))?;
        entries.remove(METADATA_KEY);
        let mut names: Vec<String> = entries.keys().cloned().collect();
        names.sort();
        let mut halves = Vec::new();
        let mut offsets = HashMap::new();
        for name in names {
            let info: TensorInfo = serde_json::from_value(entries.remove(&name).unwrap())
                .map_err(|error| format!("RIFE weights: tensor {name}: {error}"))?;
            if info.dtype != F16_DTYPE {
                return Err(format!("RIFE weights: tensor {name} is {}, not {F16_DTYPE}", info.dtype));
            }
            let [start, end] = info.data_offsets;
            let count: usize = info.shape.iter().product();
            let bytes = file.get(data_start + start..data_start + end).filter(|bytes| bytes.len() == count * F16_BYTES)
                .ok_or_else(|| format!("RIFE weights: tensor {name} data does not match its shape"))?;
            let source: Vec<u16> = bytes.chunks_exact(F16_BYTES).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
            offsets.insert(name, halves.len() as u32);
            match info.shape[..] {
                [outputs, inputs, CONV_KERNEL, CONV_KERNEL] => {
                    let taps = CONV_KERNEL * CONV_KERNEL;
                    for k in 0..inputs * taps {
                        halves.extend((0..outputs).map(|output| source[output * inputs * taps + k]));
                    }
                }
                [inputs, outputs, TRANSPOSED_KERNEL, TRANSPOSED_KERNEL] => {
                    let kernel = TRANSPOSED_KERNEL * TRANSPOSED_KERNEL;
                    for phase_y in 0..PHASE_SIDE {
                        for phase_x in 0..PHASE_SIDE {
                            for input in 0..inputs {
                                for a in 0..PHASE_SIDE {
                                    for b in 0..PHASE_SIDE {
                                        let ky = 2 * a + 1 - phase_y;
                                        let kx = 2 * b + 1 - phase_x;
                                        halves.extend((0..outputs).map(|output|
                                            source[(input * outputs + output) * kernel + ky * TRANSPOSED_KERNEL + kx]));
                                    }
                                }
                            }
                        }
                    }
                }
                _ => halves.extend_from_slice(&source),
            }
            if halves.len() % 2 == 1 {
                halves.push(0);
            }
        }
        Ok(Self { halves, offsets })
    }

    /// Where a tensor starts, in halves.
    pub fn offset(&self, name: &str) -> Result<u32, String> {
        self.offsets.get(name).copied().ok_or_else(|| format!("RIFE weights: no tensor {name}"))
    }
}
