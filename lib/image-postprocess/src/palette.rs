//! Palette-safe sprite upscaling support.
//!
//! The types in this module separate decoded sprite semantics from generic
//! RGBA texels. Existing RGBA upscalers can be wrapped through
//! [`RgbaFilterScaler`], while future algorithm-specific implementations can
//! consume [`PaletteSemanticContext`] directly for edge and interpolation
//! decisions.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use crate::upscaling::UpscaleFilter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ColorId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RampId(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MaterialId(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const TRANSPARENT: Self = Self { r: 0, g: 0, b: 0, a: 0 };

    pub fn from_slice(bytes: &[u8]) -> Self {
        Self { r: bytes[0], g: bytes[1], b: bytes[2], a: bytes[3] }
    }

    pub fn write_to(self, out: &mut [u8]) {
        out[0] = self.r;
        out[1] = self.g;
        out[2] = self.b;
        out[3] = self.a;
    }

    pub fn rgb_key(self) -> (u8, u8, u8) {
        (self.r, self.g, self.b)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransparencyState {
    Opaque,
    Transparent,
    ColorKey,
}

impl TransparencyState {
    pub fn is_transparent(self) -> bool {
        matches!(self, Self::Transparent | Self::ColorKey)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistanceMetric {
    WeightedSrgb,
}

impl Default for DistanceMetric {
    fn default() -> Self {
        Self::WeightedSrgb
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapMode {
    StrictSnap,
    RampAwareSnap,
    ExpandedPalette { max_derived_colors: usize },
    NoSnap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntermediateColorPolicy {
    ImmediateSnap,
    DeferredSnap,
    HybridSnap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DitherPolicy {
    Off,
    DetectOnly,
    CollapseToRamp,
    PreserveButConstrain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputAlphaPolicy {
    SourceNearest,
    ScalerAlpha,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransparencyPolicy {
    pub alpha_threshold: u8,
    pub color_key: Option<[u8; 3]>,
    pub sanitize_hidden_rgb: bool,
    pub compare_transparent_as_equal: bool,
    pub output_alpha: OutputAlphaPolicy,
}

impl Default for TransparencyPolicy {
    fn default() -> Self {
        Self {
            alpha_threshold: 0,
            color_key: None,
            sanitize_hidden_rgb: true,
            compare_transparent_as_equal: true,
            output_alpha: OutputAlphaPolicy::SourceNearest,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizationConfig {
    pub remap_to_canonical: bool,
    pub collapse_near_equivalent: bool,
    pub near_equivalent_distance: u32,
    pub infer_ramps: bool,
    pub detect_dither: bool,
}

impl Default for NormalizationConfig {
    fn default() -> Self {
        Self {
            remap_to_canonical: true,
            collapse_near_equivalent: false,
            near_equivalent_distance: 32,
            infer_ramps: false,
            detect_dither: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteUpscaleConfig {
    pub snap_mode: SnapMode,
    pub intermediate_policy: IntermediateColorPolicy,
    pub dither_policy: DitherPolicy,
    pub transparency: TransparencyPolicy,
    pub normalization: NormalizationConfig,
    pub cleanup_isolated_illegal_pixels: bool,
}

impl Default for PaletteUpscaleConfig {
    fn default() -> Self {
        Self {
            snap_mode: SnapMode::StrictSnap,
            intermediate_policy: IntermediateColorPolicy::DeferredSnap,
            dither_policy: DitherPolicy::PreserveButConstrain,
            transparency: TransparencyPolicy::default(),
            normalization: NormalizationConfig::default(),
            cleanup_isolated_illegal_pixels: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteColor {
    pub id: ColorId,
    pub rgba: Rgba8,
    pub ramp: Option<RampId>,
    pub material: Option<MaterialId>,
    pub derived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForbiddenTransition {
    pub from: ColorId,
    pub to: ColorId,
}

#[derive(Debug, Clone)]
pub struct PaletteModel {
    colors: Vec<PaletteColor>,
    metric: DistanceMetric,
    forbidden: BTreeSet<(ColorId, ColorId)>,
}

impl PaletteModel {
    pub fn new(colors: Vec<PaletteColor>, metric: DistanceMetric) -> Self {
        let mut by_id = BTreeMap::new();
        for color in colors {
            by_id.entry(color.id).or_insert(color);
        }

        Self {
            colors: by_id.into_values().collect(),
            metric,
            forbidden: BTreeSet::new(),
        }
    }

    pub fn from_rgba(width: u32, height: u32, rgba: &[u8], transparency: &TransparencyPolicy) -> Result<Self, PaletteError> {
        validate_rgba_len(width, height, rgba)?;

        let mut colors = BTreeMap::new();
        for chunk in rgba.chunks_exact(4) {
            let color = Rgba8::from_slice(chunk);
            if classify_transparency(color, transparency).is_transparent() {
                continue;
            }
            let next_id = ColorId(colors.len() as u32);
            colors.entry(color.rgb_key()).or_insert_with(|| PaletteColor {
                id: next_id,
                rgba: Rgba8 { a: 255, ..color },
                ramp: None,
                material: None,
                derived: false,
            });
        }

        Ok(Self::new(colors.into_values().collect(), DistanceMetric::default()))
    }

    pub fn colors(&self) -> &[PaletteColor] {
        &self.colors
    }

    pub fn color(&self, id: ColorId) -> Option<&PaletteColor> {
        self.colors.iter().find(|color| color.id == id)
    }

    pub fn add_forbidden_transition(&mut self, from: ColorId, to: ColorId) {
        self.forbidden.insert((from, to));
        self.forbidden.insert((to, from));
    }

    pub fn nearest_color(&self, candidate: Rgba8) -> Option<(ColorId, u32)> {
        nearest_color_in(candidate, self.colors.iter(), self.metric)
    }

    pub fn nearest_color_for_ramp(&self, candidate: Rgba8, ramp: Option<RampId>) -> Option<(ColorId, u32)> {
        let Some(ramp) = ramp else {
            return self.nearest_color(candidate);
        };

        nearest_color_in(
            candidate,
            self.colors.iter().filter(|color| color.ramp == Some(ramp)),
            self.metric,
        )
        .or_else(|| self.nearest_color(candidate))
    }

    pub fn is_forbidden_transition(&self, a: ColorId, b: ColorId) -> bool {
        self.forbidden.contains(&(a, b))
    }

    pub fn with_derived_palette(&self, max_derived_colors: usize) -> Self {
        if max_derived_colors == 0 {
            return self.clone();
        }

        let mut next = self.clone();
        let mut seen = next.colors.iter().map(|color| color.rgb_key()).collect::<BTreeSet<_>>();
        let base = self.colors.iter().filter(|color| !color.derived).collect::<Vec<_>>();
        let mut produced = 0usize;

        'outer: for left in 0..base.len() {
            for right in (left + 1)..base.len() {
                let a = base[left];
                let b = base[right];
                if a.ramp != b.ramp {
                    continue;
                }

                let rgba = Rgba8 {
                    r: average_u8(a.rgba.r, b.rgba.r),
                    g: average_u8(a.rgba.g, b.rgba.g),
                    b: average_u8(a.rgba.b, b.rgba.b),
                    a: 255,
                };
                if !seen.insert(rgba.rgb_key()) {
                    continue;
                }

                next.colors.push(PaletteColor {
                    id: ColorId(1_000_000 + produced as u32),
                    rgba,
                    ramp: a.ramp,
                    material: a.material,
                    derived: true,
                });
                produced += 1;
                if produced >= max_derived_colors {
                    break 'outer;
                }
            }
        }

        next
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexedPixel {
    pub color_id: Option<ColorId>,
    pub rgba: Rgba8,
    pub transparency: TransparencyState,
    pub ramp: Option<RampId>,
    pub material: Option<MaterialId>,
    pub dither_pair: Option<(ColorId, ColorId)>,
}

impl IndexedPixel {
    pub fn is_transparent(self) -> bool {
        self.transparency.is_transparent()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<IndexedPixel>,
}

impl IndexedImage {
    pub fn from_rgba_with_palette(
        width: u32,
        height: u32,
        rgba: &[u8],
        palette: &PaletteModel,
        config: &PaletteUpscaleConfig,
    ) -> Result<Self, PaletteError> {
        validate_rgba_len(width, height, rgba)?;

        let mut pixels = Vec::with_capacity((width * height) as usize);
        for chunk in rgba.chunks_exact(4) {
            let mut color = Rgba8::from_slice(chunk);
            let transparency = classify_transparency(color, &config.transparency);
            if transparency.is_transparent() {
                if config.transparency.sanitize_hidden_rgb {
                    color = Rgba8::TRANSPARENT;
                }
                pixels.push(IndexedPixel {
                    color_id: None,
                    rgba: color,
                    transparency,
                    ramp: None,
                    material: None,
                    dither_pair: None,
                });
                continue;
            }

            let (color_id, _) = palette.nearest_color(color).ok_or(PaletteError::EmptyPalette)?;
            let canonical = palette.color(color_id).ok_or(PaletteError::MissingColor(color_id))?;
            let use_canonical = config.normalization.remap_to_canonical || config.normalization.collapse_near_equivalent;
            pixels.push(IndexedPixel {
                color_id: Some(color_id),
                rgba: if use_canonical { canonical.rgba } else { Rgba8 { a: 255, ..color } },
                transparency: TransparencyState::Opaque,
                ramp: canonical.ramp,
                material: canonical.material,
                dither_pair: None,
            });
        }

        let mut image = Self { width, height, pixels };
        if matches!(config.dither_policy, DitherPolicy::DetectOnly | DitherPolicy::CollapseToRamp | DitherPolicy::PreserveButConstrain)
            || config.normalization.detect_dither
        {
            image.mark_checkerboard_dither();
        }
        Ok(image)
    }

    pub fn to_rgba(&self, sanitize_transparent: bool) -> Vec<u8> {
        let mut out = vec![0u8; self.pixels.len() * 4];
        for (index, pixel) in self.pixels.iter().copied().enumerate() {
            let mut color = pixel.rgba;
            if sanitize_transparent && pixel.is_transparent() {
                color = Rgba8::TRANSPARENT;
            }
            color.write_to(&mut out[index * 4..index * 4 + 4]);
        }
        out
    }

    pub fn pixel(&self, x: u32, y: u32) -> IndexedPixel {
        self.pixels[(y * self.width + x) as usize]
    }

    fn mark_checkerboard_dither(&mut self) {
        if self.width < 2 || self.height < 2 {
            return;
        }

        for y in 0..self.height - 1 {
            for x in 0..self.width - 1 {
                let i0 = (y * self.width + x) as usize;
                let i1 = i0 + 1;
                let i2 = i0 + self.width as usize;
                let i3 = i2 + 1;
                let Some(a) = self.pixels[i0].color_id else { continue; };
                let Some(b) = self.pixels[i1].color_id else { continue; };
                if a == b {
                    continue;
                }
                if self.pixels[i2].color_id == Some(b) && self.pixels[i3].color_id == Some(a) {
                    let pair = ordered_pair(a, b);
                    self.pixels[i0].dither_pair = Some(pair);
                    self.pixels[i1].dither_pair = Some(pair);
                    self.pixels[i2].dither_pair = Some(pair);
                    self.pixels[i3].dither_pair = Some(pair);
                }
            }
        }
    }
}

pub trait PaletteSemanticContext {
    fn equivalent_color(&self, a: IndexedPixel, b: IndexedPixel) -> bool;
    fn same_ramp(&self, a: IndexedPixel, b: IndexedPixel) -> bool;
    fn legal_transition(&self, a: IndexedPixel, b: IndexedPixel) -> bool;
    fn edge_distance(&self, a: IndexedPixel, b: IndexedPixel) -> u32;
    fn snap_candidate(&self, candidate: Rgba8, ramp_hint: Option<RampId>) -> Option<(ColorId, Rgba8, u32)>;
}

pub struct PaletteContext<'a> {
    pub palette: &'a PaletteModel,
    pub config: &'a PaletteUpscaleConfig,
}

impl PaletteSemanticContext for PaletteContext<'_> {
    fn equivalent_color(&self, a: IndexedPixel, b: IndexedPixel) -> bool {
        if a.is_transparent() || b.is_transparent() {
            return self.config.transparency.compare_transparent_as_equal && a.is_transparent() && b.is_transparent();
        }
        a.color_id == b.color_id
    }

    fn same_ramp(&self, a: IndexedPixel, b: IndexedPixel) -> bool {
        !a.is_transparent() && !b.is_transparent() && a.ramp.is_some() && a.ramp == b.ramp
    }

    fn legal_transition(&self, a: IndexedPixel, b: IndexedPixel) -> bool {
        match (a.color_id, b.color_id) {
            (Some(left), Some(right)) => !self.palette.is_forbidden_transition(left, right),
            _ => true,
        }
    }

    fn edge_distance(&self, a: IndexedPixel, b: IndexedPixel) -> u32 {
        if a.is_transparent() || b.is_transparent() {
            return if a.is_transparent() == b.is_transparent() { 0 } else { u32::MAX / 4 };
        }
        weighted_distance(a.rgba, b.rgba, self.palette.metric)
    }

    fn snap_candidate(&self, candidate: Rgba8, ramp_hint: Option<RampId>) -> Option<(ColorId, Rgba8, u32)> {
        let (id, distance) = match self.config.snap_mode {
            SnapMode::StrictSnap => self.palette.nearest_color(candidate)?,
            SnapMode::RampAwareSnap => self.palette.nearest_color_for_ramp(candidate, ramp_hint)?,
            SnapMode::ExpandedPalette { .. } => self.palette.nearest_color_for_ramp(candidate, ramp_hint)?,
            SnapMode::NoSnap => return None,
        };
        let rgba = self.palette.color(id)?.rgba;
        Some((id, rgba, distance))
    }
}

pub trait PaletteScaler {
    fn name(&self) -> &'static str;
    fn scale_factor(&self) -> u32;
    fn scale(&self, image: &IndexedImage, palette: &PaletteModel, config: &PaletteUpscaleConfig) -> PaletteScaleOutput;
}

#[derive(Debug, Clone)]
pub struct PaletteScaleOutput {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct RgbaFilterScaler {
    filter: UpscaleFilter,
}

impl RgbaFilterScaler {
    pub fn new(filter: UpscaleFilter) -> Self {
        Self { filter }
    }
}

impl PaletteScaler for RgbaFilterScaler {
    fn name(&self) -> &'static str {
        "rgba-filter"
    }

    fn scale_factor(&self) -> u32 {
        self.filter.scale_factor()
    }

    fn scale(&self, image: &IndexedImage, _palette: &PaletteModel, config: &PaletteUpscaleConfig) -> PaletteScaleOutput {
        let rgba = image.to_rgba(config.transparency.sanitize_hidden_rgb);
        let (width, height, rgba) = self.filter.apply(image.width, image.height, &rgba);
        PaletteScaleOutput { width, height, rgba }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteUpscaleResult {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub diagnostics: PaletteDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PaletteDiagnostics {
    pub stages: Vec<StageMetrics>,
    pub snap_histogram: BTreeMap<ColorId, u64>,
    pub off_palette_candidates: u64,
    pub edge_pixels_touching_transparency: u64,
    pub ramp_violations: u64,
    pub forbidden_transition_violations: u64,
    pub dither_cells: u64,
    pub algorithm_timings: Vec<AlgorithmTiming>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageMetrics {
    pub stage: &'static str,
    pub distinct_opaque_colors: usize,
    pub transparent_pixels: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlgorithmTiming {
    pub name: &'static str,
    pub elapsed_micros: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteError {
    InvalidRgbaLength { expected: usize, actual: usize },
    EmptyPalette,
    MissingColor(ColorId),
}

pub fn palette_safe_upscale<S: PaletteScaler>(
    width: u32,
    height: u32,
    rgba: &[u8],
    source_palette: Option<&PaletteModel>,
    scaler: &S,
    config: &PaletteUpscaleConfig,
) -> Result<PaletteUpscaleResult, PaletteError> {
    let base_palette;
    let palette = match source_palette {
        Some(palette) => palette.clone(),
        None => {
            base_palette = PaletteModel::from_rgba(width, height, rgba, &config.transparency)?;
            base_palette
        }
    };

    let snap_palette = match config.snap_mode {
        SnapMode::ExpandedPalette { max_derived_colors } => palette.with_derived_palette(max_derived_colors),
        _ => palette.clone(),
    };

    let source = IndexedImage::from_rgba_with_palette(width, height, rgba, &palette, config)?;
    let mut diagnostics = PaletteDiagnostics::default();
    diagnostics.stages.push(stage_metrics("decoded", width, height, rgba, &config.transparency)?);
    diagnostics.edge_pixels_touching_transparency = count_alpha_edges(&source);
    diagnostics.dither_cells = count_dither_cells(&source);

    let scale_started = Instant::now();
    let scaled = scaler.scale(&source, &snap_palette, config);
    diagnostics.algorithm_timings.push(AlgorithmTiming {
        name: scaler.name(),
        elapsed_micros: scale_started.elapsed().as_micros(),
    });
    diagnostics.stages.push(stage_metrics("scaled-working", scaled.width, scaled.height, &scaled.rgba, &config.transparency)?);

    let mut snapped = restrict_output_palette(&scaled, &source, &snap_palette, config, &mut diagnostics)?;
    if config.cleanup_isolated_illegal_pixels && !matches!(config.snap_mode, SnapMode::NoSnap) {
        cleanup_isolated_pixels(scaled.width, scaled.height, &mut snapped, &snap_palette, &mut diagnostics);
    }

    diagnostics.stages.push(stage_metrics("final", scaled.width, scaled.height, &snapped, &config.transparency)?);
    Ok(PaletteUpscaleResult {
        width: scaled.width,
        height: scaled.height,
        rgba: snapped,
        diagnostics,
    })
}

fn restrict_output_palette(
    scaled: &PaletteScaleOutput,
    source: &IndexedImage,
    palette: &PaletteModel,
    config: &PaletteUpscaleConfig,
    diagnostics: &mut PaletteDiagnostics,
) -> Result<Vec<u8>, PaletteError> {
    validate_rgba_len(scaled.width, scaled.height, &scaled.rgba)?;

    if matches!(config.snap_mode, SnapMode::NoSnap) {
        return Ok(scaled.rgba.clone());
    }

    let mut out = vec![0u8; scaled.rgba.len()];
    for y in 0..scaled.height {
        for x in 0..scaled.width {
            let out_index = ((y * scaled.width + x) as usize) * 4;
            let source_pixel = nearest_source_pixel(source, scaled.width, scaled.height, x, y);
            let candidate = Rgba8::from_slice(&scaled.rgba[out_index..out_index + 4]);

            if matches!(config.transparency.output_alpha, OutputAlphaPolicy::SourceNearest) && source_pixel.is_transparent() {
                Rgba8::TRANSPARENT.write_to(&mut out[out_index..out_index + 4]);
                continue;
            }

            let transparent = classify_transparency(candidate, &config.transparency).is_transparent();
            if transparent {
                Rgba8::TRANSPARENT.write_to(&mut out[out_index..out_index + 4]);
                continue;
            }

            let ramp_hint = match config.snap_mode {
                SnapMode::RampAwareSnap | SnapMode::ExpandedPalette { .. } => source_pixel.ramp,
                SnapMode::StrictSnap | SnapMode::NoSnap => None,
            };
            let (id, distance) = match config.snap_mode {
                SnapMode::StrictSnap => palette.nearest_color(candidate).ok_or(PaletteError::EmptyPalette)?,
                SnapMode::RampAwareSnap | SnapMode::ExpandedPalette { .. } => {
                    palette.nearest_color_for_ramp(candidate, ramp_hint).ok_or(PaletteError::EmptyPalette)?
                }
                SnapMode::NoSnap => unreachable!(),
            };

            let snapped = palette.color(id).ok_or(PaletteError::MissingColor(id))?.rgba;
            if distance != 0 {
                diagnostics.off_palette_candidates += 1;
            }
            *diagnostics.snap_histogram.entry(id).or_insert(0) += 1;
            Rgba8 { a: 255, ..snapped }.write_to(&mut out[out_index..out_index + 4]);
        }
    }

    count_output_violations(scaled.width, scaled.height, &out, palette, config, diagnostics)?;
    Ok(out)
}

fn cleanup_isolated_pixels(width: u32, height: u32, rgba: &mut [u8], palette: &PaletteModel, diagnostics: &mut PaletteDiagnostics) {
    if width < 3 || height < 3 {
        return;
    }

    let original = rgba.to_vec();
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let idx = ((y * width + x) as usize) * 4;
            let center = Rgba8::from_slice(&original[idx..idx + 4]);
            if center.a == 0 {
                continue;
            }

            let neighbors = [
                Rgba8::from_slice(&original[(((y - 1) * width + x) as usize) * 4..(((y - 1) * width + x) as usize) * 4 + 4]),
                Rgba8::from_slice(&original[(((y + 1) * width + x) as usize) * 4..(((y + 1) * width + x) as usize) * 4 + 4]),
                Rgba8::from_slice(&original[((y * width + x - 1) as usize) * 4..((y * width + x - 1) as usize) * 4 + 4]),
                Rgba8::from_slice(&original[((y * width + x + 1) as usize) * 4..((y * width + x + 1) as usize) * 4 + 4]),
            ];

            let mut counts = BTreeMap::new();
            for neighbor in neighbors {
                if neighbor.a != 0 {
                    *counts.entry(neighbor.rgb_key()).or_insert(0usize) += 1;
                }
            }

            let Some((rgb, count)) = counts.into_iter().max_by_key(|(_, count)| *count) else { continue; };
            if count >= 3 && rgb != center.rgb_key() {
                let replacement = Rgba8 { r: rgb.0, g: rgb.1, b: rgb.2, a: 255 };
                if palette.nearest_color(replacement).is_some() {
                    replacement.write_to(&mut rgba[idx..idx + 4]);
                    diagnostics.off_palette_candidates += 1;
                }
            }
        }
    }
}

fn count_output_violations(
    width: u32,
    height: u32,
    rgba: &[u8],
    palette: &PaletteModel,
    config: &PaletteUpscaleConfig,
    diagnostics: &mut PaletteDiagnostics,
) -> Result<(), PaletteError> {
    validate_rgba_len(width, height, rgba)?;
    let indexed = IndexedImage::from_rgba_with_palette(width, height, rgba, palette, config)?;
    if width == 0 || height == 0 {
        return Ok(());
    }

    for y in 0..height {
        for x in 0..width {
            let pixel = indexed.pixel(x, y);
            if x + 1 < width {
                let right = indexed.pixel(x + 1, y);
                if let (Some(left_id), Some(right_id)) = (pixel.color_id, right.color_id) {
                    if palette.is_forbidden_transition(left_id, right_id) {
                        diagnostics.forbidden_transition_violations += 1;
                    }
                    if pixel.ramp.is_some() && right.ramp.is_some() && pixel.ramp != right.ramp {
                        diagnostics.ramp_violations += 1;
                    }
                }
            }
            if y + 1 < height {
                let down = indexed.pixel(x, y + 1);
                if let (Some(top_id), Some(down_id)) = (pixel.color_id, down.color_id) {
                    if palette.is_forbidden_transition(top_id, down_id) {
                        diagnostics.forbidden_transition_violations += 1;
                    }
                    if pixel.ramp.is_some() && down.ramp.is_some() && pixel.ramp != down.ramp {
                        diagnostics.ramp_violations += 1;
                    }
                }
            }
        }
    }
    Ok(())
}

fn nearest_source_pixel(source: &IndexedImage, out_width: u32, out_height: u32, x: u32, y: u32) -> IndexedPixel {
    let sx = ((u64::from(x) * u64::from(source.width)) / u64::from(out_width)).min(u64::from(source.width.saturating_sub(1))) as u32;
    let sy = ((u64::from(y) * u64::from(source.height)) / u64::from(out_height)).min(u64::from(source.height.saturating_sub(1))) as u32;
    source.pixel(sx, sy)
}

fn stage_metrics(stage: &'static str, width: u32, height: u32, rgba: &[u8], transparency: &TransparencyPolicy) -> Result<StageMetrics, PaletteError> {
    validate_rgba_len(width, height, rgba)?;
    let mut colors = BTreeSet::new();
    let mut transparent_pixels = 0u64;

    for chunk in rgba.chunks_exact(4) {
        let color = Rgba8::from_slice(chunk);
        if classify_transparency(color, transparency).is_transparent() {
            transparent_pixels += 1;
        } else {
            colors.insert(color.rgb_key());
        }
    }

    Ok(StageMetrics {
        stage,
        distinct_opaque_colors: colors.len(),
        transparent_pixels,
    })
}

fn count_alpha_edges(image: &IndexedImage) -> u64 {
    let mut count = 0u64;
    for y in 0..image.height {
        for x in 0..image.width {
            let pixel = image.pixel(x, y);
            if pixel.is_transparent() {
                continue;
            }
            let mut touches_transparency = false;
            if x > 0 {
                touches_transparency |= image.pixel(x - 1, y).is_transparent();
            }
            if y > 0 {
                touches_transparency |= image.pixel(x, y - 1).is_transparent();
            }
            if x + 1 < image.width {
                touches_transparency |= image.pixel(x + 1, y).is_transparent();
            }
            if y + 1 < image.height {
                touches_transparency |= image.pixel(x, y + 1).is_transparent();
            }
            if touches_transparency {
                count += 1;
            }
        }
    }
    count
}

fn count_dither_cells(image: &IndexedImage) -> u64 {
    image.pixels.iter().filter(|pixel| pixel.dither_pair.is_some()).count() as u64
}

fn classify_transparency(color: Rgba8, policy: &TransparencyPolicy) -> TransparencyState {
    if let Some(key) = policy.color_key {
        if color.r == key[0] && color.g == key[1] && color.b == key[2] {
            return TransparencyState::ColorKey;
        }
    }
    if color.a <= policy.alpha_threshold {
        TransparencyState::Transparent
    } else {
        TransparencyState::Opaque
    }
}

fn validate_rgba_len(width: u32, height: u32, rgba: &[u8]) -> Result<(), PaletteError> {
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(PaletteError::InvalidRgbaLength { expected, actual: rgba.len() });
    }
    Ok(())
}

fn nearest_color_in<'a, I>(candidate: Rgba8, colors: I, metric: DistanceMetric) -> Option<(ColorId, u32)>
where
    I: IntoIterator<Item = &'a PaletteColor>,
{
    colors
        .into_iter()
        .map(|color| (color.id, weighted_distance(candidate, color.rgba, metric)))
        .min_by_key(|(id, distance)| (*distance, *id))
}

fn weighted_distance(a: Rgba8, b: Rgba8, metric: DistanceMetric) -> u32 {
    match metric {
        DistanceMetric::WeightedSrgb => {
            let dr = i32::from(a.r) - i32::from(b.r);
            let dg = i32::from(a.g) - i32::from(b.g);
            let db = i32::from(a.b) - i32::from(b.b);
            (9 * dr * dr + 16 * dg * dg + 4 * db * db) as u32
        }
    }
}

fn ordered_pair(a: ColorId, b: ColorId) -> (ColorId, ColorId) {
    if a <= b { (a, b) } else { (b, a) }
}

fn average_u8(a: u8, b: u8) -> u8 {
    ((u16::from(a) + u16::from(b)) / 2) as u8
}

trait RgbKey {
    fn rgb_key(&self) -> (u8, u8, u8);
}

impl RgbKey for PaletteColor {
    fn rgb_key(&self) -> (u8, u8, u8) {
        self.rgba.rgb_key()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_safe_strict_snap_removes_bilinear_colors() {
        let rgba = vec![
            255, 0, 0, 255, 0, 0, 255, 255,
            0, 0, 255, 255, 255, 0, 0, 255,
        ];
        let config = PaletteUpscaleConfig::default();
        let scaler = RgbaFilterScaler::new(UpscaleFilter::Bilinear2x);

        let result = palette_safe_upscale(2, 2, &rgba, None, &scaler, &config).unwrap();

        assert_eq!((result.width, result.height), (4, 4));
        assert!(result.diagnostics.off_palette_candidates > 0);
        for pixel in result.rgba.chunks_exact(4) {
            assert!(pixel == [255, 0, 0, 255] || pixel == [0, 0, 255, 255]);
        }
    }

    #[test]
    fn palette_safe_default_sanitizes_transparent_border_halo() {
        let rgba = vec![
            255, 0, 0, 0, 0, 180, 0, 255,
            255, 0, 0, 0, 0, 180, 0, 255,
        ];
        let config = PaletteUpscaleConfig::default();
        let scaler = RgbaFilterScaler::new(UpscaleFilter::Bilinear2x);

        let result = palette_safe_upscale(2, 2, &rgba, None, &scaler, &config).unwrap();

        for y in 0..result.height {
            for x in 0..result.width {
                let idx = ((y * result.width + x) as usize) * 4;
                if x < 2 {
                    assert_eq!(&result.rgba[idx..idx + 4], &[0, 0, 0, 0]);
                } else {
                    assert_eq!(&result.rgba[idx..idx + 4], &[0, 180, 0, 255]);
                }
            }
        }
    }

    #[test]
    fn expanded_palette_is_bounded_and_deterministic() {
        let colors = vec![
            PaletteColor { id: ColorId(0), rgba: Rgba8 { r: 0, g: 0, b: 0, a: 255 }, ramp: Some(RampId(1)), material: None, derived: false },
            PaletteColor { id: ColorId(1), rgba: Rgba8 { r: 100, g: 100, b: 100, a: 255 }, ramp: Some(RampId(1)), material: None, derived: false },
            PaletteColor { id: ColorId(2), rgba: Rgba8 { r: 200, g: 200, b: 200, a: 255 }, ramp: Some(RampId(1)), material: None, derived: false },
        ];
        let palette = PaletteModel::new(colors, DistanceMetric::WeightedSrgb);

        let expanded_a = palette.with_derived_palette(2);
        let expanded_b = palette.with_derived_palette(2);

        assert_eq!(expanded_a.colors(), expanded_b.colors());
        assert_eq!(expanded_a.colors().iter().filter(|color| color.derived).count(), 2);
    }

    #[test]
    fn diagnostics_report_dither_and_alpha_edges() {
        let rgba = vec![
            10, 10, 10, 255, 20, 20, 20, 255, 0, 0, 0, 0,
            20, 20, 20, 255, 10, 10, 10, 255, 0, 0, 0, 0,
        ];
        let config = PaletteUpscaleConfig {
            dither_policy: DitherPolicy::DetectOnly,
            ..PaletteUpscaleConfig::default()
        };
        let scaler = RgbaFilterScaler::new(UpscaleFilter::Nearest2x);

        let result = palette_safe_upscale(3, 2, &rgba, None, &scaler, &config).unwrap();

        assert!(result.diagnostics.dither_cells >= 4);
        assert!(result.diagnostics.edge_pixels_touching_transparency >= 2);
        assert!(result.diagnostics.stages.iter().any(|stage| stage.stage == "final"));
    }
}
