//! A pure-Rust FBX loader with skins, blend shapes and animation evaluation.
//!
//! [`load`] parses a binary or ASCII FBX file (versions 7100 to 7700) and
//! converts it into an owned [`Scene`]: the node hierarchy with local and
//! world transforms, meshes with every UV and color set and a triangulated
//! index buffer, materials (FBX and PBR maps), textures, cameras, lights,
//! settings, bones, skins, blend shapes, poses, and the list of content it
//! does not convert yet.
//!
//! ```
//! let fbx = br#"; FBX 7.4.0 project file
//! FBXHeaderExtension:  {
//!     FBXVersion: 7400
//! }
//! Objects:  {
//!     Geometry: 1, "Geometry::Tri", "Mesh" {
//!         Vertices: *9 { a: 0,0,0, 1,0,0, 0,1,0 }
//!         PolygonVertexIndex: *3 { a: 0,1,-3 }
//!     }
//!     Model: 2, "Model::Tri", "Mesh" {
//!     }
//! }
//! Connections:  {
//!     C: "OO",1,2
//!     C: "OO",2,0
//! }
//! "#;
//! let scene = fbx_rs::load(fbx, &fbx_rs::LoadOptions::default())?;
//! let node = &scene.nodes()[1];
//! assert_eq!(node.name(), "Tri");
//! let mesh = &scene.meshes()[node.mesh().unwrap()];
//! assert_eq!(mesh.positions().values().len(), 3);
//! assert_eq!(mesh.triangles(), &[0, 1, 2]);
//! # Ok::<(), fbx_rs::Error>(())
//! ```
//!
//! # What is supported
//!
//! - Binary and ASCII FBX 7.x (file versions 7100 to 7700).
//! - Static scenes: the node hierarchy and transforms, meshes (positions,
//!   normals, UV sets with tangents, color sets, faces, material parts,
//!   triangulation), materials, textures (paths and embedded content),
//!   cameras and lights, and the scene settings.
//! - Stored skins, bind/rest poses, bones and blend shapes are converted;
//!   mesh positions stay undeformed. Animation curves, layers and pose evaluation
//!   are exposed through [`Scene`]. NURBS geometry, caches, constraints and audio
//!   are listed in [`Scene::unsupported`].
//! - FBX 6.1 and older files are rejected with [`Error::Parse`].
//! - Only the default [`LoadOptions`] (right-handed Y-up, 1 m units) are
//!   implemented; other values return [`Error::UnsupportedOptions`].
//!
//! Every [`load`] call is bounded by [`Limits`]: input size, node counts,
//! array lengths and decompressed sizes are checked before allocation. The
//! crate contains no `unsafe` code.
//!
//! The [`syntax`] module is the lower layer: a bounded binary and ASCII FBX
//! parser producing a node/property tree.
//!
//! The non-default `parity` feature exposes the converter's internal modules
//! (scene records and the canonical `fbx-scene-canon/1` writer) to the
//! unpublished `fbx-parity` crate. It is unstable, outside semver, and builds
//! only from the source repository.
#![forbid(unsafe_code)]
#![deny(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
#![warn(missing_docs)]

mod allocation;
mod api_alloc;
mod bake;
mod fallible_map;
pub use bake::{BakeOptions, BakedAnimation, BakedKey, BakedNode, BakedProperty};
mod animation;
#[cfg(feature = "parity")]
mod animation_canon;
mod api;
mod conversion_budget;
mod numeric;
#[cfg(test)]
mod scaling_tests;
mod scene_animation;
pub use animation::{
    AnimationBinding, AnimationCurve, AnimationLayer, AnimationStack, AnimationValue,
    EvaluatedNode, EvaluatedProperty, EvaluatedScene, Extrapolation, ExtrapolationMode,
    Interpolation, Keyframe, Tangent,
};
mod deform;
mod scene_deform;
pub use deform::*;
pub mod syntax;

// Converter internals: public only for the parity tooling (unstable). Without
// the feature, the parts that serve only the canonical dump (record variants,
// DOM projection constants, mesh validation helpers) are allowed to be unused.
// With the feature they are hidden from the documentation and exempt from the
// missing-docs lint: they are not part of the supported API.
#[cfg(feature = "parity")]
#[doc(hidden)]
#[allow(missing_docs)]
pub mod canon;
#[cfg(not(feature = "parity"))]
#[allow(dead_code)]
mod canon;
#[cfg(feature = "parity")]
#[doc(hidden)]
#[allow(missing_docs)]
pub mod canon_dom;
#[cfg(not(feature = "parity"))]
#[allow(dead_code)]
mod canon_dom;
#[cfg(feature = "parity")]
#[doc(hidden)]
#[allow(missing_docs)]
pub mod canon_scene;
#[cfg(not(feature = "parity"))]
#[allow(dead_code)]
mod canon_scene;
#[cfg(feature = "parity")]
#[doc(hidden)]
#[allow(missing_docs)]
pub mod scene;
#[cfg(not(feature = "parity"))]
#[allow(dead_code)]
mod scene;
mod scene_attribute;
mod scene_geometry;
mod scene_material;
#[cfg(feature = "parity")]
#[doc(hidden)]
#[allow(missing_docs)]
pub mod scene_mesh;
#[cfg(not(feature = "parity"))]
#[allow(dead_code)]
mod scene_mesh;
#[cfg(feature = "parity")]
#[doc(hidden)]
#[allow(missing_docs)]
pub mod scene_node;
#[cfg(not(feature = "parity"))]
mod scene_node;
#[cfg(feature = "parity")]
#[doc(hidden)]
#[allow(missing_docs)]
pub mod scene_property;
#[cfg(not(feature = "parity"))]
mod scene_property;

// Parity-only: canonical stream framing and schema, and the early draft
// converters that feed the dump's triangles section and the tests.
#[cfg(feature = "parity")]
mod canon_schema;
#[cfg(feature = "parity")]
#[doc(hidden)]
#[allow(missing_docs)]
pub mod canonical_writer;
#[cfg(feature = "parity")]
mod import_profile;
#[cfg(feature = "parity")]
#[doc(hidden)]
#[allow(missing_docs)]
pub mod scene_draft;
#[cfg(feature = "parity")]
#[doc(hidden)]
#[allow(missing_docs)]
pub mod scene_surface;
#[cfg(feature = "parity")]
#[doc(hidden)]
#[allow(missing_docs)]
pub mod scene_transform;

pub use api::{
    load, load_with_limits, Axis, Camera, ColorSet, CoordinateAxes, Error, Face,
    GeometryTransformHandling, InheritModeHandling, Light, LightKind, Limits, LoadOptions,
    Material, MaterialFeature, MaterialMap, MaterialTexture, Matrix, Mesh, MeshPart, Node, Scene,
    SceneSettings, SourceProperty, SpaceConversion, Texture, TextureLayer, Transform, Unsupported,
    UnsupportedKind, UvSet, VertexAttribute,
};

// Compile the public usage examples as part of the crate doctests.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
mod readme {}
