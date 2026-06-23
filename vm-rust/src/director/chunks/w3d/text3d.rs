//! 3D text (`extrude3D`) geometry — the contour-based pipeline that replaces the
//! old per-contour triangle-fan cap.
//!
//! Faithful to Director's IFX 3D-glyph pipeline (see docs/3dtext-extrude3d.md):
//! a glyph is a set of closed 2D contours (outer outlines + holes). The caps
//! (front/back faces) are filled with a polygon-with-HOLES triangulation so the
//! counters of O / A / 8 / e / B … come out hollow instead of solid, and the
//! tunnel side-walls are emitted per contour edge.
//!
//! Phase 1 = correct caps + flat tunnel (no bevel). Bevel (#miter/#round),
//! displayFace selection and planar UVs are later phases.

/// Signed area of a closed 2D polygon. CCW (counter-clockwise) is positive.
fn signed_area(c: &[[f32; 2]]) -> f32 {
    let n = c.len();
    if n < 3 {
        return 0.0;
    }
    let mut a = 0.0_f32;
    let mut j = n - 1;
    for i in 0..n {
        a += (c[j][0] + c[i][0]) * (c[j][1] - c[i][1]);
        j = i;
    }
    -a * 0.5
}

/// Even-odd ray-cast point-in-polygon test.
fn point_in_poly(p: [f32; 2], c: &[[f32; 2]]) -> bool {
    let n = c.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (c[i][0], c[i][1]);
        let (xj, yj) = (c[j][0], c[j][1]);
        if ((yi > p[1]) != (yj > p[1]))
            && (p[0] < (xj - xi) * (p[1] - yi) / (yj - yi) + xi)
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Nesting depth of contour `i`: how many OTHER contours contain it (test the
/// contour's first vertex). Even depth = solid (outer), odd depth = hole.
fn nesting_depth(contours: &[Vec<[f32; 2]>], i: usize) -> usize {
    let probe = match contours[i].first() {
        Some(p) => *p,
        None => return 0,
    };
    let mut depth = 0;
    for (j, c) in contours.iter().enumerate() {
        if j != i && c.len() >= 3 && point_in_poly(probe, c) {
            depth += 1;
        }
    }
    depth
}

/// Triangulate the glyph caps (the filled region, holes subtracted) over the
/// FLAT concatenation of all contour points. `offsets[i]` is the first global
/// index of contour `i`. Returns triangles as global point indices. Winding is
/// left as earcut produced it; the extruder forces per-face winding for ±Z.
fn triangulate_caps(contours: &[Vec<[f32; 2]>], offsets: &[usize]) -> Vec<[u32; 3]> {
    let n = contours.len();
    let depths: Vec<usize> = (0..n).map(|i| nesting_depth(contours, i)).collect();

    let mut tris: Vec<[u32; 3]> = Vec::new();
    for outer in 0..n {
        if contours[outer].len() < 3 || depths[outer] % 2 != 0 {
            continue; // only solid (even-depth) contours are outers
        }
        // Holes = immediately-nested (depth+1) contours contained in this outer.
        let mut holes: Vec<usize> = Vec::new();
        for h in 0..n {
            if h != outer
                && contours[h].len() >= 3
                && depths[h] == depths[outer] + 1
                && point_in_poly(contours[h][0], &contours[outer])
            {
                holes.push(h);
            }
        }

        // Build earcut input: outer ring then each hole ring, flat [x,y,...].
        let ring_order: Vec<usize> = std::iter::once(outer).chain(holes.iter().copied()).collect();
        let mut data: Vec<f64> = Vec::new();
        let mut hole_indices: Vec<usize> = Vec::new();
        let mut local_to_global: Vec<u32> = Vec::new();
        for (ri, &ci) in ring_order.iter().enumerate() {
            if ri > 0 {
                hole_indices.push(data.len() / 2); // vertex index where this hole starts
            }
            for (k, p) in contours[ci].iter().enumerate() {
                data.push(p[0] as f64);
                data.push(p[1] as f64);
                local_to_global.push((offsets[ci] + k) as u32);
            }
        }

        if let Ok(idx) = earcutr::earcut(&data, &hole_indices, 2) {
            for t in idx.chunks_exact(3) {
                tris.push([
                    local_to_global[t[0]],
                    local_to_global[t[1]],
                    local_to_global[t[2]],
                ]);
            }
        }
    }
    tris
}

/// Extrusion parameters (mirrors Director's 3D-text properties).
#[derive(Clone, Copy, Debug)]
pub struct ExtrudeParams {
    /// `tunnelDepth` — total extrusion length along +Z.
    pub depth: f32,
    /// `bevelType`: 0 = #none, 1 = #miter, 2 = #round.
    pub bevel_type: u32,
    /// `bevelDepth` — chamfer size (inward inset and Z extent of each chamfer).
    pub bevel_depth: f32,
    /// `smoothness` — drives the #round chamfer arc subdivision.
    pub smoothness: u32,
    /// `displayFace` selection — generate the front cap / back cap / tunnel
    /// (sides + bevel) only when the corresponding flag is set.
    pub front: bool,
    pub back: bool,
    pub tunnel: bool,
}

impl ExtrudeParams {
    /// Flat extrusion (no bevel), all faces, of the given depth.
    pub fn flat(depth: f32) -> Self {
        ExtrudeParams {
            depth,
            bevel_type: 0,
            bevel_depth: 0.0,
            smoothness: 0,
            front: true,
            back: true,
            tunnel: true,
        }
    }
}

fn edge_unit(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let l = (dx * dx + dy * dy).sqrt().max(1e-8);
    [dx / l, dy / l]
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-8);
    [v[0] / l, v[1] / l, v[2] / l]
}

/// Per-vertex outward normals for a closed contour: `unit` = the unit angle
/// bisector (for shading), `miter` = the same direction scaled by the miter
/// factor 1/cos(half-angle) (clamped) so an inward offset keeps the inset edges
/// parallel and corners gap-free.
fn vertex_normals(c: &[[f32; 2]]) -> (Vec<[f32; 2]>, Vec<[f32; 2]>) {
    let n = c.len();
    let mut unit = vec![[0.0_f32, 0.0]; n];
    let mut miter = vec![[0.0_f32, 0.0]; n];
    for i in 0..n {
        let prev = c[(i + n - 1) % n];
        let cur = c[i];
        let next = c[(i + 1) % n];
        let e0 = edge_unit(prev, cur); // incoming edge
        let e1 = edge_unit(cur, next); // outgoing edge
        let rp0 = [e0[1], -e0[0]]; // right perpendicular (outward for CCW)
        let rp1 = [e1[1], -e1[0]];
        let mut b = [rp0[0] + rp1[0], rp0[1] + rp1[1]];
        let l = (b[0] * b[0] + b[1] * b[1]).sqrt();
        if l < 1e-6 {
            b = rp1; // 180° reversal — fall back to the edge normal
        } else {
            b = [b[0] / l, b[1] / l];
        }
        unit[i] = b;
        // cos(half-angle) between the bisector and an edge normal; clamp → miter limit ≈ 4.
        let cosang = (b[0] * rp1[0] + b[1] * rp1[1]).max(0.25);
        let s = 1.0 / cosang;
        miter[i] = [b[0] * s, b[1] * s];
    }
    (unit, miter)
}

fn push_quad(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    faces: &mut Vec<[u32; 3]>,
    quad: [[f32; 3]; 4],
    norms: [[f32; 3]; 4],
) {
    let v = positions.len() as u32;
    positions.extend_from_slice(&quad);
    normals.extend_from_slice(&norms);
    faces.push([v, v + 1, v + 2]);
    faces.push([v, v + 2, v + 3]);
}

/// Extrude a glyph (a set of closed contours, outer + holes) into a 3D mesh:
/// front cap at z=0 (normal −Z), back cap at z=`depth` (normal +Z), and side
/// walls (a straight tunnel for #none, or a chamfer→tunnel→chamfer for
/// #miter/#round). Holes are filled correctly.
///
/// Returns (positions, normals, faces).
pub fn extrude_glyph(
    contours: &[Vec<[f32; 2]>],
    params: &ExtrudeParams,
) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[u32; 3]>) {
    let depth = params.depth.max(1e-4);

    // Working copy with consistent orientation: outers CCW, holes CW. This makes
    // the right-hand edge perpendicular (dy,−dx) the outward wall normal for
    // outers and the into-the-void normal for holes (both = facing away from the
    // solid material), and keeps cap winding predictable.
    let n_contours = contours.len();
    let depths: Vec<usize> = (0..n_contours).map(|i| nesting_depth(contours, i)).collect();
    let work: Vec<Vec<[f32; 2]>> = contours
        .iter()
        .enumerate()
        .filter(|(_, c)| c.len() >= 3)
        .map(|(i, c)| {
            let area = signed_area(c);
            let want_ccw = depths[i] % 2 == 0; // outer wants CCW (area > 0)
            let mut v = c.clone();
            if (want_ccw && area < 0.0) || (!want_ccw && area > 0.0) {
                v.reverse();
            }
            v
        })
        .collect();

    if work.is_empty() {
        return (Vec::new(), Vec::new(), Vec::new());
    }

    // Flat offsets for the cap vertex sets.
    let mut offsets = Vec::with_capacity(work.len());
    let mut total = 0usize;
    for c in &work {
        offsets.push(total);
        total += c.len();
    }
    let n = total as u32;

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(total * 2);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(total * 2);
    let mut faces: Vec<[u32; 3]> = Vec::new();

    // Front cap vertices (z = 0, normal −Z), then back cap vertices (z = depth, +Z).
    for c in &work {
        for p in c {
            positions.push([p[0], p[1], 0.0]);
            normals.push([0.0, 0.0, -1.0]);
        }
    }
    for c in &work {
        for p in c {
            positions.push([p[0], p[1], depth]);
            normals.push([0.0, 0.0, 1.0]);
        }
    }

    // Caps: triangulate once, emit front and/or back per displayFace with forced winding.
    if params.front || params.back {
        let cap_tris = triangulate_caps(&work, &offsets);
        for t in &cap_tris {
            let (a, b, c) = (t[0], t[1], t[2]);
            let pa = positions[a as usize];
            let pb = positions[b as usize];
            let pc = positions[c as usize];
            // XY cross product z-component → triangle winding in the plane.
            let cross = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pb[1] - pa[1]) * (pc[0] - pa[0]);
            // FRONT (normal −Z): viewed from below it must be CCW → from above CW → cross < 0.
            if params.front {
                if cross > 0.0 {
                    faces.push([a, c, b]);
                } else {
                    faces.push([a, b, c]);
                }
            }
            // BACK (normal +Z): cross > 0 (CCW from above).
            if params.back {
                if cross > 0.0 {
                    faces.push([a + n, b + n, c + n]);
                } else {
                    faces.push([a + n, c + n, b + n]);
                }
            }
        }
    }

    // displayFace without #tunnel → caps only, no sides.
    if !params.tunnel {
        return (positions, normals, faces);
    }

    // Side walls. #none → straight tunnel. #miter/#round → a chamfer from the
    // full-outline cap edge (z=0) inward to the inset tunnel, the tunnel, then a
    // mirrored chamfer back out to the full-outline back edge (z=depth). The inset
    // uses the per-vertex miter normal so corners stay gap-free.
    let bevel = if params.bevel_type == 0 {
        0.0
    } else {
        // A chamfer can't take more than half the depth (front + back).
        params.bevel_depth.max(0.0).min(depth * 0.5 - 1e-3).max(0.0)
    };
    let arc_steps: usize = if params.bevel_type == 2 {
        ((params.smoothness.max(1) as usize) / 2).clamp(2, 6) // #round: arc subdivisions
    } else {
        1 // #miter: single flat chamfer
    };
    use std::f32::consts::FRAC_PI_2;

    for c in &work {
        let m = c.len();
        // Per-vertex outward normals: `unit` is the smooth angle-bisector used for
        // SHADING (so curved edges read as one smooth lit edge instead of faceting
        // per segment), `miter` is the (miter-limited) offset for the bevel inset.
        let (unit, miter) = vertex_normals(c);

        if bevel <= 0.0 {
            // Flat tunnel: one quad per edge, smooth per-vertex side normals.
            for i in 0..m {
                let j = (i + 1) % m;
                let p0 = c[i];
                let p1 = c[j];
                let ni = [unit[i][0], unit[i][1], 0.0];
                let nj = [unit[j][0], unit[j][1], 0.0];
                push_quad(
                    &mut positions, &mut normals, &mut faces,
                    [[p0[0], p0[1], 0.0], [p1[0], p1[1], 0.0], [p1[0], p1[1], depth], [p0[0], p0[1], depth]],
                    [ni, nj, nj, ni],
                );
            }
            continue;
        }

        let inset: Vec<[f32; 2]> = (0..m)
            .map(|i| [c[i][0] - bevel * miter[i][0], c[i][1] - bevel * miter[i][1]])
            .collect();
        let z_fi = bevel; // front inset plane
        let z_bi = depth - bevel; // back inset plane

        for i in 0..m {
            let j = (i + 1) % m;

            // Front chamfer: full edge (z=0) → inset (z=bevel). Smooth per-vertex
            // normals (front → side) so the lit bevel is one clean edge.
            let fpos = |k: usize, a: f32| -> [f32; 3] {
                let off = bevel * (1.0 - a.cos());
                [c[k][0] - off * miter[k][0], c[k][1] - off * miter[k][1], bevel * a.sin()]
            };
            let fnrm = |k: usize, a: f32| -> [f32; 3] {
                normalize3([unit[k][0] * a.sin(), unit[k][1] * a.sin(), -a.cos()])
            };
            for s in 0..arc_steps {
                let a0 = (s as f32 / arc_steps as f32) * FRAC_PI_2;
                let a1 = ((s + 1) as f32 / arc_steps as f32) * FRAC_PI_2;
                let q = [fpos(i, a0), fpos(j, a0), fpos(j, a1), fpos(i, a1)];
                let nrm = [fnrm(i, a0), fnrm(j, a0), fnrm(j, a1), fnrm(i, a1)];
                push_quad(&mut positions, &mut normals, &mut faces, q, nrm);
            }

            // Tunnel: inset front (z_fi) → inset back (z_bi), smooth side normals.
            {
                let ni = [unit[i][0], unit[i][1], 0.0];
                let nj = [unit[j][0], unit[j][1], 0.0];
                push_quad(
                    &mut positions, &mut normals, &mut faces,
                    [
                        [inset[i][0], inset[i][1], z_fi],
                        [inset[j][0], inset[j][1], z_fi],
                        [inset[j][0], inset[j][1], z_bi],
                        [inset[i][0], inset[i][1], z_bi],
                    ],
                    [ni, nj, nj, ni],
                );
            }

            // Back chamfer: inset (z=z_bi) → full edge (z=depth), mirrored arc.
            let bpos = |k: usize, a: f32| -> [f32; 3] {
                let off = bevel * a.cos();
                [c[k][0] - off * miter[k][0], c[k][1] - off * miter[k][1], depth - bevel * a.cos()]
            };
            let bnrm = |k: usize, a: f32| -> [f32; 3] {
                normalize3([unit[k][0] * a.cos(), unit[k][1] * a.cos(), a.sin()])
            };
            for s in 0..arc_steps {
                let a0 = (s as f32 / arc_steps as f32) * FRAC_PI_2;
                let a1 = ((s + 1) as f32 / arc_steps as f32) * FRAC_PI_2;
                let q = [bpos(i, a0), bpos(j, a0), bpos(j, a1), bpos(i, a1)];
                let nrm = [bnrm(i, a0), bnrm(j, a0), bnrm(j, a1), bnrm(i, a1)];
                push_quad(&mut positions, &mut normals, &mut faces, q, nrm);
            }
        }
    }

    (positions, normals, faces)
}

/// Linear interpolation parameter where `thr` is crossed between corner values
/// `a` and `b` (guarded against a zero denominator).
fn cross_t(a: f32, b: f32, thr: f32) -> f32 {
    let d = b - a;
    if d.abs() < 1e-6 {
        0.5
    } else {
        ((thr - a) / d).clamp(0.0, 1.0)
    }
}

/// Separable box blur of a scalar field, clamped at the edges (one pass ≈ a
/// triangular/Gaussian-ish smoothing of the alpha before contour tracing).
fn box_blur(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let ri = r as i32;
    let mut tmp = vec![0.0f32; src.len()];
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let (mut s, mut c) = (0.0f32, 0.0f32);
            for dx in -ri..=ri {
                let xx = x as i32 + dx;
                if xx >= 0 && (xx as usize) < w {
                    s += src[row + xx as usize];
                    c += 1.0;
                }
            }
            tmp[row + x] = s / c;
        }
    }
    let mut out = vec![0.0f32; src.len()];
    for y in 0..h {
        for x in 0..w {
            let (mut s, mut c) = (0.0f32, 0.0f32);
            for dy in -ri..=ri {
                let yy = y as i32 + dy;
                if yy >= 0 && (yy as usize) < h {
                    s += tmp[yy as usize * w + x];
                    c += 1.0;
                }
            }
            out[y * w + x] = s / c;
        }
    }
    out
}

/// Trace closed iso-contours of an alpha mask at `threshold` via marching
/// squares (sub-pixel crossings), returning closed polygons in PIXEL space
/// (x∈[0,width], y∈[0,height]). Outer outlines and holes both come out as
/// separate loops; which is a hole is decided later by `extrude_glyph`'s
/// even-odd nesting. Loops are Douglas-Peucker simplified by `simplify_eps`
/// (pixels) to keep the mesh light. This is the no-PFR (system/native font)
/// outline source: the text is rasterised, then revectorised here.
pub fn vectorize_alpha(
    width: u32,
    height: u32,
    rgba: &[u8],
    threshold: u8,
    simplify_eps: f32,
    smooth_iters: usize,
    blur_radius: usize,
) -> Vec<Vec<[f32; 2]>> {
    let w = width as usize;
    let h = height as usize;
    if w < 2 || h < 2 || rgba.len() < w * h * 4 {
        return Vec::new();
    }
    let thr = threshold as f32;
    // Pre-blur the alpha so the iso-contour is a smooth curve instead of the wavy
    // 50%-coverage boundary of an anti-aliased edge (the dominant source of the
    // jagged outline). Marching squares then traces the blurred field.
    let mut abuf: Vec<f32> = (0..w * h).map(|i| rgba[i * 4 + 3] as f32).collect();
    if blur_radius > 0 {
        abuf = box_blur(&abuf, w, h, blur_radius);
    }
    let alpha = |x: usize, y: usize| -> f32 { abuf[y * w + x] };

    let mut segs: Vec<([f32; 2], [f32; 2])> = Vec::new();
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            let a0 = alpha(x, y); // top-left
            let a1 = alpha(x + 1, y); // top-right
            let a2 = alpha(x + 1, y + 1); // bottom-right
            let a3 = alpha(x, y + 1); // bottom-left
            let case = (a0 >= thr) as u8
                | (((a1 >= thr) as u8) << 1)
                | (((a2 >= thr) as u8) << 2)
                | (((a3 >= thr) as u8) << 3);
            if case == 0 || case == 15 {
                continue;
            }
            let (xf, yf) = (x as f32, y as f32);
            // Crossing point on edge e, ALWAYS parameterised from the lower grid
            // index toward the higher one so the point on a shared edge is computed
            // identically by both adjacent cells (else AA text fragments):
            //   0=top   : c0(x,y)   → c1(x+1,y)     pt = (x+t, y)
            //   1=right : c1(x+1,y) → c2(x+1,y+1)   pt = (x+1, y+t)
            //   2=bottom: c3(x,y+1) → c2(x+1,y+1)   pt = (x+t, y+1)
            //   3=left  : c0(x,y)   → c3(x,y+1)     pt = (x, y+t)
            let pt = |e: u8| -> [f32; 2] {
                match e {
                    0 => [xf + cross_t(a0, a1, thr), yf],
                    1 => [xf + 1.0, yf + cross_t(a1, a2, thr)],
                    2 => [xf + cross_t(a3, a2, thr), yf + 1.0],
                    _ => [xf, yf + cross_t(a0, a3, thr)],
                }
            };
            let pairs: &[(u8, u8)] = match case {
                1 | 14 => &[(3, 0)],
                2 | 13 => &[(0, 1)],
                3 | 12 => &[(3, 1)],
                4 | 11 => &[(1, 2)],
                6 | 9 => &[(0, 2)],
                7 | 8 => &[(2, 3)],
                5 => &[(3, 0), (1, 2)],  // saddle (fixed resolution)
                10 => &[(0, 1), (2, 3)], // saddle
                _ => &[],
            };
            for &(ea, eb) in pairs {
                segs.push((pt(ea), pt(eb)));
            }
        }
    }

    // Smooth the stair-stepped marching-squares loops (corner-averaging) so the
    // outline reads as a curve, THEN simplify, THEN drop tiny noise specks (stray
    // AA pixels that cross the threshold). DP alone can't remove a stairstep
    // because its alternating vertices are all "far" from the chord.
    let min_area = (simplify_eps * simplify_eps).max(2.0);
    link_segments(&segs)
        .into_iter()
        .map(|loop_pts| smooth_closed(&loop_pts, smooth_iters))
        .map(|loop_pts| simplify_closed(&loop_pts, simplify_eps))
        .filter(|c| c.len() >= 3 && signed_area(c).abs() > min_area)
        .collect()
}

/// One Laplacian pass on a closed loop: each point moves `factor` of the way to
/// the midpoint of its neighbours.
fn laplacian_pass(pts: &[[f32; 2]], factor: f32) -> Vec<[f32; 2]> {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let a = pts[(i + n - 1) % n];
            let b = pts[i];
            let c = pts[(i + 1) % n];
            let mx = (a[0] + c[0]) * 0.5;
            let my = (a[1] + c[1]) * 0.5;
            [b[0] + factor * (mx - b[0]), b[1] + factor * (my - b[1])]
        })
        .collect()
}

/// Taubin (λ/μ) smoothing of a closed loop: a positive shrink pass followed by a
/// negative inflate pass per iteration. De-jags the marching-squares stairstep
/// WITHOUT the net shrinkage of plain averaging — important so thin strokes in
/// small text (the stems of l/i/j) survive instead of collapsing into blobs.
fn smooth_closed(pts: &[[f32; 2]], iters: usize) -> Vec<[f32; 2]> {
    if pts.len() < 5 || iters == 0 {
        return pts.to_vec();
    }
    let lambda = 0.5_f32;
    let mu = -0.53_f32;
    let mut cur = pts.to_vec();
    for _ in 0..iters {
        cur = laplacian_pass(&cur, lambda);
        cur = laplacian_pass(&cur, mu);
    }
    cur
}

/// Quantise a point for endpoint matching (marching-squares crossings on a
/// shared edge are identical, so a coarse key links neighbouring cells).
fn key(p: [f32; 2]) -> (i32, i32) {
    ((p[0] * 16.0).round() as i32, (p[1] * 16.0).round() as i32)
}

/// Link undirected segments into closed loops by matching shared endpoints.
fn link_segments(segs: &[([f32; 2], [f32; 2])]) -> Vec<Vec<[f32; 2]>> {
    use std::collections::HashMap;
    // point key -> list of (segment index, endpoint 0|1)
    let mut adj: HashMap<(i32, i32), Vec<(usize, u8)>> = HashMap::new();
    for (i, s) in segs.iter().enumerate() {
        adj.entry(key(s.0)).or_default().push((i, 0));
        adj.entry(key(s.1)).or_default().push((i, 1));
    }
    let mut used = vec![false; segs.len()];
    let mut loops: Vec<Vec<[f32; 2]>> = Vec::new();

    for start in 0..segs.len() {
        if used[start] {
            continue;
        }
        let mut loop_pts: Vec<[f32; 2]> = Vec::new();
        let mut cur = start;
        let mut from_end: u8 = 0; // we enter `cur` at endpoint 0, exit at 1
        loop {
            used[cur] = true;
            let (p_in, p_out) = if from_end == 0 {
                (segs[cur].0, segs[cur].1)
            } else {
                (segs[cur].1, segs[cur].0)
            };
            loop_pts.push(p_in);
            // find the next unused segment sharing p_out
            let k = key(p_out);
            let mut next: Option<(usize, u8)> = None;
            if let Some(list) = adj.get(&k) {
                for &(si, end) in list {
                    if si != cur && !used[si] {
                        next = Some((si, end));
                        break;
                    }
                }
            }
            match next {
                Some((si, end)) => {
                    cur = si;
                    from_end = end; // we arrive at `end`, so exit the other side
                }
                None => {
                    break;
                }
            }
            if cur == start {
                break;
            }
        }
        if loop_pts.len() >= 3 {
            loops.push(loop_pts);
        }
    }
    loops
}

fn dist_pt_seg2(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let l2 = dx * dx + dy * dy;
    if l2 < 1e-9 {
        let ex = p[0] - a[0];
        let ey = p[1] - a[1];
        return ex * ex + ey * ey;
    }
    let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0);
    let px = a[0] + t * dx;
    let py = a[1] + t * dy;
    let ex = p[0] - px;
    let ey = p[1] - py;
    ex * ex + ey * ey
}

fn dp_rec(pts: &[[f32; 2]], a: usize, b: usize, eps2: f32, keep: &mut [bool]) {
    if b <= a + 1 {
        return;
    }
    let mut idx = 0;
    let mut dmax = 0.0_f32;
    for i in (a + 1)..b {
        let d = dist_pt_seg2(pts[i], pts[a], pts[b]);
        if d > dmax {
            dmax = d;
            idx = i;
        }
    }
    if dmax > eps2 {
        keep[idx] = true;
        dp_rec(pts, a, idx, eps2, keep);
        dp_rec(pts, idx, b, eps2, keep);
    }
}

/// Douglas-Peucker simplify a CLOSED loop: split at the point farthest from the
/// start, simplify both arcs, keep the close.
fn simplify_closed(pts: &[[f32; 2]], eps: f32) -> Vec<[f32; 2]> {
    let n = pts.len();
    if n < 4 || eps <= 0.0 {
        return pts.to_vec();
    }
    let mut far = 0;
    let mut fd = 0.0_f32;
    for i in 1..n {
        let dx = pts[i][0] - pts[0][0];
        let dy = pts[i][1] - pts[0][1];
        let d = dx * dx + dy * dy;
        if d > fd {
            fd = d;
            far = i;
        }
    }
    let eps2 = eps * eps;
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[far] = true;
    dp_rec(pts, 0, far, eps2, &mut keep);
    dp_rec(pts, far, n - 1, eps2, &mut keep);
    (0..n).filter(|&i| keep[i]).map(|i| pts[i]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri_area_xy(p: &[[f32; 3]], t: &[u32; 3]) -> f32 {
        let a = p[t[0] as usize];
        let b = p[t[1] as usize];
        let c = p[t[2] as usize];
        0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs()
    }

    #[test]
    fn signed_area_orientation() {
        // CCW square → positive.
        let ccw = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        assert!(signed_area(&ccw) > 0.0);
        let mut cw = ccw.clone();
        cw.reverse();
        assert!(signed_area(&cw) < 0.0);
    }

    #[test]
    fn square_with_hole_caps_are_hollow() {
        // Outer 10x10 (CCW) with a 4x4 centred hole (CW). Filled area = 84.
        let outer = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let hole = vec![[3.0, 3.0], [3.0, 7.0], [7.0, 7.0], [7.0, 3.0]]; // CW
        let contours = vec![outer, hole];

        let (pos, nrm, faces) = extrude_glyph(&contours, &ExtrudeParams::flat(5.0));
        assert!(!faces.is_empty());

        // Front-cap triangles = those whose 3 verts are all at z≈0.
        let front: Vec<&[u32; 3]> = faces
            .iter()
            .filter(|t| t.iter().all(|&i| pos[i as usize][2].abs() < 1e-3))
            .collect();
        assert!(!front.is_empty(), "no front-cap triangles");

        // (a) Total front-cap area ≈ 84 (hole subtracted, not 100).
        let area: f32 = front.iter().map(|t| tri_area_xy(&pos, t)).sum();
        assert!((area - 84.0).abs() < 0.5, "front cap area {area} != ~84 (hole not subtracted?)");

        // (b) No front-cap triangle centroid lies inside the hole.
        for t in &front {
            let a = pos[t[0] as usize];
            let b = pos[t[1] as usize];
            let c = pos[t[2] as usize];
            let cx = (a[0] + b[0] + c[0]) / 3.0;
            let cy = (a[1] + b[1] + c[1]) / 3.0;
            let in_hole = cx > 3.0 && cx < 7.0 && cy > 3.0 && cy < 7.0;
            assert!(!in_hole, "cap triangle centroid ({cx},{cy}) inside the hole");
        }

        // (c) Front-cap normals point −Z.
        for t in &front {
            for &i in t.iter() {
                assert!(nrm[i as usize][2] < 0.0, "front cap normal not −Z");
            }
        }
    }

    #[test]
    fn concave_l_shape_fills() {
        // An L (concave) — a triangle fan from vertex 0 would spill outside.
        let l = vec![
            [0.0, 0.0],
            [6.0, 0.0],
            [6.0, 2.0],
            [2.0, 2.0],
            [2.0, 6.0],
            [0.0, 6.0],
        ];
        let (pos, _n, faces) = extrude_glyph(&[l], &ExtrudeParams::flat(3.0));
        let front: Vec<&[u32; 3]> = faces
            .iter()
            .filter(|t| t.iter().all(|&i| pos[i as usize][2].abs() < 1e-3))
            .collect();
        let area: f32 = front.iter().map(|t| tri_area_xy(&pos, t)).sum();
        // L area = 6*2 + 2*4 = 20.
        assert!((area - 20.0).abs() < 0.5, "L-shape front area {area} != ~20");
    }

    #[test]
    fn miter_bevel_insets_tunnel_and_keeps_full_caps() {
        // 10x10 square, depth 6, miter bevel 1.0.
        let sq = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let params = ExtrudeParams { depth: 6.0, bevel_type: 1, bevel_depth: 1.0, smoothness: 0, front: true, back: true, tunnel: true };
        let (pos, _n, faces) = extrude_glyph(&[sq], &params);
        assert!(!faces.is_empty());

        // Front cap is still the FULL outline (area ~100) at z=0.
        let front: Vec<&[u32; 3]> = faces
            .iter()
            .filter(|t| t.iter().all(|&i| pos[i as usize][2].abs() < 1e-3))
            .collect();
        let area: f32 = front.iter().map(|t| tri_area_xy(&pos, t)).sum();
        assert!((area - 100.0).abs() < 0.5, "beveled front cap area {area} != ~100 (cap should stay full)");

        // The bevel must inset the wall by ~bevelDepth: corner (0,0) → (1,1).
        // So the inset left edge sits at x≈1 (vs the full outline's x=0/10), at a
        // z beyond the front cap plane.
        let inset_seen = pos.iter().any(|p| (p[0] - 1.0).abs() < 0.25 && p[2] > 0.5);
        assert!(inset_seen, "no inset wall vertices (x≈1) — bevel did not inset");
    }

    #[test]
    fn round_bevel_produces_more_rings_than_miter() {
        let sq = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let miter = ExtrudeParams { depth: 6.0, bevel_type: 1, bevel_depth: 1.0, smoothness: 6, front: true, back: true, tunnel: true };
        let round = ExtrudeParams { depth: 6.0, bevel_type: 2, bevel_depth: 1.0, smoothness: 6, front: true, back: true, tunnel: true };
        let (_pm, _nm, fm) = extrude_glyph(&[sq.clone()], &miter);
        let (_pr, _nr, fr) = extrude_glyph(&[sq], &round);
        assert!(fr.len() > fm.len(), "round bevel ({}) should add arc rings vs miter ({})", fr.len(), fm.len());
    }

    fn alpha_bitmap(w: u32, h: u32, fill: &dyn Fn(u32, u32) -> bool) -> Vec<u8> {
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                if fill(x, y) {
                    rgba[((y * w + x) * 4 + 3) as usize] = 255;
                }
            }
        }
        rgba
    }

    #[test]
    fn vectorize_filled_square_one_contour() {
        let (w, h) = (10u32, 10u32);
        let rgba = alpha_bitmap(w, h, &|x, y| (3..7).contains(&x) && (3..7).contains(&y));
        let contours = vectorize_alpha(w, h, &rgba, 128, 0.0, 0, 0);
        assert_eq!(contours.len(), 1, "expected 1 contour, got {}", contours.len());
        // 50% crossing → square spans ~[2.5,6.5] → area ~16.
        let area = signed_area(&contours[0]).abs();
        assert!((area - 16.0).abs() < 2.0, "vectorized square area {area} != ~16");
    }

    #[test]
    fn vectorize_square_with_hole_two_contours_and_hollow_cap() {
        let (w, h) = (12u32, 12u32);
        let rgba = alpha_bitmap(w, h, &|x, y| {
            (2..10).contains(&x) && (2..10).contains(&y) && !((5..7).contains(&x) && (5..7).contains(&y))
        });
        let contours = vectorize_alpha(w, h, &rgba, 128, 0.0, 0, 0);
        assert_eq!(contours.len(), 2, "expected outer+hole = 2 contours, got {}", contours.len());
        // Feed into extrude_glyph: the cap must subtract the hole (outer ~64 − hole ~4 = ~60).
        let (pos, _n, faces) = extrude_glyph(&contours, &ExtrudeParams::flat(2.0));
        let front: Vec<&[u32; 3]> = faces
            .iter()
            .filter(|t| t.iter().all(|&i| pos[i as usize][2].abs() < 1e-3))
            .collect();
        let area: f32 = front.iter().map(|t| tri_area_xy(&pos, t)).sum();
        assert!((area - 60.0).abs() < 6.0, "vectorized hole cap area {area} != ~60");
    }

    #[test]
    fn vectorize_aa_edge_no_sawtooth() {
        // Anti-aliased vertical edge → cross_t ≈ 0.335 (not 0.5). The left boundary
        // must trace a straight line; the old mirrored bottom/left crossings made it
        // sawtooth between adjacent rows (which fragmented real text).
        let (w, h) = (10u32, 8u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 1..7 {
            for x in 0..w {
                let a: u8 = match x {
                    3 => 64,
                    4 | 5 => 255,
                    6 => 64,
                    _ => 0,
                };
                rgba[((y * w + x) * 4 + 3) as usize] = a;
            }
        }
        let cs = vectorize_alpha(w, h, &rgba, 128, 0.0, 0, 0);
        assert_eq!(cs.len(), 1, "expected 1 contour, got {}", cs.len());
        let xs: Vec<f32> = cs[0].iter().map(|p| p[0]).filter(|&x| x > 3.0 && x < 4.0).collect();
        assert!(xs.len() >= 2, "no left-edge crossing points");
        let mn = xs.iter().cloned().fold(f32::MAX, f32::min);
        let mx = xs.iter().cloned().fold(f32::MIN, f32::max);
        assert!(mx - mn < 0.1, "left edge sawtooths: x in [{mn}, {mx}] (mirrored-crossing bug)");
    }

    #[test]
    fn display_face_front_only_omits_back_and_tunnel() {
        let sq = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let p = ExtrudeParams {
            depth: 5.0,
            bevel_type: 0,
            bevel_depth: 0.0,
            smoothness: 0,
            front: true,
            back: false,
            tunnel: false,
        };
        let (pos, _n, faces) = extrude_glyph(&[sq], &p);
        assert!(!faces.is_empty(), "front-only produced no faces");
        // Every face must lie on the front cap plane (z≈0): no back cap, no walls.
        for t in &faces {
            for &i in t.iter() {
                assert!(
                    pos[i as usize][2].abs() < 1e-3,
                    "front-only displayFace emitted non-front geometry at z={}",
                    pos[i as usize][2]
                );
            }
        }
    }

    #[test]
    fn display_face_tunnel_only_has_no_cap_planes() {
        let sq = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let p = ExtrudeParams {
            depth: 5.0,
            bevel_type: 0,
            bevel_depth: 0.0,
            smoothness: 0,
            front: false,
            back: false,
            tunnel: true,
        };
        let (pos, _n, faces) = extrude_glyph(&[sq], &p);
        assert!(!faces.is_empty(), "tunnel-only produced no faces");
        // No face may be a flat cap (all three verts on z=0 or all on z=depth).
        for t in &faces {
            let zs: Vec<f32> = t.iter().map(|&i| pos[i as usize][2]).collect();
            let all_front = zs.iter().all(|z| z.abs() < 1e-3);
            let all_back = zs.iter().all(|z| (z - 5.0).abs() < 1e-3);
            assert!(!all_front && !all_back, "tunnel-only emitted a cap face");
        }
    }
}
