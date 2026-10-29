//! Owned skin, skeleton and blend-shape data.
use crate::{Matrix, Transform};

/// Skinning method stored in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SkinningMethod {
    /// Linear blend skinning.
    Linear,
    /// Rigid skinning.
    Rigid,
    /// Dual-quaternion skinning.
    DualQuaternion,
    /// Per-vertex blend of linear and dual-quaternion skinning.
    BlendedDqLinear,
}

/// A skeletal node attribute.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Bone {
    /// Object name.
    pub name: String,
    /// Source object ID, absent for generated objects.
    pub source_id: Option<i64>,
    /// Indices into Scene::nodes.
    pub instances: Vec<usize>,
    /// Display radius in converted units.
    pub radius: f64,
    /// Display length relative to the parent.
    pub relative_length: f64,
    /// Whether the attribute was authored as a root bone.
    pub is_root: bool,
}

/// One unnormalized control-point influence.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SkinWeight {
    /// Index within SkinDeformer::clusters.
    pub cluster_index: usize,
    /// Stored weight, without truncation or normalization.
    pub weight: f64,
}

/// Influence range for one control point.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SkinVertex {
    /// First index in SkinDeformer::weights.
    pub weight_begin: usize,
    /// Number of influences.
    pub num_weights: usize,
    /// Dual-quaternion blend factor.
    pub dq_weight: f64,
}

/// A skin with all influences, sorted stably by descending weight per vertex.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SkinDeformer {
    /// Object name.
    pub name: String,
    /// Source object ID.
    pub source_id: Option<i64>,
    /// Skinning algorithm.
    pub skinning_method: SkinningMethod,
    /// Indices into Scene::skin_clusters, in connection order.
    pub clusters: Vec<usize>,
    /// Influence ranges indexed by control point; includes unweighted points.
    pub vertices: Vec<SkinVertex>,
    /// All influences, without a maximum-influence reduction.
    pub weights: Vec<SkinWeight>,
    /// Largest influence count.
    pub max_weights_per_vertex: usize,
    /// Authored control-point indices for dual-quaternion blend factors.
    pub dq_vertices: Vec<u32>,
    /// Authored dual-quaternion blend factors.
    pub dq_weights: Vec<f64>,
}

/// One bone binding and its authored sparse weights.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SkinCluster {
    /// Object name.
    pub name: String,
    /// Source object ID.
    pub source_id: Option<i64>,
    /// Index into Scene::nodes.
    pub bone_node: Option<usize>,
    /// Geometry-to-bone bind matrix.
    pub geometry_to_bone: Matrix,
    /// Mesh-node-to-bone bind matrix.
    pub mesh_node_to_bone: Matrix,
    /// Bone bind matrix in converted world space.
    pub bind_to_world: Matrix,
    /// Geometry matrix using the loaded bone transform.
    pub geometry_to_world: Matrix,
    /// Decomposition of geometry_to_world.
    pub geometry_to_world_transform: Transform,
    /// Authored sparse control-point indices, in source order.
    pub vertices: Vec<u32>,
    /// Authored sparse weights, parallel to vertices.
    pub weights: Vec<f64>,
}

/// A group of blend-shape channels.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BlendDeformer {
    /// Object name.
    pub name: String,
    /// Source object ID.
    pub source_id: Option<i64>,
    /// Indices into Scene::blend_channels, in connection order.
    pub channels: Vec<usize>,
}

/// An in-between blend target.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BlendKeyframe {
    /// Index into Scene::blend_shapes.
    pub shape: usize,
    /// Full target weight (percentage divided by 100).
    pub target_weight: f64,
    /// Contribution at the stored channel weight.
    pub effective_weight: f64,
}

/// A blend-shape channel at its stored deformation percentage.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BlendChannel {
    /// Object name.
    pub name: String,
    /// Source object ID, absent for generated objects.
    pub source_id: Option<i64>,
    /// DeformPercent multiplied by 0.01; animation is not evaluated.
    pub weight: f64,
    /// In-between targets stably sorted by target weight.
    pub keyframes: Vec<BlendKeyframe>,
    /// Last target, indexed into Scene::blend_shapes.
    pub target_shape: Option<usize>,
}

/// Sparse offsets for a blend target, stably sorted by control point.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BlendShape {
    /// Object name.
    pub name: String,
    /// Source object ID, absent for generated objects.
    pub source_id: Option<i64>,
    /// Control-point indices.
    pub offset_vertices: Vec<u32>,
    /// Position offsets in converted geometry space.
    pub position_offsets: Vec<[f64; 3]>,
    /// Normal offsets, empty when absent.
    pub normal_offsets: Vec<[f64; 3]>,
    /// Optional per-offset weights used by older Blender exporters.
    pub offset_weights: Vec<f64>,
}

/// One node transform in a pose.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BonePose {
    /// Index into Scene::nodes.
    pub bone_node: usize,
    /// Stored bind/rest transform in converted world space.
    pub bone_to_world: Matrix,
    /// Transform relative to the parent in this pose, or its loaded transform.
    pub bone_to_parent: Matrix,
}

/// A stored bind or rest pose.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Pose {
    /// Object name.
    pub name: String,
    /// Source object ID.
    pub source_id: Option<i64>,
    /// Whether this is a BindPose.
    pub is_bind_pose: bool,
    /// Node transforms in source node order.
    pub bone_poses: Vec<BonePose>,
}
