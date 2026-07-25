//! Butterfly subdivision surfaces for Shockwave 3D (the `#sds` modifier).
//!
//! Implements the Modified Butterfly (interpolating) scheme. Positions use the
//! 8-point interior butterfly mask; UVs/normals use linear midpoints reduce to
//! averaging for our closed meshes).
//!
//! - Interior edge midpoint: `0.5·(V0+V1) + 2w·(V2+V3) − w·(V4+V5+V6+V7)`
//!   where `V0,V1` are the edge endpoints, `V2,V3` the opposite (wing-hub)
//!   vertices of the two adjacent triangles, and `V4..V7` the four outer wing
//!   vertices. `w` is the butterfly smoothing factor.
//! - `w = 0.2·(1 − surfaceTension)` with `surfaceTension = tension/100`.
//!   Lingo `sds.tension` is 0..100; the default 65 gives `w = 0.07`.
//!   At tension 100 → `w = 0`, i.e. a plain linear midpoint.
//! - Boundary / irregular edges fall back to the simple midpoint.

use std::collections::HashMap;

type EdgeFaces = HashMap<(u32, u32), Vec<usize>>;

fn edge_key(a: u32, b: u32) -> (u32, u32) {
    if a < b { (a, b) } else { (b, a) }
}

/// Find the vertex in a face that is NOT a or b.
fn opposite_vertex(face: [u32; 3], a: u32, b: u32) -> Option<u32> {
    face.iter().copied().find(|&v| v != a && v != b)
}

/// The vertex opposite edge `(a,b)` in the single face adjacent to that edge
/// which is NOT `exclude_face`. Used to gather the outer butterfly wings.
fn wing_across(
    edge_faces: &EdgeFaces,
    faces: &[[u32; 3]],
    a: u32,
    b: u32,
    exclude_face: usize,
) -> Option<u32> {
    let adj = edge_faces.get(&edge_key(a, b))?;
    for &fi in adj {
        if fi != exclude_face {
            return opposite_vertex(faces[fi], a, b);
        }
    }
    None
}

fn normalize_vec3(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 1e-8 { [v[0] / len, v[1] / len, v[2] / len] } else { [0.0, 1.0, 0.0] }
}

/// One level of Modified Butterfly subdivision carrying positions, normals and
/// (optionally) one UV set. `tension` is Lingo's 0..100.
fn subdivide_once(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    faces: &[[u32; 3]],
    tension: f32,
) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<[u32; 3]>) {
    // Butterfly smoothing factor: w = 0.2 * (1 - surfaceTension).
    let w = 0.2 * (1.0 - (tension / 100.0).clamp(0.0, 1.0));

    let mut new_positions = positions.to_vec();
    let mut new_normals = normals.to_vec();
    let mut new_uvs = uvs.to_vec();
    let mut new_faces: Vec<[u32; 3]> = Vec::with_capacity(faces.len() * 4);

    // edge → adjacent face indices
    let mut edge_faces: EdgeFaces = HashMap::new();
    for (fi, face) in faces.iter().enumerate() {
        for ei in 0..3 {
            edge_faces
                .entry(edge_key(face[ei], face[(ei + 1) % 3]))
                .or_default()
                .push(fi);
        }
    }

    let has_uvs = !uvs.is_empty();
    let mut edge_midpoints: HashMap<(u32, u32), u32> = HashMap::new();

    for (fi, face) in faces.iter().enumerate() {
        let mut mids = [0u32; 3];
        for ei in 0..3 {
            let a = face[ei];
            let b = face[(ei + 1) % 3];
            let key = edge_key(a, b);
            let idx = if let Some(&idx) = edge_midpoints.get(&key) {
                idx
            } else {
                let pa = positions[a as usize];
                let pb = positions[b as usize];

                // Interior 8-point butterfly if the edge has two triangles and
                // all four outer wings exist; otherwise simple midpoint.
                let adj = edge_faces.get(&key);
                let pos = 'pos: {
                    if let Some(adj) = adj {
                        if adj.len() == 2 {
                            let c = opposite_vertex(faces[adj[0]], a, b);
                            let d = opposite_vertex(faces[adj[1]], a, b);
                            if let (Some(c), Some(d)) = (c, d) {
                                let wings = [
                                    wing_across(&edge_faces, faces, a, c, adj[0]),
                                    wing_across(&edge_faces, faces, b, c, adj[0]),
                                    wing_across(&edge_faces, faces, a, d, adj[1]),
                                    wing_across(&edge_faces, faces, b, d, adj[1]),
                                ];
                                if wings.iter().all(|x| x.is_some()) {
                                    let pc = positions[c as usize];
                                    let pd = positions[d as usize];
                                    let mut m = [
                                        0.5 * (pa[0] + pb[0]) + 2.0 * w * (pc[0] + pd[0]),
                                        0.5 * (pa[1] + pb[1]) + 2.0 * w * (pc[1] + pd[1]),
                                        0.5 * (pa[2] + pb[2]) + 2.0 * w * (pc[2] + pd[2]),
                                    ];
                                    for wg in wings.iter().flatten() {
                                        let p = positions[*wg as usize];
                                        m[0] -= w * p[0];
                                        m[1] -= w * p[1];
                                        m[2] -= w * p[2];
                                    }
                                    break 'pos m;
                                }
                            }
                        }
                    }
                    [
                        0.5 * (pa[0] + pb[0]),
                        0.5 * (pa[1] + pb[1]),
                        0.5 * (pa[2] + pb[2]),
                    ]
                };

                // Normals / UVs: linear midpoint (interpolating attributes).
                let na = normals.get(a as usize).copied().unwrap_or([0.0, 1.0, 0.0]);
                let nb = normals.get(b as usize).copied().unwrap_or([0.0, 1.0, 0.0]);
                let mn = normalize_vec3([
                    (na[0] + nb[0]) * 0.5,
                    (na[1] + nb[1]) * 0.5,
                    (na[2] + nb[2]) * 0.5,
                ]);

                let idx = new_positions.len() as u32;
                new_positions.push(pos);
                new_normals.push(mn);
                if has_uvs {
                    let ua = uvs[a as usize];
                    let ub = uvs[b as usize];
                    new_uvs.push([(ua[0] + ub[0]) * 0.5, (ua[1] + ub[1]) * 0.5]);
                }
                edge_midpoints.insert(key, idx);
                idx
            };
            mids[ei] = idx;
        }

        // Split triangle (a,b,c) into 4 using the three edge midpoints.
        let (a, b, c) = (face[0], face[1], face[2]);
        let (ab, bc, ca) = (mids[0], mids[1], mids[2]);
        let _ = fi;
        new_faces.push([a, ab, ca]);
        new_faces.push([ab, b, bc]);
        new_faces.push([ca, bc, c]);
        new_faces.push([ab, bc, ca]);
    }

    (new_positions, new_normals, new_uvs, new_faces)
}

/// Apply `levels` iterations of Modified Butterfly subdivision, carrying
/// positions, normals and one UV set. `tension` is Lingo's `sds.tension`
/// (0..100). `levels` maps to `sds.depth`.
pub fn subdivide(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    faces: &[[u32; 3]],
    levels: u32,
    tension: f32,
) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<[u32; 3]>) {
    let mut p = positions.to_vec();
    let mut n = normals.to_vec();
    let mut u = uvs.to_vec();
    let mut f = faces.to_vec();
    for _ in 0..levels {
        if f.is_empty() {
            break;
        }
        let (np, nn, nu, nf) = subdivide_once(&p, &n, &u, &f, tension);
        p = np;
        n = nn;
        u = nu;
        f = nf;
    }
    // Normals are the REFINED original smooth normals (interpolated per level in
    // subdivide_once), NOT geometric normals recomputed from the subdivided
    // faces. Director refines the source normal field, so the surface keeps its
    // smooth radial shading even when the geometry stays faceted — e.g. at
    // tension 100 (w=0, linear) the pitcher still shades smooth-round. A
    // geometric recompute would flat-shade each coplanar facet and produce
    // visible facets, which Director does not.
    (p, n, u, f)
}

/// One level of subdivision (positions/normals/faces only), kept for callers
/// that don't carry UVs. Uses the default tension (65 → butterfly `w = 0.07`).
pub fn butterfly_subdivide(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    faces: &[[u32; 3]],
) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[u32; 3]>) {
    let (p, n, _u, f) = subdivide(positions, normals, &[], faces, 1, 65.0);
    (p, n, f)
}
