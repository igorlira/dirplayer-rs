//! PhysX helper math — Welzl `Miniball`, convex decomposition POD types,
//! shared math primitives used by the AGEIA wrapper layer.
//!
//! The native bodies are all in `physicsworldageia.o` alongside
//! `CPhysicsWorldAGEIA`. We model the public surface here; the actual
//! solver math is filled in incrementally as the integrator lands.

/// 3D vector helper used by Miniball and convex decomposition.
#[inline]
pub fn vec3_sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
pub fn vec3_dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
pub fn vec3_cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
pub fn vec3_length_sq(a: [f64; 3]) -> f64 { vec3_dot(a, a) }

/// Smallest enclosing ball — direct port of `CPhysicsWorldAGEIA`'s
/// embedded Welzl/Gärtner `Miniball` class.
///
/// Phase 1 holds a center + squared radius; the actual recursion is
/// stubbed (matches the C# port's "structure documented, body skeleton"
/// status). Inertia-tensor bias for dynamic bodies will need this filled
/// in.
#[derive(Debug, Clone)]
pub struct Miniball {
    pub center: [f64; 3],
    pub radius_sq: f64,
}

impl Miniball {
    pub fn empty() -> Self { Self { center: [0.0; 3], radius_sq: 0.0 } }

    /// Squared distance from the ball's center to a point.
    /// Mirrors `Miniball::d2(p)` — returns *signed* distance² (negative ⇒ inside).
    pub fn d2(&self, p: [f64; 3]) -> f64 {
        vec3_length_sq(vec3_sub(p, self.center)) - self.radius_sq
    }

    /// Distance.
    pub fn d(&self, p: [f64; 3]) -> f64 {
        vec3_length_sq(vec3_sub(p, self.center)).sqrt()
    }
}

/// Convex decomposition output — mirrors AGEIA's
/// `ConvexDecomposition::ConvexResult` POD.
#[derive(Debug, Clone, Default)]
pub struct ConvexResult {
    pub vertices: Vec<[f64; 3]>,
    pub indices: Vec<u32>,
    pub volume_center: [f64; 3],
    pub volume: f64,
}
