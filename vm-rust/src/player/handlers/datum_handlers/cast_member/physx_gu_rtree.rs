//! Gu::RTree — verbatim port of PhysX 3.4's quad-tree BVH used as the
//! triangle-mesh midphase.
//!
//! See the C# file for an extensive header. Rust differences:
//!   - All vectors are `[f64; 3]` to match the rest of dirplayer-rs's PhysX
//!     state (which uses doubles end-to-end). PhysX itself is float, but our
//!     wrapper stays in doubles to avoid a flurry of casts at the Lingo edge.
//!   - Indices into Pages / TriIndices are `u32`, matching PhysX's encoding.

/// Source: GuRTree.h:271-289. Packed 32-bit encoding of a leaf:
///   bit 0   : leaf flag (always 1 for leaf entries)
///   bits 1-4: (NbTriangles - 1)  ⇒ supports 1..16 triangles per leaf
///   bits 5-31: TriangleIndex (offset into the mesh's sorted triangle table)
#[derive(Clone, Copy, Debug)]
pub struct LeafTriangles {
    pub data: u32,
}

impl LeafTriangles {
    #[inline] pub fn nb_triangles(self) -> u32 { ((self.data >> 1) & 15) + 1 }
    #[inline] pub fn triangle_index(self) -> u32 { self.data >> 5 }
    pub fn encode(nb: u32, index: u32) -> u32 {
        debug_assert!((1..=16).contains(&nb));
        debug_assert!(index < (1u32 << 27));
        (index << 5) | (((nb - 1) & 15) << 1) | 1
    }
}

/// Source: GuRTree.h:68-78 — per-node temp/build struct.
#[derive(Clone, Copy, Debug, Default)]
pub struct RTreeNodeQ {
    pub minx: f32, pub miny: f32, pub minz: f32,
    pub maxx: f32, pub maxy: f32, pub maxz: f32,
    pub ptr: u32,
}

impl RTreeNodeQ {
    pub fn set_empty(&mut self) {
        self.minx = RTreePage::MX; self.miny = RTreePage::MX; self.minz = RTreePage::MX;
        self.maxx = RTreePage::MN; self.maxy = RTreePage::MN; self.maxz = RTreePage::MN;
    }
    pub fn grow(&mut self, other: &RTreeNodeQ) {
        self.minx = self.minx.min(other.minx);
        self.miny = self.miny.min(other.miny);
        self.minz = self.minz.min(other.minz);
        self.maxx = self.maxx.max(other.maxx);
        self.maxy = self.maxy.max(other.maxy);
        self.maxz = self.maxz.max(other.maxz);
    }
}

/// Source: GuRTree.h:84-116. SoA 4-children page.
#[derive(Clone, Debug)]
pub struct RTreePage {
    pub minx: [f32; 4], pub miny: [f32; 4], pub minz: [f32; 4],
    pub maxx: [f32; 4], pub maxy: [f32; 4], pub maxz: [f32; 4],
    pub ptrs: [u32; 4],
}

impl RTreePage {
    pub const N: usize = 4;
    pub const MN: f32 = -f32::MAX;
    pub const MX: f32 = f32::MAX;

    pub fn new_empty() -> Self {
        let mut p = Self {
            minx: [Self::MX; 4], miny: [Self::MX; 4], minz: [Self::MX; 4],
            maxx: [Self::MN; 4], maxy: [Self::MN; 4], maxz: [Self::MN; 4],
            ptrs: [0; 4],
        };
        p.set_empty(0);
        p
    }

    pub fn is_empty(&self, i: usize) -> bool { self.minx[i] > self.maxx[i] }
    pub fn is_leaf(&self, i: usize) -> bool { (self.ptrs[i] & 1) != 0 }

    pub fn clear_node(&mut self, i: usize) {
        self.minx[i] = Self::MX; self.miny[i] = Self::MX; self.minz[i] = Self::MX;
        self.maxx[i] = Self::MN; self.maxy[i] = Self::MN; self.maxz[i] = Self::MN;
        self.ptrs[i] = 0;
    }

    pub fn set_empty(&mut self, start: usize) {
        for i in start..Self::N { self.clear_node(i); }
    }

    pub fn set_node(&mut self, i: usize, n: &RTreeNodeQ) {
        self.minx[i] = n.minx; self.miny[i] = n.miny; self.minz[i] = n.minz;
        self.maxx[i] = n.maxx; self.maxy[i] = n.maxy; self.maxz[i] = n.maxz;
        self.ptrs[i] = n.ptr;
    }

    pub fn compute_bounds(&self) -> RTreeNodeQ {
        let (mut mnx, mut mny, mut mnz) = (Self::MX, Self::MX, Self::MX);
        let (mut mxx, mut mxy, mut mxz) = (Self::MN, Self::MN, Self::MN);
        for j in 0..Self::N {
            if self.is_empty(j) { continue; }
            mnx = mnx.min(self.minx[j]); mny = mny.min(self.miny[j]); mnz = mnz.min(self.minz[j]);
            mxx = mxx.max(self.maxx[j]); mxy = mxy.max(self.maxy[j]); mxz = mxz.max(self.maxz[j]);
        }
        RTreeNodeQ { minx: mnx, miny: mny, minz: mnz, maxx: mxx, maxy: mxy, maxz: mxz, ptr: 0 }
    }
}

/// Source: GuRTree.h:120-227. Root: world AABB, the page array, and the level / counts.
#[derive(Debug, Clone)]
pub struct RTree {
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub num_root_pages: u32,
    pub num_levels: u32,
    pub total_nodes: u32,
    pub pages: Vec<RTreePage>,
    /// Permutation of input triangle IDs in leaf order.
    pub tri_indices: Vec<u32>,
}

impl Default for RTree {
    fn default() -> Self {
        Self {
            bounds_min: [0.0; 3], bounds_max: [0.0; 3],
            num_root_pages: 0, num_levels: 0, total_nodes: 0,
            pages: Vec::new(), tri_indices: Vec::new(),
        }
    }
}

/// Callbacks — trait objects, mirrors C# RTree.ICallback / ICallbackRaycast.
pub trait RTreeAabbCallback {
    /// Called with leaf-encoded ptr (low bit stripped). Return false to abort traversal.
    fn process(&mut self, leaf_encoded: u32) -> bool;
}

pub trait RTreeRaycastCallback {
    /// Called with leaf-encoded ptr (low bit stripped). `max_t` is in/out;
    /// callback may shrink it to early-clip subsequent traversal.
    fn process(&mut self, leaf_encoded: u32, max_t: &mut f32) -> bool;
}

impl RTree {
    /// Source: GuRTreeQueries.cpp:84-171 — `traverseAABB` (scalar).
    pub fn traverse_aabb<C: RTreeAabbCallback + ?Sized>(&self, box_min: [f32; 3], box_max: [f32; 3], cb: &mut C) {
        if self.pages.is_empty() || self.num_root_pages == 0 { return; }
        let mut stack = Vec::with_capacity(128);
        for j in (0..self.num_root_pages).rev() {
            stack.push(j);
        }
        while let Some(page_idx) = stack.pop() {
            let page = &self.pages[page_idx as usize];
            for i in 0..RTreePage::N {
                if page.is_empty(i) { continue; }
                if box_min[0] > page.maxx[i] || box_max[0] < page.minx[i]
                || box_min[1] > page.maxy[i] || box_max[1] < page.miny[i]
                || box_min[2] > page.maxz[i] || box_max[2] < page.minz[i] {
                    continue;
                }
                if page.is_leaf(i) {
                    let leaf = page.ptrs[i] & !1u32;
                    if !cb.process(leaf) { return; }
                } else {
                    stack.push(page.ptrs[i] >> 1);
                }
            }
        }
    }

    /// Source: GuRTreeQueries.cpp:174-320 — `traverseRay<inflate>` (scalar).
    /// Kay-Kajiya slab raycast. `aabb_inflate` adds a margin for thick rays.
    pub fn traverse_ray<C: RTreeRaycastCallback + ?Sized>(
        &self,
        ray_origin: [f32; 3],
        ray_dir: [f32; 3],
        mut max_t: f32,
        cb: &mut C,
        aabb_inflate: [f32; 3],
    ) {
        if self.pages.is_empty() || self.num_root_pages == 0 { return; }
        const EPS_FLOAT: f32 = 1e-6;
        let ix = safe_inv_dir(ray_dir[0], EPS_FLOAT);
        let iy = safe_inv_dir(ray_dir[1], EPS_FLOAT);
        let iz = safe_inv_dir(ray_dir[2], EPS_FLOAT);

        let mut stack = Vec::with_capacity(128);
        for j in (0..self.num_root_pages).rev() {
            stack.push(j << 1); // leaf bit clear
        }
        while let Some(top) = stack.pop() {
            if (top & 1) != 0 {
                let leaf_encoded = top - 1;
                let mut new_max_t = max_t;
                if !cb.process(leaf_encoded, &mut new_max_t) { return; }
                if new_max_t < max_t { max_t = new_max_t; }
                continue;
            }
            let page_idx = top >> 1;
            let page = &self.pages[page_idx as usize];
            for i in 0..RTreePage::N {
                if page.is_empty(i) { continue; }
                let minx = page.minx[i] - aabb_inflate[0];
                let miny = page.miny[i] - aabb_inflate[1];
                let minz = page.minz[i] - aabb_inflate[2];
                let maxx = page.maxx[i] + aabb_inflate[0];
                let maxy = page.maxy[i] + aabb_inflate[1];
                let maxz = page.maxz[i] + aabb_inflate[2];

                let tx0 = (minx - ray_origin[0]) * ix;
                let ty0 = (miny - ray_origin[1]) * iy;
                let tz0 = (minz - ray_origin[2]) * iz;
                let tx1 = (maxx - ray_origin[0]) * ix;
                let ty1 = (maxy - ray_origin[1]) * iy;
                let tz1 = (maxz - ray_origin[2]) * iz;

                let tnear = tx0.min(tx1).max(ty0.min(ty1)).max(tz0.min(tz1));
                let tfar  = tx0.max(tx1).min(ty0.max(ty1)).min(tz0.max(tz1));

                if tfar < EPS_FLOAT || tnear > max_t || tnear > tfar { continue; }

                stack.push(page.ptrs[i]); // includes leaf bit verbatim
            }
        }
    }
}

#[inline]
fn safe_inv_dir(d: f32, eps: f32) -> f32 {
    let mut a = d.abs();
    if a < eps { a = eps; }
    let signed = if d < 0.0 { -a } else { a };
    1.0 / signed
}

/// Runtime top-down 4-ary RTree builder.
/// Source: parallel to `RTreeBuilder.Build` in the C# reference.
pub struct RTreeBuilder;

impl RTreeBuilder {
    pub const MAX_LEAF_TRIS: usize = 16;
    pub const N: usize = RTreePage::N;

    /// Build an RTree over `triangles[i] = (a, b, c)` (indices into vertices).
    pub fn build(triangles: &[u32], vertices: &[[f32; 3]]) -> RTree {
        let tri_count = triangles.len() / 3;
        let mut tree = RTree::default();
        if tri_count == 0 { return tree; }

        let mut tri_aabbs = vec![RTreeNodeQ::default(); tri_count];
        let mut tri_cx = vec![0f32; tri_count];
        let mut tri_cy = vec![0f32; tri_count];
        let mut tri_cz = vec![0f32; tri_count];
        let mut work: Vec<u32> = (0..tri_count as u32).collect();

        for t in 0..tri_count {
            let a = vertices[triangles[t * 3] as usize];
            let b = vertices[triangles[t * 3 + 1] as usize];
            let c = vertices[triangles[t * 3 + 2] as usize];
            let bnd = RTreeNodeQ {
                minx: a[0].min(b[0]).min(c[0]),
                miny: a[1].min(b[1]).min(c[1]),
                minz: a[2].min(b[2]).min(c[2]),
                maxx: a[0].max(b[0]).max(c[0]),
                maxy: a[1].max(b[1]).max(c[1]),
                maxz: a[2].max(b[2]).max(c[2]),
                ptr: 0,
            };
            tri_aabbs[t] = bnd;
            tri_cx[t] = (bnd.minx + bnd.maxx) * 0.5;
            tri_cy[t] = (bnd.miny + bnd.maxy) * 0.5;
            tri_cz[t] = (bnd.minz + bnd.maxz) * 0.5;
        }

        let mut pages: Vec<RTreePage> = vec![RTreePage::new_empty()];
        let mut max_levels: u32 = 0;

        let mut root_child_nodes = [RTreeNodeQ::default(); 4];
        for k in 0..4 { root_child_nodes[k].set_empty(); }

        let root_parts = tri_count.min(Self::N);
        let root_base = tri_count / root_parts;
        let root_rem = tri_count - root_base * root_parts;
        let mut cursor = 0;
        for k in 0..root_parts {
            let part_count = root_base + if k < root_rem { 1 } else { 0 };
            if part_count == 0 { break; }
            Self::recurse(
                &mut pages, &mut max_levels,
                &tri_aabbs, &tri_cx, &tri_cy, &tri_cz, &mut work,
                cursor, part_count, 1, &mut root_child_nodes, k,
            );
            cursor += part_count;
        }
        for k in 0..4 {
            pages[0].set_node(k, &root_child_nodes[k]);
        }

        let root_bounds = pages[0].compute_bounds();
        tree.bounds_min = [root_bounds.minx, root_bounds.miny, root_bounds.minz];
        tree.bounds_max = [root_bounds.maxx, root_bounds.maxy, root_bounds.maxz];
        tree.num_root_pages = 1;
        tree.num_levels = max_levels + 1;
        tree.total_nodes = tri_count as u32;
        tree.pages = pages;
        tree.tri_indices = work;
        tree
    }

    fn recurse(
        pages: &mut Vec<RTreePage>,
        max_levels: &mut u32,
        tri_aabbs: &[RTreeNodeQ],
        tri_cx: &[f32], tri_cy: &[f32], tri_cz: &[f32],
        work: &mut [u32],
        span_start: usize, span_count: usize,
        level: u32,
        aabbs_out: &mut [RTreeNodeQ; 4],
        out_index: usize,
    ) {
        if level > *max_levels { *max_levels = level; }

        // Bounds of the span.
        let mut bnd = RTreeNodeQ::default(); bnd.set_empty();
        for i in 0..span_count {
            bnd.grow(&tri_aabbs[work[span_start + i] as usize]);
        }
        aabbs_out[out_index] = bnd;

        if span_count <= Self::MAX_LEAF_TRIS {
            aabbs_out[out_index].ptr = LeafTriangles::encode(span_count as u32, span_start as u32);
            return;
        }

        // Pick split axis = longest centroid extent.
        let axis = {
            let (mut mnx, mut mny, mut mnz) = (f32::MAX, f32::MAX, f32::MAX);
            let (mut mxx, mut mxy, mut mxz) = (f32::MIN, f32::MIN, f32::MIN);
            for i in 0..span_count {
                let idx = work[span_start + i] as usize;
                if tri_cx[idx] < mnx { mnx = tri_cx[idx]; } if tri_cx[idx] > mxx { mxx = tri_cx[idx]; }
                if tri_cy[idx] < mny { mny = tri_cy[idx]; } if tri_cy[idx] > mxy { mxy = tri_cy[idx]; }
                if tri_cz[idx] < mnz { mnz = tri_cz[idx]; } if tri_cz[idx] > mxz { mxz = tri_cz[idx]; }
            }
            let ex = mxx - mnx; let ey = mxy - mny; let ez = mxz - mnz;
            if ex >= ey && ex >= ez { 0 } else if ey >= ez { 1 } else { 2 }
        };
        let cents: &[f32] = match axis { 0 => tri_cx, 1 => tri_cy, _ => tri_cz };

        // Sort the span by centroid on that axis (extract key, sort, write back).
        let mut sub: Vec<(f32, u32)> = (0..span_count)
            .map(|i| (cents[work[span_start + i] as usize], work[span_start + i]))
            .collect();
        sub.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        for i in 0..span_count {
            work[span_start + i] = sub[i].1;
        }

        let n_parts = Self::N.min(span_count);
        let base_part = span_count / n_parts;
        let remainder = span_count - base_part * n_parts;

        let new_page_idx = pages.len();
        pages.push(RTreePage::new_empty());

        let mut child_nodes = [RTreeNodeQ::default(); 4];
        for k in 0..4 { child_nodes[k].set_empty(); }

        let mut cursor = span_start;
        for k in 0..n_parts {
            let part_count = base_part + if k < remainder { 1 } else { 0 };
            if part_count == 0 { break; }
            Self::recurse(
                pages, max_levels,
                tri_aabbs, tri_cx, tri_cy, tri_cz, work,
                cursor, part_count, level + 1, &mut child_nodes, k,
            );
            cursor += part_count;
        }

        for k in 0..4 {
            pages[new_page_idx].set_node(k, &child_nodes[k]);
        }
        aabbs_out[out_index].ptr = (new_page_idx as u32) << 1; // inner: bit 0 clear
    }
}
