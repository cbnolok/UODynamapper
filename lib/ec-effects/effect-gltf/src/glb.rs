//! A small glTF 2.0 document builder: one binary buffer, buffer views and accessors packed into it, the JSON
//! arrays of the other objects, and the GLB container (`.glb`: a 12-byte header, a JSON chunk and a BIN chunk).

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

/// `glTF` in little-endian: the GLB header magic.
pub const GLB_MAGIC: u32 = 0x4654_6C67;
/// The GLB container version.
pub const GLB_VERSION: u32 = 2;
/// Chunk type of the JSON chunk (`JSON`).
pub const CHUNK_JSON: u32 = 0x4E4F_534A;
/// Chunk type of the binary chunk (`BIN\0`).
pub const CHUNK_BIN: u32 = 0x004E_4942;
/// Size of the GLB header: magic, version, total length.
pub const GLB_HEADER_BYTES: usize = 12;
/// Size of a chunk header: length, type.
pub const CHUNK_HEADER_BYTES: usize = 8;
/// Chunks and buffer views start on 4-byte boundaries.
pub const GLB_ALIGNMENT: usize = 4;
/// The JSON chunk is padded with spaces.
const JSON_PADDING: u8 = b' ';
/// The binary chunk is padded with zeros.
const BIN_PADDING: u8 = 0;
/// The glTF asset version written.
pub const GLTF_VERSION: &str = "2.0";

/// `componentType` of 32-bit floats.
pub const COMPONENT_FLOAT: u32 = 5126;
/// `componentType` of unsigned 16-bit integers.
pub const COMPONENT_UNSIGNED_SHORT: u32 = 5123;
/// `componentType` of unsigned 32-bit integers.
pub const COMPONENT_UNSIGNED_INT: u32 = 5125;
/// Buffer view target of vertex attributes.
pub const TARGET_ARRAY_BUFFER: u32 = 34962;
/// Buffer view target of indices.
pub const TARGET_ELEMENT_ARRAY_BUFFER: u32 = 34963;
/// Primitive mode: triangle list.
pub const MODE_TRIANGLES: u32 = 4;
/// Bytes of a 32-bit float or integer.
const FLOAT_BYTES: usize = 4;
/// Bytes of a 16-bit index.
const SHORT_BYTES: usize = 2;
/// Vertex counts above this need 32-bit indices.
pub const MAX_SHORT_INDEX: usize = u16::MAX as usize;

/// One glTF node, serialised at the end (children are added while the scene is built).
#[derive(Debug, Clone, Default)]
pub struct Node {
    pub name: String,
    pub translation: [f32; 3],
    /// Quaternion x, y, z, w.
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub children: Vec<usize>,
    pub mesh: Option<usize>,
    /// Default morph target weights of the node's mesh.
    pub weights: Option<Vec<f32>>,
    pub extensions: Option<Value>,
    pub extras: Option<Value>,
}

/// Identity rotation, x, y, z, w.
pub const IDENTITY_ROTATION: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

impl Node {
    pub fn named(name: impl Into<String>) -> Self {
        Self { name: name.into(), rotation: IDENTITY_ROTATION, scale: [1.0; 3], ..Default::default() }
    }

    fn to_json(&self) -> Value {
        let mut node = Map::new();
        node.insert("name".into(), json!(self.name));
        if self.translation != [0.0; 3] {
            node.insert("translation".into(), json!(self.translation));
        }
        if self.rotation != IDENTITY_ROTATION {
            node.insert("rotation".into(), json!(self.rotation));
        }
        if self.scale != [1.0; 3] {
            node.insert("scale".into(), json!(self.scale));
        }
        if !self.children.is_empty() {
            node.insert("children".into(), json!(self.children));
        }
        if let Some(mesh) = self.mesh {
            node.insert("mesh".into(), json!(mesh));
        }
        if let Some(weights) = &self.weights {
            node.insert("weights".into(), json!(weights));
        }
        if let Some(extensions) = &self.extensions {
            node.insert("extensions".into(), extensions.clone());
        }
        if let Some(extras) = &self.extras {
            node.insert("extras".into(), extras.clone());
        }
        Value::Object(node)
    }
}

/// A glTF document under construction.
#[derive(Debug, Default)]
pub struct Builder {
    buffer: Vec<u8>,
    pub buffer_views: Vec<Value>,
    pub accessors: Vec<Value>,
    pub nodes: Vec<Node>,
    pub meshes: Vec<Value>,
    pub materials: Vec<Value>,
    pub textures: Vec<Value>,
    pub images: Vec<Value>,
    pub samplers: Vec<Value>,
    pub animations: Vec<Value>,
    /// `KHR_lights_punctual` lights.
    pub lights: Vec<Value>,
    pub extensions_used: BTreeSet<&'static str>,
}

impl Builder {
    pub fn add_node(&mut self, node: Node) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    /// Appends `bytes` as a buffer view (aligned); returns its index.
    pub fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while self.buffer.len() % GLB_ALIGNMENT != 0 {
            self.buffer.push(BIN_PADDING);
        }
        let mut view = json!({ "buffer": 0, "byteOffset": self.buffer.len(), "byteLength": bytes.len() });
        if let Some(target) = target {
            view["target"] = json!(target);
        }
        self.buffer.extend_from_slice(bytes);
        self.buffer_views.push(view);
        self.buffer_views.len() - 1
    }

    /// A float accessor of `N`-component elements (`type_name`: SCALAR, VEC2, ...); `bounds` adds min and max.
    fn float_accessor<const N: usize>(
        &mut self,
        data: &[[f32; N]],
        type_name: &str,
        target: Option<u32>,
        bounds: bool,
    ) -> usize {
        let mut bytes = Vec::with_capacity(data.len() * N * FLOAT_BYTES);
        for element in data {
            for value in element {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        let view = self.view(&bytes, target);
        let mut accessor =
            json!({ "bufferView": view, "componentType": COMPONENT_FLOAT, "count": data.len(), "type": type_name });
        if bounds && !data.is_empty() {
            let mut min = [f32::MAX; N];
            let mut max = [f32::MIN; N];
            for element in data {
                for k in 0..N {
                    min[k] = min[k].min(element[k]);
                    max[k] = max[k].max(element[k]);
                }
            }
            accessor["min"] = json!(min.as_slice());
            accessor["max"] = json!(max.as_slice());
        }
        self.accessors.push(accessor);
        self.accessors.len() - 1
    }

    /// Scalars with min and max (animation inputs need them).
    pub fn scalars(&mut self, data: &[f32]) -> usize {
        let elements: Vec<[f32; 1]> = data.iter().map(|&value| [value]).collect();
        self.float_accessor(&elements, "SCALAR", None, true)
    }

    pub fn vec2(&mut self, data: &[[f32; 2]], target: Option<u32>) -> usize {
        self.float_accessor(data, "VEC2", target, false)
    }

    /// `bounds`: POSITION accessors need min and max.
    pub fn vec3(&mut self, data: &[[f32; 3]], target: Option<u32>, bounds: bool) -> usize {
        self.float_accessor(data, "VEC3", target, bounds)
    }

    pub fn vec4(&mut self, data: &[[f32; 4]], target: Option<u32>) -> usize {
        self.float_accessor(data, "VEC4", target, false)
    }

    /// Floats grouped by `components` into elements of `type_name` (animation outputs).
    pub fn floats(&mut self, data: &[f32], type_name: &str, components: usize) -> usize {
        let mut bytes = Vec::with_capacity(data.len() * FLOAT_BYTES);
        for value in data {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let view = self.view(&bytes, None);
        let count = data.len() / components.max(1);
        self.accessors.push(json!({ "bufferView": view, "componentType": COMPONENT_FLOAT, "count": count, "type": type_name }));
        self.accessors.len() - 1
    }

    /// Triangle indices, 16-bit when every index fits.
    pub fn indices(&mut self, indices: &[u32]) -> usize {
        let wide = indices.iter().any(|&index| index as usize > MAX_SHORT_INDEX);
        let mut bytes = Vec::with_capacity(indices.len() * if wide { FLOAT_BYTES } else { SHORT_BYTES });
        for &index in indices {
            if wide {
                bytes.extend_from_slice(&index.to_le_bytes());
            } else {
                bytes.extend_from_slice(&(index as u16).to_le_bytes());
            }
        }
        let view = self.view(&bytes, Some(TARGET_ELEMENT_ARRAY_BUFFER));
        let component = if wide { COMPONENT_UNSIGNED_INT } else { COMPONENT_UNSIGNED_SHORT };
        self.accessors.push(json!({ "bufferView": view, "componentType": component, "count": indices.len(), "type": "SCALAR" }));
        self.accessors.len() - 1
    }

    /// A PNG image stored in the buffer; returns the image index.
    pub fn png_image(&mut self, name: &str, png: &[u8]) -> usize {
        let view = self.view(png, None);
        self.images.push(json!({ "name": name, "bufferView": view, "mimeType": "image/png" }));
        self.images.len() - 1
    }

    /// The GLB file of the document with `roots` as its single scene; `asset_extras` goes into `asset.extras`.
    pub fn finish(self, scene_name: &str, roots: Vec<usize>, asset_extras: Value) -> Vec<u8> {
        let mut document = Map::new();
        document.insert(
            "asset".into(),
            json!({ "version": GLTF_VERSION, "generator": concat!("effect-gltf ", env!("CARGO_PKG_VERSION")), "extras": asset_extras }),
        );
        document.insert("scene".into(), json!(0));
        document.insert("scenes".into(), json!([{ "name": scene_name, "nodes": roots }]));
        document.insert("nodes".into(), Value::Array(self.nodes.iter().map(Node::to_json).collect()));
        let arrays = [
            ("meshes", self.meshes),
            ("materials", self.materials),
            ("textures", self.textures),
            ("images", self.images),
            ("samplers", self.samplers),
            ("animations", self.animations),
            ("accessors", self.accessors),
            ("bufferViews", self.buffer_views),
        ];
        for (key, values) in arrays {
            if !values.is_empty() {
                document.insert(key.into(), Value::Array(values));
            }
        }
        let mut buffer = self.buffer;
        while buffer.len() % GLB_ALIGNMENT != 0 {
            buffer.push(BIN_PADDING);
        }
        if !buffer.is_empty() {
            document.insert("buffers".into(), json!([{ "byteLength": buffer.len() }]));
        }
        if !self.lights.is_empty() {
            document.insert("extensions".into(), json!({ "KHR_lights_punctual": { "lights": self.lights } }));
        }
        if !self.extensions_used.is_empty() {
            document.insert("extensionsUsed".into(), json!(self.extensions_used));
        }
        let mut json_bytes = serde_json::to_vec(&Value::Object(document)).expect("a JSON value serialises");
        while json_bytes.len() % GLB_ALIGNMENT != 0 {
            json_bytes.push(JSON_PADDING);
        }
        let mut total = GLB_HEADER_BYTES + CHUNK_HEADER_BYTES + json_bytes.len();
        if !buffer.is_empty() {
            total += CHUNK_HEADER_BYTES + buffer.len();
        }
        let mut glb = Vec::with_capacity(total);
        glb.extend_from_slice(&GLB_MAGIC.to_le_bytes());
        glb.extend_from_slice(&GLB_VERSION.to_le_bytes());
        glb.extend_from_slice(&(total as u32).to_le_bytes());
        glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
        glb.extend_from_slice(&CHUNK_JSON.to_le_bytes());
        glb.extend_from_slice(&json_bytes);
        if !buffer.is_empty() {
            glb.extend_from_slice(&(buffer.len() as u32).to_le_bytes());
            glb.extend_from_slice(&CHUNK_BIN.to_le_bytes());
            glb.extend_from_slice(&buffer);
        }
        glb
    }
}
