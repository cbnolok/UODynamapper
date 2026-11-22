//! Compiles the networks' compute pipelines ahead of use, RIFE's and the SPAN upscaler's, without uploading weights or
//! running anything. An app calls [`warm_up`] once, off its frame path (the first compile on a cold driver cache takes
//! seconds), and builds its models from what it returns ([`crate::rife::RifeModel::with_pipelines`],
//! [`crate::upscale::UpscaleModel::with_pipelines`]), so nothing compiles again when they are first needed. The block
//! encoder is not part of it: its BC7 UltraFast pipeline takes about 6.6 s on a cold cache, so an app compiles it
//! ([`crate::block_encode::MobileBlockEncoder::compile`], with a [`CompileReport`] of its own) only when it will
//! encode blocks. The driver's own shader cache (Mesa's on disk) makes a later warm-up, in another run, much
//! shorter; this crate keeps no pipeline cache of its own.
//!
//! The callback hears each compile twice, as it starts and as it ends, with its name, its place among the
//! [`warm_up_compiles`] (or the block encoder's [`crate::block_encode::BLOCK_ENCODER_COMPILES`]) and, at the end, its
//! time: enough for a progress display.

use std::time::{Duration, Instant};
use crate::rife::{rife_compiles, RifePipelines};
use crate::upscale::{upscale_compiles, UpscalePipelines};
use crate::ConvUnits;

/// The compiles [`warm_up`] reports for the convolutions on `units`.
pub const fn warm_up_compiles(units: ConvUnits) -> usize {
    rife_compiles(units) + upscale_compiles(units)
}

/// A compile starting or ending.
#[derive(Debug, Clone, Copy)]
pub struct PipelineCompile<'a> {
    /// What compiles, as the pipeline's label: `"RIFE conv Stride1/Leaky tile 64x32"` (on the matrix units also
    /// `"RIFE matrix conv Stride1/Residual x64"`), `"RIFE pixel step: merge"`,
    /// `"upscale conv 3x3/Silu tile 64x48"` (on the matrix units `"upscale matrix conv 3x3/Silu"`),
    /// `"upscale prepare"`, `"BC7 encoder (UltraFast)"` (the block
    /// encoder's others: [`crate::block_encode::MobileBlockFormat::compile_name`]). A RIFE conv on the shader cores
    /// compiles two pipelines of one shader (`main` and `reduce`).
    pub name: &'a str,
    /// Its place among the compiles of the run (0-based), and how many there are.
    pub index: usize,
    pub total: usize,
    /// `false` as it starts, `true` once it ended.
    pub done: bool,
    /// How long it took; zero as it starts.
    pub elapsed: Duration,
}

/// Tells a callback of each compile in turn.
pub struct CompileReport<'a> {
    callback: Box<dyn FnMut(PipelineCompile) + 'a>,
    next: usize,
    total: usize,
}

impl<'a> CompileReport<'a> {
    /// A report of `total` compiles to `callback`.
    pub fn new(total: usize, callback: impl FnMut(PipelineCompile) + 'a) -> Self {
        Self { callback: Box::new(callback), next: 0, total }
    }

    /// A report nobody hears.
    pub fn silent(total: usize) -> Self {
        Self::new(total, |_| {})
    }

    /// Runs `make`, the compile `name`, telling the callback as it starts and ends.
    pub fn compile<T>(&mut self, name: &str, make: impl FnOnce() -> T) -> T {
        let (index, total) = (self.next, self.total);
        (self.callback)(PipelineCompile { name, index, total, done: false, elapsed: Duration::ZERO });
        let started = Instant::now();
        let made = make();
        (self.callback)(PipelineCompile { name, index, total, done: true, elapsed: started.elapsed() });
        self.next += 1;
        made
    }
}

/// The networks' pipelines on one device, compiled.
pub struct WarmPipelines {
    pub rife: RifePipelines,
    pub upscale: UpscalePipelines,
}

/// Compiles the networks' pipelines on `device`, the convolutions for `units` ([`ConvUnits::of`] the device), telling
/// `callback` of each of the [`warm_up_compiles`] as it starts and ends.
pub fn warm_up(device: &wgpu::Device, units: ConvUnits, callback: impl FnMut(PipelineCompile)) -> WarmPipelines {
    let mut report = CompileReport::new(warm_up_compiles(units), callback);
    let rife = RifePipelines::new(device, units, &mut report);
    let upscale = UpscalePipelines::new(device, units, &mut report);
    WarmPipelines { rife, upscale }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use super::*;
    use crate::block_encode::{MobileBlockEncoder, MobileBlockFormat, BLOCK_ENCODER_COMPILES};

    /// A callback printing each of `total` compiles as it ends, checking that it started first, and counting it in
    /// `ended`.
    fn printer(total: usize, ended: &Cell<usize>) -> impl FnMut(PipelineCompile) + '_ {
        let mut previous = None;
        move |compile| {
            assert_eq!(compile.total, total);
            if compile.done {
                assert_eq!(previous, Some((compile.index, false)), "{} ends after it starts", compile.name);
                ended.set(ended.get() + 1);
                println!("{:>3}/{} {:>9.1} ms  {}", compile.index + 1, compile.total,
                    compile.elapsed.as_secs_f64() * 1e3, compile.name);
            }
            previous = Some((compile.index, compile.done));
        }
    }

    /// Times each compile of a warm-up, and of the block encoder after it (and its BC1 pipeline). Run it with
    /// `MESA_SHADER_CACHE_DIR` set to a new folder for a cold driver cache, and again for a warm one.
    #[test]
    #[ignore = "requires a GPU adapter; run explicitly to time the warm-up"]
    fn time_the_warm_up() {
        let Ok((crate::Gpu { device, queue }, _, units)) = crate::own_device_and_units() else {
            println!("skipped: no GPU adapter");
            return;
        };
        let started = Instant::now();
        let ended = Cell::new(0);
        warm_up(&device, units, printer(warm_up_compiles(units), &ended));
        assert_eq!(ended.get(), warm_up_compiles(units));
        println!("convolutions on the {units:?}");
        println!("warm-up: {:.1} ms", started.elapsed().as_secs_f64() * 1e3);
        let started = Instant::now();
        let ended = Cell::new(0);
        let mut encoder = MobileBlockEncoder::compile(&device, &queue,
            &mut CompileReport::new(BLOCK_ENCODER_COMPILES, printer(BLOCK_ENCODER_COMPILES, &ended)));
        assert_eq!(ended.get(), BLOCK_ENCODER_COMPILES);
        println!("block encoder: {:.1} ms", started.elapsed().as_secs_f64() * 1e3);
        let ended = Cell::new(0);
        encoder.compile_format(MobileBlockFormat::Bc1, &mut CompileReport::new(1, printer(1, &ended)));
        assert_eq!(ended.get(), 1);
    }
}
