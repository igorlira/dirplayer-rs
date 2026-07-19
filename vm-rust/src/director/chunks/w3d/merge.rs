//! Scene merging for Director's `loadFile()`.
//!
//! `member(x).loadFile(fileName {, overwrite, generateUniqueNames})`
//! (Director 11.5 Scripting Dictionary) imports the assets of a W3D file into a
//! 3D cast member. With `overwrite = TRUE` (the default) the file simply
//! replaces the member's assets, which needs no merging. This module implements
//! the `overwrite = FALSE` half: splicing a second W3D file's assets into an
//! existing scene, optionally renaming incoming elements that collide.

use std::collections::{HashMap, HashSet};

use super::types::W3dScene;

/// Pick a name not already present in `taken` (compared case-insensitively).
///
/// The dictionary says only that a colliding element "is renamed"; it doesn't
/// document the pattern. We reuse the `-clone<N>` suffix this codebase already
/// applies to W3D name collisions in `cloneModelFromCastmember`, so both paths
/// read alike.
fn unique_w3d_name(base: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(&base.to_ascii_lowercase()) {
        return base.to_string();
    }
    let mut n = 1;
    loop {
        let candidate = format!("{}-clone{}", base, n);
        if !taken.contains(&candidate.to_ascii_lowercase()) {
            return candidate;
        }
        n += 1;
    }
}

/// Apply a rename map (keyed by lowercased original name) to a reference field.
fn remap(field: &mut String, map: &HashMap<String, String>) {
    if field.is_empty() {
        return;
    }
    if let Some(new_name) = map.get(&field.to_ascii_lowercase()) {
        *field = new_name.clone();
    }
}

/// Plan renames for one namespace, recording only the names that actually move.
fn plan_renames(
    names: Vec<String>,
    taken: &mut HashSet<String>,
    out: &mut HashMap<String, String>,
) {
    for name in names {
        if name.is_empty() {
            continue;
        }
        let key = name.to_ascii_lowercase();
        if out.contains_key(&key) || !taken.contains(&key) {
            // Either already planned (the same name appears twice inside the
            // incoming file) or not colliding at all — leave it alone.
            if !taken.contains(&key) {
                taken.insert(key);
            }
            continue;
        }
        let new_name = unique_w3d_name(&name, taken);
        taken.insert(new_name.to_ascii_lowercase());
        out.insert(key, new_name);
    }
}

/// Replace a same-named entry or append, comparing names case-insensitively.
fn merge_named<T, F: Fn(&T) -> String>(dst: &mut Vec<T>, src: Vec<T>, name_of: F) {
    for item in src {
        let name = name_of(&item).to_ascii_lowercase();
        match dst.iter().position(|e| name_of(e).to_ascii_lowercase() == name) {
            Some(i) => dst[i] = item,
            None => dst.push(item),
        }
    }
}

impl W3dScene {
    /// Merge the assets of `src` into this scene — the `overwrite = FALSE`
    /// behaviour of `loadFile()`.
    ///
    /// `generate_unique_names = true` renames each INCOMING element whose name
    /// collides with one already present, then repoints that file's internal
    /// references at the new names, so neither copy hijacks the other.
    /// `false` lets an incoming element overwrite the same-named existing one,
    /// as the dictionary specifies.
    ///
    /// Names are compared case-insensitively throughout — Director treats W3D
    /// element names that way, and the rest of this module already does.
    pub fn merge_from(&mut self, mut src: W3dScene, generate_unique_names: bool) {
        // Rename maps are per-namespace: Director lets a shader and a model
        // share a name without colliding, so they must not share a map.
        let mut model_res_renames: HashMap<String, String> = HashMap::new();
        let mut shader_renames: HashMap<String, String> = HashMap::new();
        let mut material_renames: HashMap<String, String> = HashMap::new();
        let mut texture_renames: HashMap<String, String> = HashMap::new();
        let mut node_renames: HashMap<String, String> = HashMap::new();

        if generate_unique_names {
            // Model resources and raw meshes share one namespace: a node's
            // resource_name may refer to either.
            let mut taken_model_res: HashSet<String> = self
                .model_resources
                .keys()
                .map(|k| k.to_ascii_lowercase())
                .collect();
            taken_model_res.extend(self.raw_meshes.iter().map(|m| m.name.to_ascii_lowercase()));
            let mut taken_shaders: HashSet<String> =
                self.shaders.iter().map(|s| s.name.to_ascii_lowercase()).collect();
            let mut taken_materials: HashSet<String> =
                self.materials.iter().map(|m| m.name.to_ascii_lowercase()).collect();
            // Likewise texture_images (bytes, keyed by name) and texture_infos
            // (metadata) name the same textures.
            let mut taken_textures: HashSet<String> = self
                .texture_images
                .keys()
                .map(|k| k.to_ascii_lowercase())
                .collect();
            taken_textures.extend(self.texture_infos.iter().map(|t| t.name.to_ascii_lowercase()));
            let mut taken_nodes: HashSet<String> =
                self.nodes.iter().map(|n| n.name.to_ascii_lowercase()).collect();

            let mut res_names: Vec<String> = src.model_resources.keys().cloned().collect();
            res_names.extend(src.raw_meshes.iter().map(|m| m.name.clone()));
            plan_renames(res_names, &mut taken_model_res, &mut model_res_renames);
            plan_renames(
                src.shaders.iter().map(|s| s.name.clone()).collect(),
                &mut taken_shaders,
                &mut shader_renames,
            );
            plan_renames(
                src.materials.iter().map(|m| m.name.clone()).collect(),
                &mut taken_materials,
                &mut material_renames,
            );
            let mut tex_names: Vec<String> = src.texture_images.keys().cloned().collect();
            tex_names.extend(src.texture_infos.iter().map(|t| t.name.clone()));
            plan_renames(tex_names, &mut taken_textures, &mut texture_renames);
            plan_renames(
                src.nodes.iter().map(|n| n.name.clone()).collect(),
                &mut taken_nodes,
                &mut node_renames,
            );

            // Rewrite src's own references so the incoming assets stay
            // internally consistent under their new names.
            for shader in &mut src.shaders {
                remap(&mut shader.name, &shader_renames);
                remap(&mut shader.material_name, &material_renames);
                for layer in &mut shader.texture_layers {
                    remap(&mut layer.name, &texture_renames);
                }
            }
            for mat in &mut src.materials {
                remap(&mut mat.name, &material_renames);
            }
            for tex in &mut src.texture_infos {
                remap(&mut tex.name, &texture_renames);
            }
            for node in &mut src.nodes {
                remap(&mut node.name, &node_renames);
                remap(&mut node.parent_name, &node_renames);
                remap(&mut node.shader_name, &shader_renames);
                remap(&mut node.resource_name, &model_res_renames);
                remap(&mut node.model_resource_name, &model_res_renames);
            }
            for mesh in &mut src.raw_meshes {
                remap(&mut mesh.name, &model_res_renames);
            }
            // Keyed collections have to be rebuilt under the new keys.
            src.model_resources = src
                .model_resources
                .into_iter()
                .map(|(k, mut v)| {
                    remap(&mut v.name, &model_res_renames);
                    for binding in &mut v.shader_bindings {
                        for mesh_binding in &mut binding.mesh_bindings {
                            remap(mesh_binding, &shader_renames);
                        }
                    }
                    let mut key = k;
                    remap(&mut key, &model_res_renames);
                    (key, v)
                })
                .collect();
            src.clod_meshes = src
                .clod_meshes
                .into_iter()
                .map(|(mut k, v)| {
                    remap(&mut k, &model_res_renames);
                    (k, v)
                })
                .collect();
            src.clod_decoders = src
                .clod_decoders
                .into_iter()
                .map(|(mut k, v)| {
                    remap(&mut k, &model_res_renames);
                    (k, v)
                })
                .collect();
            src.texture_images = src
                .texture_images
                .into_iter()
                .map(|(mut k, v)| {
                    remap(&mut k, &texture_renames);
                    (k, v)
                })
                .collect();
        }

        // Splice the assets in. With generateUniqueNames the names are now
        // distinct so these all append/insert; without it, a same-named
        // incoming element replaces the existing one.
        merge_named(&mut self.materials, src.materials, |m| m.name.clone());
        merge_named(&mut self.shaders, src.shaders, |s| s.name.clone());
        merge_named(&mut self.nodes, src.nodes, |n| n.name.clone());
        merge_named(&mut self.lights, src.lights, |l| l.name.clone());
        merge_named(&mut self.texture_infos, src.texture_infos, |t| t.name.clone());
        merge_named(&mut self.skeletons, src.skeletons, |s| s.name.clone());
        merge_named(&mut self.motions, src.motions, |m| m.name.clone());
        merge_named(&mut self.raw_meshes, src.raw_meshes, |m| m.name.clone());
        self.texture_images.extend(src.texture_images);
        self.model_resources.extend(src.model_resources);
        self.clod_meshes.extend(src.clod_meshes);
        self.clod_decoders.extend(src.clod_decoders);

        // Force the renderer to re-upload geometry and textures.
        self.mesh_content_version = self.mesh_content_version.wrapping_add(1);
        self.texture_content_version = self.texture_content_version.wrapping_add(1);
    }
}
