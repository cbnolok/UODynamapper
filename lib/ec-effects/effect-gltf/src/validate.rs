//! A structural check of a `.glb` file (the Khronos validator is not a dependency): GLB header and chunks,
//! JSON, object references, buffer view and accessor bounds, POSITION and animation input bounds, index ranges,
//! animation inputs strictly increasing and outputs of the right size, a node hierarchy that is a forest,
//! embedded PNG signatures, and unlit materials without the lit-only members the `KHR_materials_unlit`
//! implementation note leaves out (emissive, normal, occlusion and metallic-roughness textures, a non-zero
//! `emissiveFactor`), neither stored nor animated (Blender's importer fails on an animated transform of such a
//! texture: see [`crate::material`]).

use serde_json::Value;

use crate::glb::{
    CHUNK_BIN, CHUNK_HEADER_BYTES, CHUNK_JSON, COMPONENT_FLOAT, COMPONENT_UNSIGNED_INT, COMPONENT_UNSIGNED_SHORT,
    GLB_ALIGNMENT, GLB_HEADER_BYTES, GLB_MAGIC, GLB_VERSION,
};

/// `componentType` of unsigned bytes.
const COMPONENT_UNSIGNED_BYTE: u64 = 5121;
/// A node rotation farther than this from unit length is an error.
const QUATERNION_LENGTH_TOLERANCE: f64 = 1e-3;
/// Outputs per key of a CUBICSPLINE sampler (in tangent, value, out tangent).
const CUBIC_SPLINE_OUTPUTS: usize = 3;
/// Bytes of a 32-bit value.
const WORD_BYTES: usize = 4;

/// The PNG file signature.
const PNG_SIGNATURE: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
/// `componentType` byte sizes.
const COMPONENT_SIZES: [(u64, usize); 6] = [(5120, 1), (5121, 1), (5122, 2), (5123, 2), (5125, 4), (5126, 4)];
/// Accessor element sizes.
const TYPE_COMPONENTS: [(&str, usize); 7] =
    [("SCALAR", 1), ("VEC2", 2), ("VEC3", 3), ("VEC4", 4), ("MAT2", 4), ("MAT3", 9), ("MAT4", 16)];
/// Morph weights are animated as scalars.
const WEIGHTS_PATH: &str = "weights";
/// Members of an unlit material that only a lit fallback uses, as JSON pointers below the material.
const UNLIT_FALLBACK_ONLY: [&str; 5] = [
    "/emissiveTexture",
    "/normalTexture",
    "/occlusionTexture",
    "/pbrMetallicRoughness/metallicRoughnessTexture",
    "/emissiveFactor",
];
/// The default (and, for an unlit material, the only valid) `emissiveFactor`.
const ZERO_EMISSIVE: [f64; 3] = [0.0; 3];

/// Counts of a valid file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub nodes: usize,
    pub meshes: usize,
    pub materials: usize,
    pub images: usize,
    pub animations: usize,
    pub channels: usize,
    pub bytes: usize,
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset + WORD_BYTES)?.try_into().ok()?))
}

fn array<'a>(document: &'a Value, key: &str) -> &'a [Value] {
    document.get(key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn index(value: Option<&Value>) -> Option<usize> {
    value.and_then(Value::as_u64).map(|value| value as usize)
}

fn is_unlit(material: &Value) -> bool {
    material.pointer(&format!("/extensions/{}", crate::material::EXT_MATERIALS_UNLIT)).is_some()
}

/// The lit-only member of an unlit material that `pointer` (a document JSON pointer) targets.
fn unlit_fallback_target(document: &Value, pointer: &str) -> Option<(usize, &'static str)> {
    let rest = pointer.strip_prefix("/materials/")?;
    let (material, member) = rest.split_once('/')?;
    let material: usize = material.parse().ok()?;
    if !array(document, "materials").get(material).is_some_and(is_unlit) {
        return None;
    }
    let member = format!("/{member}");
    UNLIT_FALLBACK_ONLY
        .into_iter()
        .find(|lit| member == *lit || member.starts_with(&format!("{lit}/")))
        .map(|lit| (material, lit))
}

struct Checker<'a> {
    document: &'a Value,
    bin: &'a [u8],
    errors: Vec<String>,
}

impl Checker<'_> {
    fn reference(&mut self, what: &str, value: Option<&Value>, collection: &str) -> Option<usize> {
        let count = array(self.document, collection).len();
        match index(value) {
            Some(index) if index < count => Some(index),
            Some(index) => {
                self.errors.push(format!("{what}: {collection} {index} of {count}"));
                None
            }
            None => {
                self.errors.push(format!("{what}: missing {collection} reference"));
                None
            }
        }
    }

    fn optional_reference(&mut self, what: &str, value: Option<&Value>, collection: &str) -> Option<usize> {
        value.and_then(|value| self.reference(what, Some(value), collection))
    }

    /// The bytes of buffer view `view`.
    fn view_bytes(&self, view: usize) -> Option<&[u8]> {
        let view = &array(self.document, "bufferViews")[view];
        let offset = index(view.get("byteOffset")).unwrap_or(0);
        let length = index(view.get("byteLength"))?;
        self.bin.get(offset..offset + length)
    }

    /// The floats of a float accessor.
    fn floats(&self, accessor: usize) -> Option<Vec<f32>> {
        let accessor = &array(self.document, "accessors")[accessor];
        if accessor.get("componentType").and_then(Value::as_u64) != Some(u64::from(COMPONENT_FLOAT)) {
            return None;
        }
        let bytes = self.view_bytes(index(accessor.get("bufferView"))?)?;
        Some(bytes.chunks_exact(WORD_BYTES).map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap())).collect())
    }

    fn accessor_count(&self, accessor: usize) -> usize {
        index(array(self.document, "accessors")[accessor].get("count")).unwrap_or(0)
    }

    fn check_buffers(&mut self) {
        let buffers = array(self.document, "buffers");
        if buffers.len() > 1 {
            self.errors.push(format!("{} buffers; a GLB has one", buffers.len()));
        }
        if let Some(buffer) = buffers.first() {
            let length = index(buffer.get("byteLength")).unwrap_or(0);
            if length > self.bin.len() {
                self.errors.push(format!("buffer of {length} bytes, BIN chunk of {}", self.bin.len()));
            }
        }
        for (view_index, view) in array(self.document, "bufferViews").iter().enumerate() {
            let what = format!("bufferView {view_index}");
            self.reference(&what, view.get("buffer"), "buffers");
            let offset = index(view.get("byteOffset")).unwrap_or(0);
            let length = index(view.get("byteLength")).unwrap_or(0);
            if offset + length > self.bin.len() {
                self.errors.push(format!("{what}: bytes {offset}..{} past the buffer", offset + length));
            }
        }
    }

    fn check_accessors(&mut self) {
        let accessors = array(self.document, "accessors").to_vec();
        for (accessor_index, accessor) in accessors.iter().enumerate() {
            let what = format!("accessor {accessor_index}");
            let Some(view) = self.reference(&what, accessor.get("bufferView"), "bufferViews") else { continue };
            let component = accessor.get("componentType").and_then(Value::as_u64);
            let Some(&(_, component_size)) = COMPONENT_SIZES.iter().find(|(id, _)| Some(*id) == component) else {
                self.errors.push(format!("{what}: componentType {component:?}"));
                continue;
            };
            let type_name = accessor.get("type").and_then(Value::as_str).unwrap_or("");
            let Some(&(_, components)) = TYPE_COMPONENTS.iter().find(|(name, _)| *name == type_name) else {
                self.errors.push(format!("{what}: type {type_name:?}"));
                continue;
            };
            let count = index(accessor.get("count")).unwrap_or(0);
            if count == 0 {
                self.errors.push(format!("{what}: count 0"));
            }
            let offset = index(accessor.get("byteOffset")).unwrap_or(0);
            let view_length = index(array(self.document, "bufferViews")[view].get("byteLength")).unwrap_or(0);
            if offset + count * components * component_size > view_length {
                self.errors.push(format!("{what}: {count} {type_name} past its view of {view_length} bytes"));
            }
            let view_offset = index(array(self.document, "bufferViews")[view].get("byteOffset")).unwrap_or(0);
            if (view_offset + offset) % component_size != 0 {
                self.errors.push(format!("{what}: misaligned"));
            }
            if let (Some(min), Some(max)) = (accessor.get("min"), accessor.get("max")) {
                let (Some(min), Some(max)) = (min.as_array(), max.as_array()) else {
                    self.errors.push(format!("{what}: min / max are not arrays"));
                    continue;
                };
                if min.len() != components || max.len() != components {
                    self.errors.push(format!("{what}: min / max of the wrong size"));
                }
                if let Some(values) = self.floats(accessor_index) {
                    for (element, chunk) in values.chunks_exact(components).enumerate() {
                        for (k, value) in chunk.iter().enumerate() {
                            let low = min.get(k).and_then(Value::as_f64).unwrap_or(f64::NEG_INFINITY) as f32;
                            let high = max.get(k).and_then(Value::as_f64).unwrap_or(f64::INFINITY) as f32;
                            if !(*value >= low && *value <= high) {
                                self.errors.push(format!("{what}: element {element} outside min / max"));
                            }
                        }
                    }
                }
            }
            if let Some(values) = self.floats(accessor_index) {
                if values.iter().any(|value| !value.is_finite()) {
                    self.errors.push(format!("{what}: non-finite values"));
                }
            }
        }
    }

    fn indices(&self, accessor: usize) -> Option<Vec<u32>> {
        let accessor = &array(self.document, "accessors")[accessor];
        let bytes = self.view_bytes(index(accessor.get("bufferView"))?)?;
        let count = index(accessor.get("count"))?;
        match accessor.get("componentType").and_then(Value::as_u64)? {
            COMPONENT_UNSIGNED_BYTE => Some(bytes.iter().take(count).map(|&value| u32::from(value)).collect()),
            id if id == u64::from(COMPONENT_UNSIGNED_SHORT) => {
                Some(bytes.chunks_exact(2).take(count).map(|c| u32::from(u16::from_le_bytes([c[0], c[1]]))).collect())
            }
            id if id == u64::from(COMPONENT_UNSIGNED_INT) => {
                Some(bytes.chunks_exact(WORD_BYTES).take(count).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect())
            }
            _ => None,
        }
    }

    fn check_meshes(&mut self) {
        let meshes = array(self.document, "meshes").to_vec();
        for (mesh_index, mesh) in meshes.iter().enumerate() {
            let primitives = array(mesh, "primitives");
            if primitives.is_empty() {
                self.errors.push(format!("mesh {mesh_index}: no primitives"));
            }
            for (primitive_index, primitive) in primitives.iter().enumerate() {
                let what = format!("mesh {mesh_index} primitive {primitive_index}");
                let Some(attributes) = primitive.get("attributes").and_then(Value::as_object) else {
                    self.errors.push(format!("{what}: no attributes"));
                    continue;
                };
                let Some(position) = self.reference(&what, attributes.get("POSITION"), "accessors") else { continue };
                let accessor = &array(self.document, "accessors")[position];
                if accessor.get("min").is_none() || accessor.get("max").is_none() {
                    self.errors.push(format!("{what}: POSITION without min / max"));
                }
                let vertices = self.accessor_count(position);
                for (name, value) in attributes {
                    if let Some(accessor) = self.reference(&format!("{what} {name}"), Some(value), "accessors") {
                        if self.accessor_count(accessor) != vertices {
                            self.errors.push(format!("{what}: {name} count differs from POSITION"));
                        }
                    }
                }
                if let Some(indices) = self.optional_reference(&what, primitive.get("indices"), "accessors") {
                    match self.indices(indices) {
                        Some(values) => {
                            if values.iter().any(|&value| value as usize >= vertices) {
                                self.errors.push(format!("{what}: index past {vertices} vertices"));
                            }
                            if values.len() % 3 != 0 {
                                self.errors.push(format!("{what}: {} indices for triangles", values.len()));
                            }
                        }
                        None => self.errors.push(format!("{what}: unreadable indices")),
                    }
                }
                self.optional_reference(&what, primitive.get("material"), "materials");
                let targets = array(primitive, "targets");
                for target in targets {
                    if let Some(position) = self.optional_reference(&what, target.get("POSITION"), "accessors") {
                        let accessor = &array(self.document, "accessors")[position];
                        if accessor.get("min").is_none() || accessor.get("max").is_none() {
                            self.errors.push(format!("{what}: target POSITION without min / max"));
                        }
                        if self.accessor_count(position) != vertices {
                            self.errors.push(format!("{what}: target count differs from POSITION"));
                        }
                    }
                }
                if let Some(weights) = mesh.get("weights").and_then(Value::as_array) {
                    if weights.len() != targets.len() {
                        self.errors.push(format!("{what}: {} weights for {} targets", weights.len(), targets.len()));
                    }
                }
            }
        }
    }

    fn check_textures(&mut self) {
        for (image_index, image) in array(self.document, "images").to_vec().iter().enumerate() {
            let what = format!("image {image_index}");
            if let Some(view) = self.reference(&what, image.get("bufferView"), "bufferViews") {
                if !self.view_bytes(view).is_some_and(|bytes| bytes.starts_with(PNG_SIGNATURE)) {
                    self.errors.push(format!("{what}: not a PNG"));
                }
            }
        }
        for (texture_index, texture) in array(self.document, "textures").to_vec().iter().enumerate() {
            let what = format!("texture {texture_index}");
            self.reference(&what, texture.get("source"), "images");
            self.optional_reference(&what, texture.get("sampler"), "samplers");
        }
        for (material_index, material) in array(self.document, "materials").to_vec().iter().enumerate() {
            let what = format!("material {material_index}");
            let pbr = material.get("pbrMetallicRoughness");
            for info in [pbr.and_then(|pbr| pbr.get("baseColorTexture")), material.get("emissiveTexture")].into_iter().flatten() {
                self.reference(&what, info.get("index"), "textures");
            }
            if is_unlit(material) {
                for member in UNLIT_FALLBACK_ONLY {
                    let Some(value) = material.pointer(member) else { continue };
                    let zero = value.as_array().is_some_and(|factor| {
                        factor.len() == ZERO_EMISSIVE.len()
                            && factor.iter().zip(ZERO_EMISSIVE).all(|(value, zero)| value.as_f64() == Some(zero))
                    });
                    if !zero {
                        self.errors.push(format!("{what}: unlit with {member}"));
                    }
                }
            }
        }
    }

    fn check_nodes(&mut self) {
        let nodes = array(self.document, "nodes").to_vec();
        let mut parents = vec![None; nodes.len()];
        for (node_index, node) in nodes.iter().enumerate() {
            let what = format!("node {node_index}");
            for child in array(node, "children") {
                if let Some(child) = self.reference(&what, Some(child), "nodes") {
                    if parents[child].is_some() {
                        self.errors.push(format!("node {child}: more than one parent"));
                    }
                    parents[child] = Some(node_index);
                }
            }
            if let Some(mesh) = self.optional_reference(&what, node.get("mesh"), "meshes") {
                let targets = array(&array(self.document, "meshes")[mesh], "primitives")
                    .first()
                    .map_or(0, |primitive| array(primitive, "targets").len());
                if let Some(weights) = node.get("weights").and_then(Value::as_array) {
                    if weights.len() != targets {
                        self.errors.push(format!("{what}: {} weights for {targets} targets", weights.len()));
                    }
                }
            }
            if let Some(rotation) = node.get("rotation").and_then(Value::as_array) {
                let length: f64 = rotation.iter().filter_map(Value::as_f64).map(|v| v * v).sum::<f64>().sqrt();
                if rotation.len() != 4 || (length - 1.0).abs() > QUATERNION_LENGTH_TOLERANCE {
                    self.errors.push(format!("{what}: rotation is not a unit quaternion"));
                }
            }
        }
        for start in 0..nodes.len() {
            let mut current = start;
            let mut steps = 0;
            while let Some(parent) = parents[current] {
                current = parent;
                steps += 1;
                if steps > nodes.len() {
                    self.errors.push(format!("node {start}: cycle in the hierarchy"));
                    break;
                }
            }
        }
        for (scene_index, scene) in array(self.document, "scenes").to_vec().iter().enumerate() {
            for node in array(scene, "nodes") {
                if let Some(node) = self.reference(&format!("scene {scene_index}"), Some(node), "nodes") {
                    if parents[node].is_some() {
                        self.errors.push(format!("scene {scene_index}: root node {node} has a parent"));
                    }
                }
            }
        }
    }

    fn check_animations(&mut self) -> usize {
        let mut channels_total = 0;
        for (animation_index, animation) in array(self.document, "animations").to_vec().iter().enumerate() {
            let samplers = array(animation, "samplers").to_vec();
            let mut sampler_components = Vec::new();
            for (sampler_index, sampler) in samplers.iter().enumerate() {
                let what = format!("animation {animation_index} sampler {sampler_index}");
                let input = self.reference(&what, sampler.get("input"), "accessors");
                let output = self.reference(&what, sampler.get("output"), "accessors");
                let (Some(input), Some(output)) = (input, output) else {
                    sampler_components.push(None);
                    continue;
                };
                let accessor = &array(self.document, "accessors")[input];
                if accessor.get("min").is_none() || accessor.get("max").is_none() {
                    self.errors.push(format!("{what}: input without min / max"));
                }
                match self.floats(input) {
                    Some(times) => {
                        if times.windows(2).any(|pair| !(pair[1] > pair[0])) {
                            self.errors.push(format!("{what}: input not strictly increasing"));
                        }
                        if times.first().is_some_and(|&time| time < 0.0) {
                            self.errors.push(format!("{what}: negative time"));
                        }
                    }
                    None => self.errors.push(format!("{what}: input is not float")),
                }
                let input_count = self.accessor_count(input);
                let output_count = self.accessor_count(output);
                let interpolation = sampler.get("interpolation").and_then(Value::as_str).unwrap_or("LINEAR");
                let per_key = if interpolation == "CUBICSPLINE" { CUBIC_SPLINE_OUTPUTS } else { 1 };
                if input_count == 0 || output_count % (input_count * per_key) != 0 {
                    self.errors.push(format!("{what}: {output_count} outputs for {input_count} inputs"));
                    sampler_components.push(None);
                } else {
                    sampler_components.push(Some(output_count / (input_count * per_key)));
                }
            }
            for (channel_index, channel) in array(animation, "channels").to_vec().iter().enumerate() {
                channels_total += 1;
                let what = format!("animation {animation_index} channel {channel_index}");
                let sampler = index(channel.get("sampler"));
                let Some(sampler) = sampler.filter(|&sampler| sampler < samplers.len()) else {
                    self.errors.push(format!("{what}: bad sampler"));
                    continue;
                };
                let Some(target) = channel.get("target") else {
                    self.errors.push(format!("{what}: no target"));
                    continue;
                };
                let path = target.get("path").and_then(Value::as_str).unwrap_or("");
                if path == "pointer" {
                    let pointer = target
                        .pointer("/extensions/KHR_animation_pointer/pointer")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if self.document.pointer(pointer).is_none() {
                        self.errors.push(format!("{what}: pointer {pointer:?} does not resolve"));
                    }
                    if let Some((material, member)) = unlit_fallback_target(self.document, pointer) {
                        self.errors.push(format!("{what}: animates {member} of unlit material {material}"));
                    }
                    continue;
                }
                let Some(node) = self.reference(&what, target.get("node"), "nodes") else { continue };
                if path == WEIGHTS_PATH {
                    let node_json = &array(self.document, "nodes")[node];
                    let targets = index(node_json.get("mesh"))
                        .and_then(|mesh| array(self.document, "meshes").get(mesh))
                        .and_then(|mesh| array(mesh, "primitives").first())
                        .map_or(0, |primitive| array(primitive, "targets").len());
                    if sampler_components[sampler] != Some(targets) {
                        self.errors.push(format!("{what}: weights for {targets} targets"));
                    }
                } else if !matches!(path, "translation" | "rotation" | "scale") {
                    self.errors.push(format!("{what}: path {path:?}"));
                }
            }
        }
        channels_total
    }
}

/// Checks a `.glb` file; returns its summary or every problem found.
pub fn validate_glb(bytes: &[u8]) -> Result<Summary, Vec<String>> {
    let fail = |message: String| Err(vec![message]);
    if bytes.len() < GLB_HEADER_BYTES + CHUNK_HEADER_BYTES {
        return fail(format!("{} bytes: too short", bytes.len()));
    }
    if u32_at(bytes, 0) != Some(GLB_MAGIC) || u32_at(bytes, WORD_BYTES) != Some(GLB_VERSION) {
        return fail("not a glTF 2 binary".to_owned());
    }
    if u32_at(bytes, 2 * WORD_BYTES) != Some(bytes.len() as u32) {
        return fail(format!("header length {:?}, file {}", u32_at(bytes, 8), bytes.len()));
    }
    let json_length = u32_at(bytes, GLB_HEADER_BYTES).unwrap_or(0) as usize;
    if u32_at(bytes, GLB_HEADER_BYTES + WORD_BYTES) != Some(CHUNK_JSON) || json_length % GLB_ALIGNMENT != 0 {
        return fail("first chunk is not an aligned JSON chunk".to_owned());
    }
    let json_start = GLB_HEADER_BYTES + CHUNK_HEADER_BYTES;
    let Some(json_bytes) = bytes.get(json_start..json_start + json_length) else {
        return fail("JSON chunk past the end".to_owned());
    };
    let document: Value = match serde_json::from_slice(json_bytes) {
        Ok(document) => document,
        Err(error) => return fail(format!("JSON: {error}")),
    };
    let bin_header = json_start + json_length;
    let bin: &[u8] = if bin_header < bytes.len() {
        let bin_length = u32_at(bytes, bin_header).unwrap_or(0) as usize;
        if u32_at(bytes, bin_header + WORD_BYTES) != Some(CHUNK_BIN) || bin_length % GLB_ALIGNMENT != 0 {
            return fail("second chunk is not an aligned BIN chunk".to_owned());
        }
        let start = bin_header + CHUNK_HEADER_BYTES;
        match bytes.get(start..start + bin_length) {
            Some(bin) if start + bin_length == bytes.len() => bin,
            _ => return fail("BIN chunk length does not match the file".to_owned()),
        }
    } else {
        &[]
    };
    if document.pointer("/asset/version").and_then(Value::as_str) != Some(crate::glb::GLTF_VERSION) {
        return fail("asset.version is not 2.0".to_owned());
    }
    let mut checker = Checker { document: &document, bin, errors: Vec::new() };
    checker.check_buffers();
    checker.check_accessors();
    checker.check_meshes();
    checker.check_textures();
    checker.check_nodes();
    let channels = checker.check_animations();
    if let Some(scene) = document.get("scene") {
        checker.reference("scene", Some(scene), "scenes");
    }
    if !checker.errors.is_empty() {
        return Err(checker.errors);
    }
    Ok(Summary {
        nodes: array(&document, "nodes").len(),
        meshes: array(&document, "meshes").len(),
        materials: array(&document, "materials").len(),
        images: array(&document, "images").len(),
        animations: array(&document, "animations").len(),
        channels,
        bytes: bytes.len(),
    })
}

/// The JSON document of a `.glb` file.
pub fn glb_json(bytes: &[u8]) -> Option<Value> {
    let json_length = u32_at(bytes, GLB_HEADER_BYTES)? as usize;
    let start = GLB_HEADER_BYTES + CHUNK_HEADER_BYTES;
    serde_json::from_slice(bytes.get(start..start + json_length)?).ok()
}
