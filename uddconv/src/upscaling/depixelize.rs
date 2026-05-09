//! Implementation of the Kopf-Lischinski "Depixelizing Pixel Art" algorithm.
//!
//! Reference: https://johanneskopf.de/publications/pixelart/
//! Reference: https://github.com/vvanirudh/Pixel-Art

use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PixelPos {
    pub x: i32,
    pub y: i32,
}

impl PixelPos {
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

pub struct SimilarityGraph {
    pub width: u32,
    pub height: u32,
    /// Edges between pixels. stored as (from, to) where from.idx < to.idx.
    pub edges: HashSet<(usize, usize)>,
}

impl SimilarityGraph {
    pub fn new(width: u32, height: u32, rgba: &[u8]) -> Self {
        let mut edges = HashSet::new();

        let get_color = |x: i32, y: i32| -> Option<[u8; 4]> {
            if x < 0 || x >= width as i32 || y < 0 || y >= height as i32 {
                return None;
            }
            let idx = (y as usize * width as usize + x as usize) * 4;
            Some([rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]])
        };

        let is_similar = |c1: [u8; 4], c2: [u8; 4]| -> bool {
            if c1[3] == 0 && c2[3] == 0 { return true; }
            if c1[3] != c2[3] { return false; }
            
            // YUV-like distance
            let r1 = c1[0] as i32;
            let g1 = c1[1] as i32;
            let b1 = c1[2] as i32;
            let r2 = c2[0] as i32;
            let g2 = c2[1] as i32;
            let b2 = c2[2] as i32;
            
            let dr = r1 - r2;
            let dg = g1 - g2;
            let db = b1 - b2;
            
            // Euclidean distance squared in RGB space
            (dr*dr + dg*dg + db*db) < 30 * 30 // Threshold
        };

        let pos_to_idx = |x: i32, y: i32| (y as usize * width as usize + x as usize);

        // 1. Build initial graph (4-connectivity and 8-connectivity)
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                let c = get_color(x, y).unwrap();
                let idx = pos_to_idx(x, y);

                // Check neighbors: (1, 0), (0, 1), (1, 1), (1, -1)
                let neighbors = [(1, 0), (0, 1), (1, 1), (1, -1)];
                for (dx, dy) in neighbors {
                    let nx = x + dx;
                    let ny = y + dy;
                    if let Some(nc) = get_color(nx, ny) {
                        if is_similar(c, nc) {
                            let n_idx = pos_to_idx(nx, ny);
                            edges.insert((idx.min(n_idx), idx.max(n_idx)));
                        }
                    }
                }
            }
        }

        // 2. Resolve ambiguities (crossing diagonals)
        for y in 0..height as i32 - 1 {
            for x in 0..width as i32 - 1 {
                let i00 = pos_to_idx(x, y);
                let i10 = pos_to_idx(x + 1, y);
                let i01 = pos_to_idx(x, y + 1);
                let i11 = pos_to_idx(x + 1, y + 1);

                let d1 = (i00.min(i11), i00.max(i11)); // (0,0) - (1,1)
                let d2 = (i10.min(i01), i10.max(i01)); // (1,0) - (0,1)

                if edges.contains(&d1) && edges.contains(&d2) {
                    let score_d1 = calculate_curve_score(x, y, x + 1, y + 1, width, height, rgba, is_similar);
                    let score_d2 = calculate_curve_score(x + 1, y, x, y + 1, width, height, rgba, is_similar);

                    if score_d1 >= score_d2 {
                        edges.remove(&d2);
                    } else {
                        edges.remove(&d1);
                    }
                }
            }
        }

        SimilarityGraph { width, height, edges }
    }
}

fn calculate_curve_score<F>(x1: i32, y1: i32, x2: i32, y2: i32, width: u32, height: u32, rgba: &[u8], is_similar: F) -> i32 
where F: Fn([u8; 4], [u8; 4]) -> bool {
    let mut score = 0;
    
    let get_color = |x: i32, y: i32| -> Option<[u8; 4]> {
        if x < 0 || x >= width as i32 || y < 0 || y >= height as i32 {
            return None;
        }
        let idx = (y as usize * width as usize + x as usize) * 4;
        Some([rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]])
    };

    let c1 = get_color(x1, y1).unwrap();
    let c2 = get_color(x2, y2).unwrap();

    let dx = x2 - x1;
    let dy = y2 - y1;

    // Check 8 directions around endpoints
    for ddx in -1..=1 {
        for ddy in -1..=1 {
            if ddx == 0 && ddy == 0 { continue; }
            if let Some(nc) = get_color(x1 + ddx, y1 + ddy) {
                if is_similar(nc, c1) { score += 1; }
            }
            if let Some(nc) = get_color(x2 + ddx, y2 + ddy) {
                if is_similar(nc, c2) { score += 1; }
            }
        }
    }

    // Directional consistency
    if let Some(nc) = get_color(x1 - dx, y1 - dy) {
        if is_similar(nc, c1) { score += 2; }
    }
    if let Some(nc) = get_color(x2 + dx, y2 + dy) {
        if is_similar(nc, c2) { score += 2; }
    }

    score
}

pub fn apply_depixelize(width: u32, height: u32, rgba: &[u8], scale: u32) -> (u32, u32, Vec<u8>) {
    let graph = SimilarityGraph::new(width, height, rgba);
    
    let target_width = width * scale;
    let target_height = height * scale;
    let mut out_rgba = vec![0u8; (target_width * target_height * 4) as usize];

    for ty in 0..target_height {
        for tx in 0..target_width {
            let sx = (tx / scale) as i32;
            let sy = (ty / scale) as i32;
            
            let fx = (tx % scale) as f32 / scale as f32;
            let fy = (ty % scale) as f32 / scale as f32;

            let best_s = resolve_best_pixel(&graph, sx, sy, fx, fy);
            
            let s_idx = (best_s.y as usize * width as usize + best_s.x as usize) * 4;
            let out_idx = (ty as usize * target_width as usize + tx as usize) * 4;
            
            out_rgba[out_idx..out_idx + 4].copy_from_slice(&rgba[s_idx..s_idx + 4]);
        }
    }

    (target_width, target_height, out_rgba)
}

fn is_connected(graph: &SimilarityGraph, p1: PixelPos, p2: PixelPos) -> bool {
    if p1.x < 0 || p1.x >= graph.width as i32 || p1.y < 0 || p1.y >= graph.height as i32 { return false; }
    if p2.x < 0 || p2.x >= graph.width as i32 || p2.y < 0 || p2.y >= graph.height as i32 { return false; }
    let i1 = (p1.y * graph.width as i32 + p1.x) as usize;
    let i2 = (p2.y * graph.width as i32 + p2.x) as usize;
    graph.edges.contains(&(i1.min(i2), i1.max(i2)))
}

fn resolve_best_pixel(graph: &SimilarityGraph, sx: i32, sy: i32, fx: f32, fy: f32) -> PixelPos {
    // Determine quadrant
    let qx = if fx > 0.5 { 1 } else { -1 };
    let qy = if fy > 0.5 { 1 } else { -1 };

    let p00 = PixelPos::new(sx, sy);
    let p10 = PixelPos::new(sx + qx, sy);
    let p01 = PixelPos::new(sx, sy + qy);
    let p11 = PixelPos::new(sx + qx, sy + qy);

    // Diagonal connection check
    if is_connected(graph, p00, p11) {
        let lx = if qx == 1 { fx - 0.5 } else { 0.5 - fx };
        let ly = if qy == 1 { fy - 0.5 } else { 0.5 - fy };
        if lx + ly > 0.5 {
            return p11;
        }
    }

    // If not diagonally connected, we belong to p00 if we are in its "voronoi cell".
    // In the absence of other connections, p00 owns the whole square.
    // If p10 is connected to p01 (the OTHER diagonal), then p00 only owns half of the 2x2.
    if is_connected(graph, p10, p01) {
        let lx = if qx == 1 { fx - 0.5 } else { 0.5 - fx };
        let ly = if qy == 1 { fy - 0.5 } else { 0.5 - fy };
        // The boundary is lx + ly = 0.5
        if lx + ly > 0.5 {
            // We are in the "other" triangle. 
            // Decide between p10 and p01.
            if lx > ly { return p10; }
            else { return p01; }
        }
    }

    p00
}

