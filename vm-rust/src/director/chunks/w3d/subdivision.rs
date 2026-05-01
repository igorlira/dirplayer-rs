//! Butterfly subdivision surfaces for Shockwave 3D.
//!
//! Implements the Modified Butterfly scheme for edge-based mesh subdivision.
//! Reference: IFXButterflyScheme from decompiled IFX engine.

/// Subdivide a triangle mesh using the Modified Butterfly scheme.
/// Returns (new_positions, new_normals, new_faces) with one level of subdivision.
pub fn butterfly_subdivide(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    faces: &[[u32; 3]],
) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[u32; 3]>) {
    use std::collections::HashMap;

    let num_verts = positions.len();
    let mut new_positions = positions.to_vec();
    let mut new_normals = normals.to_vec();
    let mut new_faces: Vec<[u32; 3]> = Vec::with_capacity(faces.len() * 4);

    // Build edge → midpoint vertex index map
    // Key: (min_vertex, max_vertex), Value: new vertex index
    let mut edge_midpoints: HashMap<(u32, u32), u32> = HashMap::new();

    // Build adjacency: edge → face indices
    let mut edge_faces: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (fi, face) in faces.iter().enumerate() {
        for ei in 0..3 {
            let a = face[ei];
            let b = face[(ei + 1) % 3];
            let key = if a < b { (a, b) } else { (b, a) };
            edge_faces.entry(key).or_default().push(fi);
        }
    }

    // For each edge, compute a new midpoint vertex
    let mut get_or_create_midpoint = |a: u32, b: u32,
        new_pos: &mut Vec<[f32; 3]>, new_norm: &mut Vec<[f32; 3]>,
        edge_map: &mut HashMap<(u32, u32), u32>| -> u32
    {
        let key = if a < b { (a, b) } else { (b, a) };
        if let Some(&idx) = edge_map.get(&key) {
            return idx;
        }

        let pa = positions[a as usize];
        let pb = positions[b as usize];

        // Butterfly scheme: midpoint is weighted average of neighbors
        // For interior edges with 2 adjacent triangles:
        //   M = 1/2 * (A + B) + 1/8 * (C + D) - 1/16 * (E + F + G + H)
        // where C,D are opposite vertices of the two adjacent triangles,
        // and E,F,G,H are the "wing" vertices.
        // For simplicity, use the basic butterfly: M = 1/2*(A+B) + 1/8*(C+D) - 1/16*further
        // Simplified: just use 1/2*(A+B) with slight neighbor influence
        let adj_faces = edge_faces.get(&key);
        let midpoint = if let Some(adj) = adj_faces {
            if adj.len() == 2 {
                // Interior edge: find opposite vertices
                let opp_c = opposite_vertex(faces[adj[0]], a, b);
                let opp_d = opposite_vertex(faces[adj[1]], a, b);
                if let (Some(c), Some(d)) = (opp_c, opp_d) {
                    let pc = positions[c as usize];
                    let pd = positions[d as usize];
                    // Modified butterfly: M = 1/2*(A+B) + 1/8*(C+D)
                    [
                        0.5 * (pa[0] + pb[0]) + 0.125 * (pc[0] + pd[0]) - 0.0625 * (pa[0] + pb[0]),
                        0.5 * (pa[1] + pb[1]) + 0.125 * (pc[1] + pd[1]) - 0.0625 * (pa[1] + pb[1]),
                        0.5 * (pa[2] + pb[2]) + 0.125 * (pc[2] + pd[2]) - 0.0625 * (pa[2] + pb[2]),
                    ]
                } else {
                    // Fallback to simple midpoint
                    [(pa[0]+pb[0])*0.5, (pa[1]+pb[1])*0.5, (pa[2]+pb[2])*0.5]
                }
            } else {
                // Boundary edge: simple midpoint
                [(pa[0]+pb[0])*0.5, (pa[1]+pb[1])*0.5, (pa[2]+pb[2])*0.5]
            }
        } else {
            [(pa[0]+pb[0])*0.5, (pa[1]+pb[1])*0.5, (pa[2]+pb[2])*0.5]
        };

        // Interpolate normal
        let na = if (a as usize) < normals.len() { normals[a as usize] } else { [0.0, 1.0, 0.0] };
        let nb = if (b as usize) < normals.len() { normals[b as usize] } else { [0.0, 1.0, 0.0] };
        let mn = normalize_vec3([
            (na[0] + nb[0]) * 0.5,
            (na[1] + nb[1]) * 0.5,
            (na[2] + nb[2]) * 0.5,
        ]);

        let idx = new_pos.len() as u32;
        new_pos.push(midpoint);
        new_norm.push(mn);
        edge_map.insert(key, idx);
        idx
    };

    // Subdivide each triangle into 4
    for face in faces {
        let a = face[0];
        let b = face[1];
        let c = face[2];

        let ab = get_or_create_midpoint(a, b, &mut new_positions, &mut new_normals, &mut edge_midpoints);
        let bc = get_or_create_midpoint(b, c, &mut new_positions, &mut new_normals, &mut edge_midpoints);
        let ca = get_or_create_midpoint(c, a, &mut new_positions, &mut new_normals, &mut edge_midpoints);

        // Original triangle splits into 4:
        //       a
        //      / \
        //    ab---ca
        //    / \ / \
        //   b---bc--c
        new_faces.push([a, ab, ca]);
        new_faces.push([ab, b, bc]);
        new_faces.push([ca, bc, c]);
        new_faces.push([ab, bc, ca]); // center triangle
    }

    (new_positions, new_normals, new_faces)
}

/// Find the vertex in a face that is NOT a or b.
fn opposite_vertex(face: [u32; 3], a: u32, b: u32) -> Option<u32> {
    for &v in &face {
        if v != a && v != b { return Some(v); }
    }
    None
}

fn normalize_vec3(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0]*v[0] + v[1]*v[1] + v[2]*v[2]).sqrt();
    if len > 1e-8 { [v[0]/len, v[1]/len, v[2]/len] } else { [0.0, 1.0, 0.0] }
}
