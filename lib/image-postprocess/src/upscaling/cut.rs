//! Cheap Upscaling Triangulation 2x filters.
//!
//! Reference: https://github.com/Swordfish90/cheap-upscaling-triangulation

const EPSILON: f32 = 0.02;
const CUT1_EDGE_MIN_VALUE: f32 = 0.05;
const CUT1_BLEND_MIN_CONTRAST_EDGE: f32 = 0.25;
const CUT1_BLEND_MAX_CONTRAST_EDGE: f32 = 0.50;
const CUT1_BLEND_MIN_SHARPNESS: f32 = 0.0;
const CUT1_BLEND_MAX_SHARPNESS: f32 = 0.50;
const CUT2_BLEND_MIN_CONTRAST_EDGE: f32 = 0.0;
const CUT2_BLEND_MAX_CONTRAST_EDGE: f32 = 0.25;
const CUT2_BLEND_MIN_SHARPNESS: f32 = 0.0;
const CUT2_BLEND_MAX_SHARPNESS: f32 = 0.75;
const HARD_EDGES_SEARCH_MAX_ERROR: f32 = 0.25;
const CUT3_HARD_EDGES_SEARCH_MAX_DISTANCE: i32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CutMode {
    Cut1,
    Cut2,
    Cut3,
}

#[derive(Clone, Copy, Debug)]
struct Pixel {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

impl Pixel {
    fn from_rgba(rgba: &[u8], index: usize) -> Self {
        let base = index * 4;
        Self {
            r: f32::from(rgba[base]) / 255.0,
            g: f32::from(rgba[base + 1]) / 255.0,
            b: f32::from(rgba[base + 2]) / 255.0,
            a: f32::from(rgba[base + 3]) / 255.0,
        }
    }

    fn luma(self) -> f32 {
        0.299 * self.r + 0.587 * self.g + 0.114 * self.b
    }

    fn mix(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            r: mix(self.r, other.r, t),
            g: mix(self.g, other.g, t),
            b: mix(self.b, other.b, t),
            a: mix(self.a, other.a, t),
        }
    }

    fn write_rgba(self, out: &mut [u8], index: usize) {
        let base = index * 4;
        out[base] = float_to_u8(self.r);
        out[base + 1] = float_to_u8(self.g);
        out[base + 2] = float_to_u8(self.b);
        out[base + 3] = float_to_u8(self.a);
    }
}

#[derive(Clone, Copy, Debug)]
struct Pixels {
    p0: Pixel,
    p1: Pixel,
    p2: Pixel,
    p3: Pixel,
}

#[derive(Clone, Copy, Debug)]
struct CutFlags {
    triangle: bool,
    flip: bool,
    edges: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
struct Quad {
    scores: [f32; 4],
    max_edge_contrast: f32,
    max_score: f32,
}

#[derive(Clone, Copy, Debug)]
struct Cut3Cell {
    pattern: i32,
    soft_edges: [f32; 4],
}

pub fn apply_cut(width: u32, height: u32, rgba: &[u8], mode: CutMode) -> (u32, u32, Vec<u8>) {
    let source: Vec<Pixel> = (0..(width * height) as usize)
        .map(|index| Pixel::from_rgba(rgba, index))
        .collect();
    let flags = match mode {
        CutMode::Cut1 => None,
        CutMode::Cut2 => Some(cut2_flags(width, height, &source)),
        CutMode::Cut3 => Some(cut3_flags(width, height, &source)),
    };
    let out_width = width * 2;
    let out_height = height * 2;
    let mut out = vec![0; (out_width * out_height * 4) as usize];

    for y in 0..out_height {
        for x in 0..out_width {
            let cell_x = (x / 2).min(width.saturating_sub(1));
            let cell_y = (y / 2).min(height.saturating_sub(1));
            let px_coords = ((x % 2) as f32 * 0.5 + 0.25, (y % 2) as f32 * 0.5 + 0.25);
            let pixels = cell_pixels(width, height, &source, cell_x, cell_y);
            let pixel = if let Some(flags) = flags.as_ref() {
                let flag = flags[(cell_y * width + cell_x) as usize];
                render_cut23_pixel(pixels, flag, px_coords)
            } else {
                render_cut1_pixel(pixels, px_coords)
            };
            pixel.write_rgba(&mut out, (y * out_width + x) as usize);
        }
    }

    (out_width, out_height, out)
}

fn render_cut1_pixel(mut pixels: Pixels, mut px_coords: (f32, f32)) -> Pixel {
    let l0 = pixels.p0.luma();
    let l1 = pixels.p1.luma();
    let l2 = pixels.p2.luma();
    let l3 = pixels.p3.luma();
    let d0_3 = (l0 - l3).abs() * 2.0 + CUT1_EDGE_MIN_VALUE < (l1 - l2).abs();
    let d1_2 = (l1 - l2).abs() * 2.0 + CUT1_EDGE_MIN_VALUE < (l0 - l3).abs();

    if d1_2 {
        pixels = Pixels {
            p0: pixels.p1,
            p1: pixels.p0,
            p2: pixels.p3,
            p3: pixels.p2,
        };
        px_coords.0 = 1.0 - px_coords.0;
    }

    let weights = if d0_3 || d1_2 {
        if px_coords.1 > px_coords.0 {
            pixels = Pixels {
                p0: pixels.p0,
                p1: pixels.p2,
                p2: pixels.p2,
                p3: pixels.p3,
            };
            triangle_weights(px_coords)
        } else {
            pixels = Pixels {
                p0: pixels.p0,
                p1: pixels.p1,
                p2: pixels.p1,
                p3: pixels.p3,
            };
            triangle_weights((px_coords.1, px_coords.0))
        }
    } else {
        [px_coords.0, px_coords.0, px_coords.1]
    };

    blend_cut1(
        blend_cut1(pixels.p0, pixels.p1, weights[0]),
        blend_cut1(pixels.p2, pixels.p3, weights[1]),
        weights[2],
    )
}

fn render_cut23_pixel(mut pixels: Pixels, flags: CutFlags, mut px_coords: (f32, f32)) -> Pixel {
    let mut edges = flags.edges;
    if flags.flip {
        pixels = Pixels {
            p0: pixels.p1,
            p1: pixels.p0,
            p2: pixels.p3,
            p3: pixels.p2,
        };
        px_coords.0 = 1.0 - px_coords.0;
    }

    let first_triangle = flags.triangle && px_coords.0 + px_coords.1 <= 1.0;
    let second_triangle = flags.triangle && !first_triangle;
    if second_triangle {
        px_coords = (1.0 - px_coords.1, 1.0 - px_coords.0);
        pixels = Pixels {
            p0: pixels.p3,
            p1: pixels.p1,
            p2: pixels.p2,
            p3: pixels.p0,
        };
        edges = [1.0 - edges[1], 1.0 - edges[0], 1.0 - edges[3], 1.0 - edges[2]];
    }

    let (weights, mid_points, base_sharpness, pattern_pixels) = if flags.triangle {
        let coords_sum = (px_coords.0 + px_coords.1).max(EPSILON);
        let denominator = edges[3] * px_coords.0 + edges[0] * px_coords.1;
        let midpoint0 = if denominator.abs() <= EPSILON {
            0.5
        } else {
            edges[0] * edges[3] * coords_sum / denominator
        };
        let midpoint1 = 0.5 + 0.5 * (-edges[0] + edges[1] - edges[2] + edges[3]).clamp(-1.0, 1.0);
        (
            [coords_sum, coords_sum, px_coords.1 / coords_sum],
            [midpoint0, midpoint0, midpoint1],
            [1.0, 1.0, 0.0],
            Pixels {
                p0: pixels.p0,
                p1: pixels.p1,
                p2: pixels.p0,
                p3: pixels.p2,
            },
        )
    } else {
        (
            [px_coords.0, px_coords.0, px_coords.1],
            [
                mix(edges[0], edges[2], px_coords.1),
                mix(edges[3], edges[1], px_coords.0),
                mix(edges[3], edges[1], px_coords.0),
            ],
            [1.0, 1.0, 1.0],
            pixels,
        )
    };

    blend_cut23(
        blend_cut23(
            pattern_pixels.p0,
            pattern_pixels.p1,
            weights[0],
            mid_points[0],
            base_sharpness[0],
        ),
        blend_cut23(
            pattern_pixels.p2,
            pattern_pixels.p3,
            weights[1],
            mid_points[1],
            base_sharpness[1],
        ),
        weights[2],
        mid_points[2],
        base_sharpness[2],
    )
}

fn cut2_flags(width: u32, height: u32, source: &[Pixel]) -> Vec<CutFlags> {
    (0..height)
        .flat_map(|y| {
            (0..width).map(move |x| {
                let quads = neighbor_quads(width, height, source, x, y);
                let pattern = find_pattern(quads[0]);
                let neighbors = [
                    find_pattern(quads[1]),
                    find_pattern(quads[2]),
                    find_pattern(quads[3]),
                    find_pattern(quads[4]),
                ];
                let mut edges = [
                    hard_edge_weight(pattern, neighbors[0], 1, 4, 3),
                    hard_edge_weight(pattern, neighbors[1], 2, 3, 4),
                    hard_edge_weight(pattern, neighbors[2], 1, 3, 4),
                    hard_edge_weight(pattern, neighbors[3], 2, 4, 3),
                ];
                let l = neighbor_lumas(width, height, source, x, y);
                let soft_edges = soft_edges(l);
                for index in 0..4 {
                    if edges[index].abs() <= EPSILON {
                        edges[index] = soft_edges[index];
                    }
                }

                let mut pattern = find_pattern_with_neighbors(quads).abs();
                if pattern == 3 {
                    edges = [-edges[0], edges[3], -edges[2], edges[1]];
                }
                if pattern < 3 {
                    pattern = pattern.max(0);
                }

                CutFlags {
                    triangle: pattern >= 3,
                    flip: pattern == 3,
                    edges: normalize_edges(edges),
                }
            })
        })
        .collect()
}

fn cut3_flags(width: u32, height: u32, source: &[Pixel]) -> Vec<CutFlags> {
    let cells: Vec<Cut3Cell> = (0..height)
        .flat_map(|y| (0..width).map(move |x| cut3_cell(width, height, source, x, y)))
        .collect();
    let step = 0.5 / CUT3_HARD_EDGES_SEARCH_MAX_DISTANCE as f32;
    let hstep = step * 0.5;

    (0..height)
        .flat_map(|y| {
            let cells = &cells;
            (0..width).map(move |x| {
                let cell = cells[(y * width + x) as usize];
                let pattern = cell.pattern;
                let mut result_n = (0.0, 0.0);
                let mut result_s = (0.0, 0.0);
                let mut result_w = (0.0, 0.0);
                let mut result_e = (0.0, 0.0);

                if pattern == 1 || pattern == 3 || pattern == 4 {
                    result_n = walk_cut3(width, height, cells, x as i32, y as i32, 0, -1, (-1.0, 1.0), 1);
                    result_s = walk_cut3(width, height, cells, x as i32, y as i32, 0, 1, (1.0, -1.0), 1);
                }
                if pattern == 2 || pattern == 3 || pattern == 4 {
                    result_w = walk_cut3(width, height, cells, x as i32, y as i32, -1, 0, (-1.0, 1.0), 2);
                    result_e = walk_cut3(width, height, cells, x as i32, y as i32, 1, 0, (1.0, -1.0), 2);
                }

                let mut edge_pairs = [[(0.0, 0.0); 2]; 4];
                if pattern == 1 {
                    edge_pairs[0] = [result_n, (result_s.0 + step, result_s.1)];
                    edge_pairs[2] = [(result_n.0 + step, result_n.1), result_s];
                } else if pattern == 2 {
                    edge_pairs[3] = [result_w, (result_e.0 + step, result_e.1)];
                    edge_pairs[1] = [(result_w.0 + step, result_w.1), result_e];
                } else if pattern == 3 {
                    edge_pairs[0] = [result_n, (hstep, 1.0)];
                    edge_pairs[2] = [(hstep, -1.0), result_s];
                    edge_pairs[3] = [result_w, (hstep, 1.0)];
                    edge_pairs[1] = [(hstep, -1.0), result_e];
                } else if pattern == 4 {
                    edge_pairs[0] = [result_n, (hstep, -1.0)];
                    edge_pairs[2] = [(hstep, 1.0), result_s];
                    edge_pairs[3] = [result_w, (hstep, -1.0)];
                    edge_pairs[1] = [(hstep, 1.0), result_e];
                }

                let mut edges = [
                    blend_weights(edge_pairs[0][0], edge_pairs[0][1], step),
                    blend_weights(edge_pairs[1][0], edge_pairs[1][1], step),
                    blend_weights(edge_pairs[2][0], edge_pairs[2][1], step),
                    blend_weights(edge_pairs[3][0], edge_pairs[3][1], step),
                ];
                for index in 0..4 {
                    if edges[index].abs() <= EPSILON {
                        edges[index] = 2.0 * cell.soft_edges[index];
                    }
                }

                let original_pattern = pattern.abs();
                if original_pattern == 3 {
                    edges = [-edges[0], edges[3], -edges[2], edges[1]];
                }

                CutFlags {
                    triangle: original_pattern >= 3,
                    flip: original_pattern == 3,
                    edges: normalize_edges(edges),
                }
            })
        })
        .collect()
}

fn cut3_cell(width: u32, height: u32, source: &[Pixel], x: u32, y: u32) -> Cut3Cell {
    let quads = neighbor_quads(width, height, source, x, y);
    let mut pattern = find_pattern_with_neighbors(quads);
    let l = neighbor_lumas(width, height, source, x, y);
    let soft_edges = soft_edges(l);
    let values = [l[5], l[6], l[9], l[10]];
    let main_edges = [
        (values[0] - values[1]).abs(),
        (values[1] - values[3]).abs(),
        (values[2] - values[3]).abs(),
        (values[0] - values[2]).abs(),
    ];
    let max_edge = main_edges.iter().copied().fold(0.0, f32::max);
    let connections = [
        main_edges[0] >= 0.5 * max_edge,
        main_edges[1] >= 0.5 * max_edge,
        main_edges[2] >= 0.5 * max_edge,
        main_edges[3] >= 0.5 * max_edge,
    ];
    let mut neighbor_patterns = [
        find_pattern(quads[1]),
        find_pattern(quads[2]),
        find_pattern(quads[3]),
        find_pattern(quads[4]),
    ];
    for index in 0..4 {
        if !connections[index] {
            neighbor_patterns[index] = 0;
        }
    }

    let vertical = neighbor_patterns[0] == 1 || neighbor_patterns[2] == 1;
    let horizontal = neighbor_patterns[1] == 2 || neighbor_patterns[3] == 2;
    let corner = vertical && horizontal;
    let opposite = neighbor_patterns
        .iter()
        .any(|neighbor| *neighbor == if pattern == 3 { 4 } else { 3 });
    let is_triangle = pattern >= 3;
    let any_connection = connections.iter().any(|connected| *connected);
    let reject = (is_triangle && (opposite || corner)) || !any_connection;
    if pattern > 0 && reject {
        pattern = -pattern;
    }

    Cut3Cell {
        pattern,
        soft_edges,
    }
}

fn walk_cut3(
    width: u32,
    height: u32,
    cells: &[Cut3Cell],
    x: i32,
    y: i32,
    dx: i32,
    dy: i32,
    results: (f32, f32),
    continue_pattern: i32,
) -> (f32, f32) {
    let step = 0.5 / CUT3_HARD_EDGES_SEARCH_MAX_DISTANCE as f32;
    let hstep = step * 0.5;
    let mut result = (0.0, 0.0);
    for i in 1..=CUT3_HARD_EDGES_SEARCH_MAX_DISTANCE {
        let cx = (x + dx * i).clamp(0, width as i32 - 1) as u32;
        let cy = (y + dy * i).clamp(0, height as i32 - 1) as u32;
        let current_pattern = cells[(cy * width + cx) as usize].pattern;
        if current_pattern == 3 {
            result.1 = results.0;
        } else if current_pattern == 4 {
            result.1 = results.1;
        }
        if current_pattern == 3 || current_pattern == 4 {
            result.0 += hstep;
        } else if current_pattern == continue_pattern {
            result.0 += step;
        }
        if current_pattern != continue_pattern {
            break;
        }
    }
    result
}

fn neighbor_quads(width: u32, height: u32, source: &[Pixel], x: u32, y: u32) -> [Quad; 5] {
    let l = neighbor_lumas(width, height, source, x, y);
    [
        quad([l[5], l[6], l[9], l[10]]),
        quad([l[1], l[2], l[5], l[6]]),
        quad([l[6], l[7], l[10], l[11]]),
        quad([l[9], l[10], l[13], l[14]]),
        quad([l[4], l[5], l[8], l[9]]),
    ]
}

fn neighbor_lumas(width: u32, height: u32, source: &[Pixel], x: u32, y: u32) -> [f32; 16] {
    let mut lumas = [0.0; 16];
    for row in 0..4 {
        for col in 0..4 {
            let sx = (x as i32 + col as i32 - 1).clamp(0, width as i32 - 1) as u32;
            let sy = (y as i32 + row as i32 - 1).clamp(0, height as i32 - 1) as u32;
            lumas[row * 4 + col] = source[(sy * width + sx) as usize].luma();
        }
    }
    lumas
}

fn cell_pixels(width: u32, height: u32, source: &[Pixel], x: u32, y: u32) -> Pixels {
    let x1 = (x + 1).min(width.saturating_sub(1));
    let y1 = (y + 1).min(height.saturating_sub(1));
    Pixels {
        p0: source[(y * width + x) as usize],
        p1: source[(y * width + x1) as usize],
        p2: source[(y1 * width + x) as usize],
        p3: source[(y1 * width + x1) as usize],
    }
}

fn quad(values: [f32; 4]) -> Quad {
    let edges = [
        values[0] - values[1],
        values[1] - values[3],
        values[2] - values[3],
        values[0] - values[2],
    ];
    let scores = [
        (edges[0] + edges[2]).abs(),
        (edges[3] + edges[1]).abs(),
        (edges[0] - edges[1]).abs().max((edges[3] - edges[2]).abs()),
        (edges[0] + edges[3]).abs().max((edges[1] + edges[2]).abs()),
    ];
    Quad {
        scores,
        max_edge_contrast: edges.iter().map(|edge| edge.abs()).fold(0.0, f32::max),
        max_score: scores.iter().copied().fold(0.0, f32::max),
    }
}

fn find_pattern(quad: Quad) -> i32 {
    compute_pattern(quad, [0.0; 4])
}

fn find_pattern_with_neighbors(quads: [Quad; 5]) -> i32 {
    let mut adjustments = [0.0; 4];
    for quad in quads.iter().skip(1) {
        for index in 0..4 {
            adjustments[index] += quad.scores[index];
        }
    }
    compute_pattern(quads[0], adjustments)
}

fn compute_pattern(quad: Quad, neighbor_scores: [f32; 4]) -> i32 {
    let max_orthogonal = quad.scores[0].max(quad.scores[1]);
    let max_diagonal = quad.scores[2].max(quad.scores[3]);
    let is_diagonal = max_diagonal > max_orthogonal;
    let adjusted = [
        quad.scores[0] + 0.25 * neighbor_scores[0],
        quad.scores[1] + 0.25 * neighbor_scores[1],
        quad.scores[2] + 0.25 * neighbor_scores[2],
        quad.scores[3] + 0.25 * neighbor_scores[3],
    ];
    let threshold = 1.05;
    let mut result = 0;
    if !is_diagonal {
        if adjusted[0] > (threshold * adjusted[1]).max(EPSILON) {
            result = 1;
        } else if adjusted[1] > (threshold * adjusted[0]).max(EPSILON) {
            result = 2;
        }
    } else if adjusted[2] > (threshold * adjusted[3]).max(EPSILON) {
        result = 3;
    } else if adjusted[3] > (threshold * adjusted[2]).max(EPSILON) {
        result = 4;
    }
    let error = 2.0 * quad.max_edge_contrast - quad.max_score;
    if error > HARD_EDGES_SEARCH_MAX_ERROR * (0.5 + 0.5 * quad.max_edge_contrast) {
        result = -result;
    }
    result
}

fn hard_edge_weight(cp: i32, np: i32, vertical: i32, positive_diagonal: i32, negative_diagonal: i32) -> f32 {
    if (cp == vertical && np == positive_diagonal) || (np == vertical && cp == negative_diagonal) {
        0.5
    } else if (cp == vertical && np == negative_diagonal) || (np == vertical && cp == positive_diagonal) {
        -0.5
    } else {
        0.0
    }
}

fn soft_edges(l: [f32; 16]) -> [f32; 4] {
    [
        soft_edge_weight(l[4], l[5], l[6], l[7]),
        soft_edge_weight(l[2], l[6], l[10], l[14]),
        soft_edge_weight(l[8], l[9], l[10], l[11]),
        soft_edge_weight(l[1], l[5], l[9], l[13]),
    ]
}

fn soft_edge_weight(a: f32, b: f32, c: f32, d: f32) -> f32 {
    let diff = (b - c).abs();
    let result = diff / ((a - c).abs() + EPSILON) - diff / ((b - d).abs() + EPSILON);
    (2.0 * result).clamp(-1.0, 1.0)
}

fn blend_weights(d1: (f32, f32), d2: (f32, f32), step: f32) -> f32 {
    let max_double_distance = CUT3_HARD_EDGES_SEARCH_MAX_DISTANCE as f32 * step;
    let max_distance = step * (CUT3_HARD_EDGES_SEARCH_MAX_DISTANCE / 2) as f32 + step * 0.5;
    let total_distance = d1.0 + d2.0;
    if total_distance <= EPSILON {
        0.0
    } else {
        let d1_ratio = d1.0 / total_distance;
        if total_distance <= max_double_distance {
            if d1.0 < d2.0 {
                mix(d1.1, 0.0, 2.0 * d1_ratio)
            } else {
                mix(0.0, d2.1, (d1_ratio - 0.5) * 2.0)
            }
        } else if d1.0 <= max_distance {
            mix(d1.1, 0.0, d1.0 / max_distance)
        } else if d2.0 <= max_distance {
            mix(d2.1, 0.0, d2.0 / max_distance)
        } else {
            0.0
        }
    }
}

fn triangle_weights(px_coords: (f32, f32)) -> [f32; 3] {
    let w0 = px_coords.1 - px_coords.0;
    let w1 = 1.0 - w0;
    let w2 = (px_coords.1 - w0) / (w1 + EPSILON);
    [w0, w1, w2]
}

fn blend_cut1(a: Pixel, b: Pixel, t: f32) -> Pixel {
    let sharpness = sharpness(
        a.luma(),
        b.luma(),
        CUT1_BLEND_MIN_CONTRAST_EDGE,
        CUT1_BLEND_MAX_CONTRAST_EDGE,
        CUT1_BLEND_MIN_SHARPNESS,
        CUT1_BLEND_MAX_SHARPNESS,
    );
    a.mix(b, linear_step(sharpness, 1.0 - sharpness, t))
}

fn blend_cut23(a: Pixel, b: Pixel, t: f32, midpoint: f32, base_sharpness: f32) -> Pixel {
    let sharpness = base_sharpness
        * sharpness(
            a.luma(),
            b.luma(),
            CUT2_BLEND_MIN_CONTRAST_EDGE,
            CUT2_BLEND_MAX_CONTRAST_EDGE,
            CUT2_BLEND_MIN_SHARPNESS,
            CUT2_BLEND_MAX_SHARPNESS,
        );
    let nt = adjust_midpoint(t, midpoint.clamp(EPSILON, 1.0 - EPSILON));
    a.mix(b, ((nt - sharpness) / (1.0 - 2.0 * sharpness)).clamp(0.0, 1.0))
}

fn adjust_midpoint(x: f32, midpoint: f32) -> f32 {
    0.5 * ((x / midpoint).clamp(0.0, 1.0) + ((x - midpoint) / (1.0 - midpoint)).clamp(0.0, 1.0))
}

fn sharpness(
    l1: f32,
    l2: f32,
    min_contrast_edge: f32,
    max_contrast_edge: f32,
    min_sharpness: f32,
    max_sharpness: f32,
) -> f32 {
    let contrast = linear_step(min_contrast_edge, max_contrast_edge, (l1 - l2).abs());
    mix(min_sharpness * 0.5, max_sharpness * 0.5, contrast)
}

fn normalize_edges(edges: [f32; 4]) -> [f32; 4] {
    [
        (edges[0] * 0.5 + 0.5).clamp(EPSILON, 1.0 - EPSILON),
        (edges[1] * 0.5 + 0.5).clamp(EPSILON, 1.0 - EPSILON),
        (edges[2] * 0.5 + 0.5).clamp(EPSILON, 1.0 - EPSILON),
        (edges[3] * 0.5 + 0.5).clamp(EPSILON, 1.0 - EPSILON),
    ]
}

fn linear_step(edge0: f32, edge1: f32, t: f32) -> f32 {
    ((t - edge0) / (edge1 - edge0 + EPSILON)).clamp(0.0, 1.0)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

fn float_to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::{apply_cut, CutMode};

    #[test]
    fn cut_modes_scale_solid_image_to_2x() {
        let rgba = vec![
            31, 47, 61, 255, 31, 47, 61, 255,
            31, 47, 61, 255, 31, 47, 61, 255,
        ];

        for mode in [CutMode::Cut1, CutMode::Cut2, CutMode::Cut3] {
            let (width, height, out) = apply_cut(2, 2, &rgba, mode);
            assert_eq!((width, height), (4, 4));
            assert_eq!(out.len(), 4 * 4 * 4);
            assert!(out.chunks_exact(4).all(|px| px == [31, 47, 61, 255]));
        }
    }

    #[test]
    fn cut_modes_preserve_transparency_range() {
        let rgba = vec![
            255, 0, 0, 32, 0, 255, 0, 96,
            0, 0, 255, 160, 255, 255, 255, 224,
        ];

        for mode in [CutMode::Cut1, CutMode::Cut2, CutMode::Cut3] {
            let (_, _, out) = apply_cut(2, 2, &rgba, mode);
            assert!(out.chunks_exact(4).all(|px| px[3] >= 32 && px[3] <= 224));
        }
    }
}
