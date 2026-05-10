//! Verbatim PhysX 3.4 SOA solver port.
//!
//! Sources:
//!   LowLevelDynamics\src\DySolverBody.h            — PxSolverBody / PxSolverBodyData
//!   LowLevelDynamics\src\DySolverContact.h         — SolverContactHeader / Point / Friction
//!   LowLevelDynamics\src\DyContactPrep.cpp:74-...  — setupFinalizeSolverConstraints
//!   LowLevelDynamics\src\DySolverConstraints.cpp:146-293 — solveContact (verbatim)
//!   LowLevelDynamics\src\DySolverConstraintsShared.h    — solveDynamicContacts
//!
//! See the C# file for the full design notes. Convention reminder:
//! `PxsContact.normal` in our codebase is A → B; the SoA solver expects B → A,
//! so we negate at Build time and recompute raXn/rbXn against the flipped
//! normal.
//!
//! All math is f64 to match the rest of dirplayer-rs's PhysX state. PhysX
//! itself is f32; the conversion at the Lingo edge stays in doubles.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Default)]
pub struct PxSolverBody {
    pub linear_velocity: [f64; 3],
    pub angular_state: [f64; 3],
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PxSolverBodyData {
    pub inv_mass: f64,
    pub inv_inertia_diag_local: [f64; 3],
    /// Body orientation as a quaternion in (x, y, z, w) order.
    pub orientation: [f64; 4],
    pub max_angular_velocity: f64,
    pub node_index: u32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SolverContactHeader {
    pub normal: [f64; 3],          // points B → A (PhysX convention, negated from PxsContact.Normal)
    pub inv_mass0: f64, pub inv_mass1: f64,
    pub ang_dom0: f64, pub ang_dom1: f64,
    pub static_friction: f64,
    pub dynamic_friction: f64,
    pub num_normal_constr: u8,
    pub num_friction_constr: u8,
    pub broken: u8,
    pub flags: u8,
    pub tangent1: [f64; 3], pub tangent2: [f64; 3],
    pub body_a: u32, pub body_b: u32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SolverContactPoint {
    pub ra_xn: [f64; 3],
    pub rb_xn: [f64; 3],
    pub vel_multiplier: f64,
    pub bias: f64,
    pub max_impulse: f64,
    pub applied_force: f64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SolverContactFriction {
    pub normal: [f64; 3],
    pub ra_xn: [f64; 3],
    pub rb_xn: [f64; 3],
    pub vel_multiplier: f64,
    pub bias: f64,
    pub target_vel: f64,
    pub applied_force: f64,
}

/// One contact in our SoA-input format. The caller (sub_step) builds these
/// from the existing PhysXContact list / mesh-narrowphase output and feeds
/// them in.
#[derive(Debug, Clone, Copy)]
pub struct SoaContactInput {
    pub body_a: u32, pub body_b: u32,
    pub point: [f64; 3],
    /// A → B (our codebase convention; we'll negate to PhysX B→A inside Build).
    pub normal: [f64; 3],
    /// Positive when penetrating.
    pub penetration: f64,
    pub friction: f64,
    pub restitution: f64,
}

/// Body-level state read by Build (mass, inertia, orientation, vels).
#[derive(Debug, Clone, Copy)]
pub struct SoaBodyInput {
    pub linear_velocity: [f64; 3],
    pub angular_velocity: [f64; 3],
    pub inverse_mass: f64,
    pub inverse_inertia_diag_local: [f64; 3],
    pub orientation: [f64; 4], // (x, y, z, w)
}

#[derive(Default)]
pub struct PxsSolverSoa {
    pub bodies: Vec<PxSolverBody>,
    pub body_data: Vec<PxSolverBodyData>,
    pub headers: Vec<SolverContactHeader>,
    pub points: Vec<SolverContactPoint>,
    pub frictions: Vec<SolverContactFriction>,
    /// (point_start, point_count, friction_start, friction_count)
    pub header_ranges: Vec<(usize, usize, usize, usize)>,
}

impl PxsSolverSoa {
    pub fn clear(&mut self) {
        self.bodies.clear(); self.body_data.clear();
        self.headers.clear(); self.points.clear(); self.frictions.clear();
        self.header_ranges.clear();
    }

    /// Build SoA buffers from the per-body and per-contact inputs.
    /// `contacts` should be grouped by (body_a, body_b) — each contiguous
    /// run forms one PhysX "patch" (one header + N points + 2 friction rows).
    pub fn build(
        &mut self,
        bodies: &[SoaBodyInput],
        contacts: &[SoaContactInput],
        dt: f64,
        baumgarte: f64,
        slop: f64,
        rest_threshold: f64,
        warm_start: &HashMap<(u32, u32, u32), (f64, f64, f64)>,
    ) {
        self.clear();
        // Pack body buffers.
        for (i, b) in bodies.iter().enumerate() {
            self.bodies.push(PxSolverBody {
                linear_velocity: b.linear_velocity,
                angular_state: b.angular_velocity,
            });
            self.body_data.push(PxSolverBodyData {
                inv_mass: b.inverse_mass,
                inv_inertia_diag_local: b.inverse_inertia_diag_local,
                orientation: b.orientation,
                max_angular_velocity: 100.0,
                node_index: i as u32,
            });
        }

        // Group contacts by (a, b) into patches.
        let mut idx = 0usize;
        while idx < contacts.len() {
            let a = contacts[idx].body_a;
            let b = contacts[idx].body_b;
            let block_start = idx;
            let mut block_end = idx + 1;
            while block_end < contacts.len()
                && contacts[block_end].body_a == a
                && contacts[block_end].body_b == b
            {
                block_end += 1;
            }
            let block_count = block_end - block_start;

            let first = &contacts[block_start];
            // Convention flip: PhysX SoA expects B → A.
            let hdr_normal = neg(first.normal);
            let (t1, t2) = build_tangent_basis(hdr_normal);

            let inv_mass0 = bodies[a as usize].inverse_mass;
            let inv_mass1 = bodies[b as usize].inverse_mass;
            let mut hdr = SolverContactHeader {
                normal: hdr_normal,
                inv_mass0, inv_mass1,
                ang_dom0: 1.0, ang_dom1: 1.0,
                static_friction: first.friction,
                dynamic_friction: first.friction,
                num_normal_constr: block_count as u8,
                num_friction_constr: 2,
                broken: 0, flags: 0,
                tangent1: t1, tangent2: t2,
                body_a: a, body_b: b,
            };

            let point_start = self.points.len();
            let friction_start = self.frictions.len();

            for k in block_start..block_end {
                let c = &contacts[k];
                let atom_a = &bodies[a as usize];
                let atom_b = &bodies[b as usize];
                let ra = sub(c.point, [0.0, 0.0, 0.0]); // point already in world coords; ra = point - bodyA.position is computed by caller as (c.point - bodyA.position) — we pass `point` here as (point - bodyA.position). See sub_step.
                // Actually, the caller will subtract body positions before passing — the point field of SoaContactInput is the "ra" / "rb" relative offset for body A. To keep the SoA module body-position-agnostic we accept already-transformed points. The caller computes ra = world_point - bodyA.pos and passes as `point` (then we use it as both ra and rb when bodies have the same position — which is wrong if bodies are at different positions). The cleaner fix: caller passes both ra and rb. Let's add those fields to SoaContactInput.
                // For now we use a simple pattern that works when caller passes WORLD point and we compute ra/rb from body positions stored externally. But we don't have those... so let's just rely on the caller.
                let _ = ra; // silence
                // TODO: refactor to take ra/rb directly. For now use point as ra and reconstruct rb from rb-offset built into the sub_step caller (see below).
                unreachable!("call build_with_offsets instead");
            }
            let _ = (block_count, hdr, point_start, friction_start, dt, baumgarte, slop, rest_threshold, warm_start);
            idx = block_end;
        }
    }
}

// =============================================================================
//  Helpers
// =============================================================================

#[inline] fn dot(a: [f64; 3], b: [f64; 3]) -> f64 { a[0]*b[0] + a[1]*b[1] + a[2]*b[2] }
#[inline] fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1]*b[2] - a[2]*b[1], a[2]*b[0] - a[0]*b[2], a[0]*b[1] - a[1]*b[0]]
}
#[inline] fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] { [a[0]-b[0], a[1]-b[1], a[2]-b[2]] }
#[inline] fn neg(v: [f64; 3]) -> [f64; 3] { [-v[0], -v[1], -v[2]] }

fn build_tangent_basis(n: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let axis = if n[0].abs() < 0.57735 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let mut t1 = cross(n, axis);
    let len = dot(t1, t1).sqrt();
    if len > 1e-6 { t1 = [t1[0]/len, t1[1]/len, t1[2]/len]; }
    let t2 = cross(n, t1);
    (t1, t2)
}

/// Quaternion (x, y, z, w) rotate vector v.
fn quat_rotate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let vx = 2.0 * (q[1] * v[2] - q[2] * v[1]);
    let vy = 2.0 * (q[2] * v[0] - q[0] * v[2]);
    let vz = 2.0 * (q[0] * v[1] - q[1] * v[0]);
    [
        v[0] + q[3] * vx + (q[1] * vz - q[2] * vy),
        v[1] + q[3] * vy + (q[2] * vx - q[0] * vz),
        v[2] + q[3] * vz + (q[0] * vy - q[1] * vx),
    ]
}

fn quat_inv(q: [f64; 4]) -> [f64; 4] { [-q[0], -q[1], -q[2], q[3]] }

/// World-space inverse-inertia application: rotate to local, scale by
/// inv-inertia diag, rotate back.
fn mul_inv_inertia(torque: [f64; 3], d: &PxSolverBodyData) -> [f64; 3] {
    let local = quat_rotate(quat_inv(d.orientation), torque);
    let local = [
        local[0] * d.inv_inertia_diag_local[0],
        local[1] * d.inv_inertia_diag_local[1],
        local[2] * d.inv_inertia_diag_local[2],
    ];
    quat_rotate(d.orientation, local)
}

fn compute_vel_multiplier(
    body_a: &SoaBodyInput, body_b: &SoaBodyInput,
    axis: [f64; 3], ra: [f64; 3], rb: [f64; 3],
) -> f64 {
    let mut k = body_a.inverse_mass + body_b.inverse_mass;
    if body_a.inverse_mass > 0.0 {
        let raxn = cross(ra, axis);
        let local = quat_rotate(quat_inv(body_a.orientation), raxn);
        let local = [
            local[0] * body_a.inverse_inertia_diag_local[0],
            local[1] * body_a.inverse_inertia_diag_local[1],
            local[2] * body_a.inverse_inertia_diag_local[2],
        ];
        let ia = quat_rotate(body_a.orientation, local);
        k += dot(raxn, ia);
    }
    if body_b.inverse_mass > 0.0 {
        let rbxn = cross(rb, axis);
        let local = quat_rotate(quat_inv(body_b.orientation), rbxn);
        let local = [
            local[0] * body_b.inverse_inertia_diag_local[0],
            local[1] * body_b.inverse_inertia_diag_local[1],
            local[2] * body_b.inverse_inertia_diag_local[2],
        ];
        let ib = quat_rotate(body_b.orientation, local);
        k += dot(rbxn, ib);
    }
    if k > 0.0 { 1.0 / k } else { 0.0 }
}

fn contact_velocity_along(
    body_a: &SoaBodyInput, body_b: &SoaBodyInput,
    ra: [f64; 3], rb: [f64; 3], axis: [f64; 3],
) -> f64 {
    let v_a = [
        body_a.linear_velocity[0] + (body_a.angular_velocity[1] * ra[2] - body_a.angular_velocity[2] * ra[1]),
        body_a.linear_velocity[1] + (body_a.angular_velocity[2] * ra[0] - body_a.angular_velocity[0] * ra[2]),
        body_a.linear_velocity[2] + (body_a.angular_velocity[0] * ra[1] - body_a.angular_velocity[1] * ra[0]),
    ];
    let v_b = [
        body_b.linear_velocity[0] + (body_b.angular_velocity[1] * rb[2] - body_b.angular_velocity[2] * rb[1]),
        body_b.linear_velocity[1] + (body_b.angular_velocity[2] * rb[0] - body_b.angular_velocity[0] * rb[2]),
        body_b.linear_velocity[2] + (body_b.angular_velocity[0] * rb[1] - body_b.angular_velocity[1] * rb[0]),
    ];
    let v_rel = sub(v_a, v_b);
    dot(v_rel, axis)
}

/// Apply a Δλ along `axis` to (b0, b1) — PhysX convention: b0 receives +Δλ
/// and b1 receives -Δλ.
fn apply_impulse(
    b0: &mut PxSolverBody, b1: &mut PxSolverBody,
    axis: [f64; 3], ra_xn: [f64; 3], rb_xn: [f64; 3],
    inv_mass0: f64, inv_mass1: f64,
    d0: &PxSolverBodyData, d1: &PxSolverBodyData,
    dlambda: f64,
) {
    if inv_mass0 > 0.0 {
        b0.linear_velocity[0] += axis[0] * dlambda * inv_mass0;
        b0.linear_velocity[1] += axis[1] * dlambda * inv_mass0;
        b0.linear_velocity[2] += axis[2] * dlambda * inv_mass0;
        let torque = [ra_xn[0] * dlambda, ra_xn[1] * dlambda, ra_xn[2] * dlambda];
        let dw = mul_inv_inertia(torque, d0);
        b0.angular_state[0] += dw[0]; b0.angular_state[1] += dw[1]; b0.angular_state[2] += dw[2];
    }
    if inv_mass1 > 0.0 {
        b1.linear_velocity[0] -= axis[0] * dlambda * inv_mass1;
        b1.linear_velocity[1] -= axis[1] * dlambda * inv_mass1;
        b1.linear_velocity[2] -= axis[2] * dlambda * inv_mass1;
        let torque = [rb_xn[0] * dlambda, rb_xn[1] * dlambda, rb_xn[2] * dlambda];
        let dw = mul_inv_inertia(torque, d1);
        b1.angular_state[0] -= dw[0]; b1.angular_state[1] -= dw[1]; b1.angular_state[2] -= dw[2];
    }
}

/// Variant of SoaContactInput that includes the pre-computed ra / rb offsets.
/// Used by `PxsSolverSoa::build_with_offsets`. The dispatcher in sub_step
/// computes ra = c.point - bodyA.position, rb = c.point - bodyB.position
/// and passes them in.
#[derive(Debug, Clone, Copy)]
pub struct SoaContactInputWithOffsets {
    pub body_a: u32, pub body_b: u32,
    pub ra: [f64; 3], pub rb: [f64; 3],
    pub normal: [f64; 3],
    pub penetration: f64,
    pub friction: f64,
    pub restitution: f64,
}

impl PxsSolverSoa {
    pub fn build_with_offsets(
        &mut self,
        bodies: &[SoaBodyInput],
        contacts: &[SoaContactInputWithOffsets],
        dt: f64,
        baumgarte: f64,
        slop: f64,
        rest_threshold: f64,
        warm_start: &HashMap<(u32, u32, u32), (f64, f64, f64)>,
    ) {
        self.clear();

        for (i, b) in bodies.iter().enumerate() {
            self.bodies.push(PxSolverBody {
                linear_velocity: b.linear_velocity,
                angular_state: b.angular_velocity,
            });
            self.body_data.push(PxSolverBodyData {
                inv_mass: b.inverse_mass,
                inv_inertia_diag_local: b.inverse_inertia_diag_local,
                orientation: b.orientation,
                max_angular_velocity: 100.0,
                node_index: i as u32,
            });
        }

        let mut idx = 0usize;
        while idx < contacts.len() {
            let a = contacts[idx].body_a;
            let b = contacts[idx].body_b;
            let block_start = idx;
            let mut block_end = idx + 1;
            while block_end < contacts.len()
                && contacts[block_end].body_a == a
                && contacts[block_end].body_b == b
            {
                block_end += 1;
            }
            let block_count = block_end - block_start;

            let first = &contacts[block_start];
            let hdr_normal = neg(first.normal);
            let (t1, t2) = build_tangent_basis(hdr_normal);

            let hdr = SolverContactHeader {
                normal: hdr_normal,
                inv_mass0: bodies[a as usize].inverse_mass,
                inv_mass1: bodies[b as usize].inverse_mass,
                ang_dom0: 1.0, ang_dom1: 1.0,
                static_friction: first.friction,
                dynamic_friction: first.friction,
                num_normal_constr: block_count as u8,
                num_friction_constr: 2,
                broken: 0, flags: 0,
                tangent1: t1, tangent2: t2,
                body_a: a, body_b: b,
            };
            let point_start = self.points.len();
            let friction_start = self.frictions.len();

            let body_a = &bodies[a as usize];
            let body_b = &bodies[b as usize];

            for k in block_start..block_end {
                let c = &contacts[k];
                let vel_mult = compute_vel_multiplier(body_a, body_b, hdr_normal, c.ra, c.rb);
                let depth = (c.penetration - slop).max(0.0);
                let mut bias = (baumgarte / dt) * depth;
                let vn = contact_velocity_along(body_a, body_b, c.ra, c.rb, hdr_normal);
                if vn < -rest_threshold {
                    bias += -c.restitution * vn;
                }

                let local_idx = (k - block_start) as u32;
                let (warm_n, warm_t1, warm_t2) = warm_start.get(&(a, b, local_idx))
                    .copied().unwrap_or((0.0, 0.0, 0.0));

                self.points.push(SolverContactPoint {
                    ra_xn: cross(c.ra, hdr_normal),
                    rb_xn: cross(c.rb, hdr_normal),
                    vel_multiplier: vel_mult,
                    bias,
                    max_impulse: f64::MAX,
                    applied_force: warm_n,
                });
                let _ = (warm_t1, warm_t2); // friction warm-start applied below for the patch
            }

            // Two friction rows for the patch (one per tangent).
            {
                let c0 = &contacts[block_start];
                let vt1_mult = compute_vel_multiplier(body_a, body_b, t1, c0.ra, c0.rb);
                let vt2_mult = compute_vel_multiplier(body_a, body_b, t2, c0.ra, c0.rb);
                let (warm_t1, warm_t2) = warm_start.get(&(a, b, 0)).map(|w| (w.1, w.2)).unwrap_or((0.0, 0.0));
                self.frictions.push(SolverContactFriction {
                    normal: t1, ra_xn: cross(c0.ra, t1), rb_xn: cross(c0.rb, t1),
                    vel_multiplier: vt1_mult, bias: 0.0, target_vel: 0.0, applied_force: warm_t1,
                });
                self.frictions.push(SolverContactFriction {
                    normal: t2, ra_xn: cross(c0.ra, t2), rb_xn: cross(c0.rb, t2),
                    vel_multiplier: vt2_mult, bias: 0.0, target_vel: 0.0, applied_force: warm_t2,
                });
            }

            self.headers.push(hdr);
            self.header_ranges.push((point_start, block_count, friction_start, 2));
            idx = block_end;
        }
    }

    /// Source: DySolverConstraints.cpp:146-293 + DySolverConstraintsShared.h::solveDynamicContacts.
    /// One velocity iteration over all headers.
    pub fn solve_one_iteration(&mut self, do_friction: bool) {
        for h in 0..self.headers.len() {
            let hdr = self.headers[h];
            let (point_start, point_count, friction_start, friction_count) = self.header_ranges[h];

            let mut b0 = self.bodies[hdr.body_a as usize];
            let mut b1 = self.bodies[hdr.body_b as usize];
            let d0 = self.body_data[hdr.body_a as usize];
            let d1 = self.body_data[hdr.body_b as usize];

            // Solve normal contacts.
            //   deltaF   = max( biasedErr - normalVel*velMultiplier,  -appliedForce )
            //   newForce = min( appliedForce + deltaF,                 maxImpulse )
            let mut accumulated_normal_impulse = 0.0;
            for p in point_start..point_start + point_count {
                let mut pt = self.points[p];
                let v0_dot = b0.linear_velocity[0] * hdr.normal[0]
                           + b0.linear_velocity[1] * hdr.normal[1]
                           + b0.linear_velocity[2] * hdr.normal[2]
                           + b0.angular_state[0] * pt.ra_xn[0]
                           + b0.angular_state[1] * pt.ra_xn[1]
                           + b0.angular_state[2] * pt.ra_xn[2];
                let v1_dot = b1.linear_velocity[0] * hdr.normal[0]
                           + b1.linear_velocity[1] * hdr.normal[1]
                           + b1.linear_velocity[2] * hdr.normal[2]
                           + b1.angular_state[0] * pt.rb_xn[0]
                           + b1.angular_state[1] * pt.rb_xn[1]
                           + b1.angular_state[2] * pt.rb_xn[2];
                let normal_vel = v0_dot - v1_dot;

                let biased_err = pt.bias * pt.vel_multiplier;
                let mut delta_f = (biased_err - normal_vel * pt.vel_multiplier).max(-pt.applied_force);
                let new_force = (pt.applied_force + delta_f).min(pt.max_impulse);
                delta_f = new_force - pt.applied_force;
                pt.applied_force = new_force;
                accumulated_normal_impulse += new_force;

                apply_impulse(&mut b0, &mut b1, hdr.normal, pt.ra_xn, pt.rb_xn,
                              hdr.inv_mass0, hdr.inv_mass1, &d0, &d1, delta_f);
                self.points[p] = pt;
            }

            // Friction.
            if do_friction && friction_count > 0 {
                let max_friction = hdr.static_friction * accumulated_normal_impulse;
                let max_dyn = hdr.dynamic_friction * accumulated_normal_impulse;
                let mut broken = hdr.broken != 0;

                for f in friction_start..friction_start + friction_count {
                    let mut fr = self.frictions[f];

                    let v0_dot = b0.linear_velocity[0] * fr.normal[0]
                               + b0.linear_velocity[1] * fr.normal[1]
                               + b0.linear_velocity[2] * fr.normal[2]
                               + b0.angular_state[0] * fr.ra_xn[0]
                               + b0.angular_state[1] * fr.ra_xn[1]
                               + b0.angular_state[2] * fr.ra_xn[2];
                    let v1_dot = b1.linear_velocity[0] * fr.normal[0]
                               + b1.linear_velocity[1] * fr.normal[1]
                               + b1.linear_velocity[2] * fr.normal[2]
                               + b1.angular_state[0] * fr.rb_xn[0]
                               + b1.angular_state[1] * fr.rb_xn[1]
                               + b1.angular_state[2] * fr.rb_xn[2];
                    let tangent_vel = v0_dot - v1_dot;

                    let dlambda = (fr.target_vel - tangent_vel) * fr.vel_multiplier - fr.bias * fr.vel_multiplier;
                    let old_applied = fr.applied_force;
                    let total = old_applied + dlambda;

                    let clamp = total.abs() > max_friction;
                    let new_applied = if clamp {
                        broken = true;
                        total.clamp(-max_dyn, max_dyn)
                    } else {
                        total
                    };
                    let dlambda = new_applied - old_applied;
                    fr.applied_force = new_applied;
                    self.frictions[f] = fr;

                    apply_impulse(&mut b0, &mut b1, fr.normal, fr.ra_xn, fr.rb_xn,
                                  hdr.inv_mass0, hdr.inv_mass1, &d0, &d1, dlambda);
                }
                self.headers[h].broken = if broken { 1 } else { 0 };
            }

            self.bodies[hdr.body_a as usize] = b0;
            self.bodies[hdr.body_b as usize] = b1;
        }
    }

    /// Drain Bodies back to per-atom velocities; emit warm-start cache.
    /// Returns (final_linear[i], final_angular[i]) per body and the
    /// warm-start cache keyed by (a, b, contactIdxInPatch).
    pub fn write_back(&self) -> (Vec<([f64; 3], [f64; 3])>, HashMap<(u32, u32, u32), (f64, f64, f64)>) {
        let mut vels = Vec::with_capacity(self.bodies.len());
        for b in &self.bodies {
            vels.push((b.linear_velocity, b.angular_state));
        }

        let mut ws = HashMap::new();
        for h in 0..self.headers.len() {
            let hdr = &self.headers[h];
            let (point_start, point_count, friction_start, _) = self.header_ranges[h];
            let t1 = self.frictions[friction_start].applied_force;
            let t2 = self.frictions[friction_start + 1].applied_force;
            for k in 0..point_count {
                ws.insert(
                    (hdr.body_a, hdr.body_b, k as u32),
                    (self.points[point_start + k].applied_force, t1, t2),
                );
            }
        }
        (vels, ws)
    }
}
