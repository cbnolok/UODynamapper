//! The SPAN weights: the fp16 safetensors file bundled with the binary, read and laid out again for the convolution
//! shader, which reads each kernel tap's weights for consecutive output channels together. The reader follows
//! `rife/weights.rs`, with square kernels of any side (SPAN has 3x3 and 1x1) and no transposed convolutions.

use std::collections::HashMap;
use serde::Deserialize;

/// The 44 tensors the network uses at inference, fp16, named as in spandrel's SPAN: each Conv3XC already folded into
/// its `eval_conv`.
pub(super) const WEIGHTS_FILE: &[u8] = include_bytes!("../../../../assets/models/span_4x_mssim_f16.safetensors");

/// Bytes of the little-endian header length that starts a safetensors file.
const HEADER_LENGTH_BYTES: usize = 8;
/// The optional header entry that is not a tensor.
const METADATA_KEY: &str = "__metadata__";
const F16_DTYPE: &str = "F16";
const F16_BYTES: usize = 2;

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
    /// Reads a safetensors file. A convolution's weight `[out, in, side, side]` becomes `[in * side² + tap][out]`;
    /// other tensors (biases) stay as they are.
    pub fn parse(file: &[u8]) -> Result<Self, String> {
        let length_bytes = file.get(..HEADER_LENGTH_BYTES).ok_or("SPAN weights: file shorter than its header length")?;
        let header_length = u64::from_le_bytes(length_bytes.try_into().unwrap()) as usize;
        let data_start = HEADER_LENGTH_BYTES + header_length;
        let header = file.get(HEADER_LENGTH_BYTES..data_start).ok_or("SPAN weights: header past the end of the file")?;
        let mut entries: HashMap<String, serde_json::Value> = serde_json::from_slice(header)
            .map_err(|error| format!("SPAN weights: header is not valid JSON: {error}"))?;
        entries.remove(METADATA_KEY);
        let mut names: Vec<String> = entries.keys().cloned().collect();
        names.sort();
        let mut halves = Vec::new();
        let mut offsets = HashMap::new();
        for name in names {
            let info: TensorInfo = serde_json::from_value(entries.remove(&name).unwrap())
                .map_err(|error| format!("SPAN weights: tensor {name}: {error}"))?;
            if info.dtype != F16_DTYPE {
                return Err(format!("SPAN weights: tensor {name} is {}, not {F16_DTYPE}", info.dtype));
            }
            let [start, end] = info.data_offsets;
            let count: usize = info.shape.iter().product();
            let bytes = file.get(data_start + start..data_start + end).filter(|bytes| bytes.len() == count * F16_BYTES)
                .ok_or_else(|| format!("SPAN weights: tensor {name} data does not match its shape"))?;
            let source: Vec<u16> = bytes.chunks_exact(F16_BYTES).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
            offsets.insert(name, halves.len() as u32);
            match info.shape[..] {
                [outputs, inputs, side_y, side_x] if side_y == side_x => {
                    let taps = side_y * side_x;
                    for k in 0..inputs * taps {
                        halves.extend((0..outputs).map(|output| source[output * inputs * taps + k]));
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
        self.offsets.get(name).copied().ok_or_else(|| format!("SPAN weights: no tensor {name}"))
    }
}
