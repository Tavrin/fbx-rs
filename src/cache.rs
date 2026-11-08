//! Fast-reload scene cache: a versioned, disposable binary sidecar of a
//! loaded [`Scene`].
//!
//! [`Scene::write_cache`] serializes a scene under a [`CacheKey`];
//! [`read`] returns the scene again when the key matches byte for byte.
//! The key holds the cache format version, the crate version, the
//! [`LoadOptions`], the [`Limits`] and a digest of the source file that the
//! caller computes (for example a BLAKE3 or SHA-256 of the FBX bytes). Any
//! difference, including a different crate version, is a
//! [`ReadError::Miss`]: the cache is never migrated, so the caller loads the
//! FBX file again and rewrites the sidecar.
//!
//! The crate never touches the file system. The caller owns the path, and
//! should write to a temporary file and rename it into place, so that a
//! reader never sees a partial file.
//!
//! ```
//! use fbx_rs::cache::{self, CacheKey, ReadError};
//! use fbx_rs::{Limits, LoadOptions};
//!
//! # fn digest(bytes: &[u8]) -> [u8; 8] { (bytes.len() as u64).to_le_bytes() }
//! # let fbx: &[u8] = b"; FBX 7.4.0 project file\n";
//! let (options, limits) = (LoadOptions::default(), Limits::default());
//! let key = CacheKey::new(&digest(fbx), &options, &limits).expect("1..=64 digest bytes");
//! # let sidecar: Vec<u8> = Vec::new();
//! match cache::read(&sidecar, &key, &limits) {
//!     Ok(scene) => drop(scene),
//!     Err(ReadError::Miss(_) | ReadError::Corrupt) => {
//!         // Load the FBX file, then `scene.write_cache(&key, file)`.
//!     }
//!     Err(other) => panic!("{other}"),
//! }
//! ```
//!
//! # Format `fbx-rs-scene-cache/1`
//!
//! Little-endian throughout:
//!
//! ```text
//! 0   magic     b"FBXRSSC\0"
//! 8   format    u32 = 1
//! 12  key       u32 length + CacheKey bytes
//! ..  sections  u32 count (17), then per section {tag [u8; 4], offset u64, length u64}
//! ..  payload   SCEN NODE MESH MATL TEXR CAMR LGHT BONE SKND SKCL BLND BLCH
//!               BLSH POSE ANIM UNSP WARN, contiguous, in this order
//! end trailer   u32 Adler-32 of all preceding bytes, b"CSRF"
//! ```
//!
//! Offsets are absolute. `f64` and `f32` are stored as raw bits, so NaN
//! payloads and negative zero survive. A vector is a `u64` count followed by
//! its elements, a string a `u64` byte length followed by UTF-8, an `Option`
//! a `u8` tag (0 or 1) followed by the value, and `usize` a `u64`.
//!
//! The Adler-32 trailer detects torn and truncated writes, not tampering:
//! whoever can write the cache can also write the FBX file it stands for.
//! The reader still treats the bytes as untrusted: every count is checked
//! against the remaining section bytes before anything is allocated, every
//! allocation is charged to [`Limits::max_allocation`], and malformed input
//! returns an error instead of panicking.
use crate::animation::{Animation, NodeEval};
use crate::api::ElementInfo;
use crate::api_alloc::Budget;
use crate::fallible_map::Map;
use crate::scene_node::{Adjust, NodeProps};
use crate::{
    AnimationBinding, AnimationCurve, AnimationLayer, AnimationStack, AnimationValue, Axis,
    BlendChannel, BlendDeformer, BlendKeyframe, BlendShape, Bone, BonePose, Camera, ColorSet,
    CoordinateAxes, Error, EvaluatedProperty, Extrapolation, ExtrapolationMode, Face,
    GeometryTransformHandling, InheritModeHandling, Interpolation, Keyframe, Light, LightKind,
    Limit, Limits, LoadOptions, Material, MaterialFeature, MaterialMap, MaterialTexture, Matrix,
    Mesh, MeshPart, Node, Pose, Scene, SceneSettings, SkinCluster, SkinDeformer, SkinVertex,
    SkinWeight, SkinningMethod, SourceProperty, SpaceConversion, Tangent, Texture, TextureLayer,
    Transform, Unsupported, UnsupportedKind, UvSet, VertexAttribute,
};
use std::io::{self, Write};

/// Version of the sidecar layout written by this crate.
pub const FORMAT_VERSION: u32 = 1;

const MAGIC: [u8; 8] = *b"FBXRSSC\0";
const TRAILER_TAG: [u8; 4] = *b"CSRF";
const SECTION_COUNT: usize = 17;
const TAGS: [[u8; 4]; SECTION_COUNT] = [
    *b"SCEN", *b"NODE", *b"MESH", *b"MATL", *b"TEXR", *b"CAMR", *b"LGHT", *b"BONE", *b"SKND",
    *b"SKCL", *b"BLND", *b"BLCH", *b"BLSH", *b"POSE", *b"ANIM", *b"UNSP", *b"WARN",
];

/// Literal identity of a cached load, compared byte for byte; no hash
/// function is applied.
///
/// Holds [`FORMAT_VERSION`], the crate version, the [`LoadOptions`], the
/// [`Limits`] and the caller's digest of the source file. Limits are part of
/// the key because a scene that loaded under looser limits must not be
/// returned to a caller whose limits would have refused it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    bytes: Vec<u8>,
}

/// A digest passed to [`CacheKey::new`] was empty or longer than
/// [`CacheKey::MAX_DIGEST_LEN`] bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DigestLengthError {
    /// Length of the rejected digest in bytes.
    pub len: usize,
}

impl std::fmt::Display for DigestLengthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cache key digest of {} bytes; expected 1 to {}",
            self.len,
            CacheKey::MAX_DIGEST_LEN
        )
    }
}

impl std::error::Error for DigestLengthError {}

impl CacheKey {
    /// Largest accepted digest, in bytes.
    pub const MAX_DIGEST_LEN: usize = 64;

    /// Build the key of a load of the file whose digest is `digest` with
    /// `options` and `limits`.
    ///
    /// # Errors
    /// [`DigestLengthError`] when `digest` is empty or longer than
    /// [`CacheKey::MAX_DIGEST_LEN`] bytes.
    pub fn new(
        digest: &[u8],
        options: &LoadOptions,
        limits: &Limits,
    ) -> Result<Self, DigestLengthError> {
        let digest_len = u8::try_from(digest.len())
            .ok()
            .filter(|&n| n >= 1 && usize::from(n) <= Self::MAX_DIGEST_LEN)
            .ok_or(DigestLengthError { len: digest.len() })?;
        let version = env!("CARGO_PKG_VERSION").as_bytes();
        let mut bytes = Vec::with_capacity(256);
        bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        // A crate version is far shorter than 255 bytes.
        bytes.push(u8::try_from(version.len()).unwrap_or(u8::MAX));
        bytes.extend_from_slice(version);
        let LoadOptions {
            target_axes,
            target_unit_meters,
            space_conversion,
            geometry_transform_handling,
            inherit_mode_handling,
            target_camera_axes,
            target_light_axes,
        } = options;
        let axes = |bytes: &mut Vec<u8>, a: &CoordinateAxes| {
            let CoordinateAxes { right, up, front } = a;
            bytes.extend_from_slice(&[axis_code(*right), axis_code(*up), axis_code(*front)]);
        };
        axes(&mut bytes, target_axes);
        bytes.extend_from_slice(&target_unit_meters.to_bits().to_le_bytes());
        bytes.push(match space_conversion {
            SpaceConversion::ModifyGeometry => 0,
        });
        bytes.push(match geometry_transform_handling {
            GeometryTransformHandling::ModifyGeometry => 0,
        });
        bytes.push(match inherit_mode_handling {
            InheritModeHandling::Compensate => 0,
        });
        axes(&mut bytes, target_camera_axes);
        axes(&mut bytes, target_light_axes);
        let Limits {
            max_input_bytes,
            max_depth,
            max_nodes,
            max_properties,
            max_array_elements,
            max_decompressed_bytes,
            max_string_length,
            max_objects,
            max_animation_keys,
            max_connections,
            max_allocation,
            max_deformer_work,
            max_animation_work,
            max_meshes,
            max_layers,
            max_face_vertices,
            max_triangulation_work,
        } = limits;
        for n in [
            max_input_bytes,
            max_depth,
            max_nodes,
            max_properties,
            max_array_elements,
            max_decompressed_bytes,
            max_string_length,
            max_objects,
            max_animation_keys,
            max_connections,
            max_allocation,
            max_deformer_work,
            max_animation_work,
            max_meshes,
            max_layers,
            max_face_vertices,
            max_triangulation_work,
        ] {
            bytes.extend_from_slice(&wide(*n).to_le_bytes());
        }
        bytes.push(digest_len);
        bytes.extend_from_slice(digest);
        Ok(Self { bytes })
    }

    /// The canonical key bytes, as stored in the sidecar.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Why a sidecar was not used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MissReason {
    /// The bytes do not start with the sidecar magic.
    NotACache,
    /// The sidecar has a different [`FORMAT_VERSION`].
    FormatVersion,
    /// The sidecar was written for another file, crate version, options or
    /// limits.
    Key,
}

/// Errors returned by [`read`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReadError {
    /// The sidecar does not belong to this key; load the FBX file instead.
    Miss(MissReason),
    /// The sidecar is truncated, torn or malformed; load the FBX file instead.
    Corrupt,
    /// A [`Limits`] field was exceeded while reading.
    Limit(Limit),
    /// An allocator refused a fallible reservation.
    Allocation,
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Miss(MissReason::NotACache) => f.write_str("not a scene cache"),
            ReadError::Miss(MissReason::FormatVersion) => {
                f.write_str("scene cache format version differs")
            }
            ReadError::Miss(MissReason::Key) => f.write_str("scene cache key differs"),
            ReadError::Corrupt => f.write_str("scene cache is corrupt"),
            ReadError::Limit(limit) => write!(
                f,
                "limit exceeded while reading the scene cache (Limits::{})",
                limit.field()
            ),
            ReadError::Allocation => f.write_str("allocation refused"),
        }
    }
}

impl std::error::Error for ReadError {}

type R<T> = Result<T, ReadError>;

fn alloc_error(e: Error) -> ReadError {
    match e {
        Error::Allocation => ReadError::Allocation,
        Error::LimitExceeded(limit) => ReadError::Limit(limit),
        _ => ReadError::Limit(Limit::Allocation),
    }
}

/// Read a scene written by [`Scene::write_cache`] under `key`.
///
/// Allocations are charged to `limits.max_allocation`, as during a load.
///
/// # Errors
/// [`ReadError::Miss`] when the bytes are not a sidecar of this format or
/// were written under a different key; [`ReadError::Corrupt`] when they are
/// truncated or malformed; [`ReadError::Limit`] or [`ReadError::Allocation`]
/// when the scene does not fit the allocation envelope. No partial scene is
/// returned.
pub fn read(bytes: &[u8], key: &CacheKey, limits: &Limits) -> Result<Scene, ReadError> {
    if bytes.get(..MAGIC.len()) != Some(&MAGIC[..]) {
        return Err(ReadError::Miss(MissReason::NotACache));
    }
    let mut header = In::new(bytes);
    header.take(MAGIC.len())?;
    if header.u32()? != FORMAT_VERSION {
        return Err(ReadError::Miss(MissReason::FormatVersion));
    }
    let key_len = usize::try_from(header.u32()?).map_err(|_| ReadError::Corrupt)?;
    if header.take(key_len)? != key.as_bytes() {
        return Err(ReadError::Miss(MissReason::Key));
    }
    let body_end = bytes.len().checked_sub(8).ok_or(ReadError::Corrupt)?;
    let (body, trailer) = bytes.split_at(body_end);
    let (checksum, tag) = trailer.split_at(4);
    if tag != TRAILER_TAG
        || checksum
            != miniz_oxide::mz_adler32_oxide(1, body)
                .to_le_bytes()
                .as_slice()
    {
        return Err(ReadError::Corrupt);
    }
    if usize::try_from(header.u32()?).ok() != Some(SECTION_COUNT) {
        return Err(ReadError::Corrupt);
    }
    let mut ranges = [(0usize, 0usize); SECTION_COUNT];
    let table_end = header
        .pos
        .checked_add(SECTION_COUNT * 20)
        .ok_or(ReadError::Corrupt)?;
    let mut next = table_end;
    for (range, tag) in ranges.iter_mut().zip(TAGS) {
        let found: [u8; 4] = header.arr()?;
        let offset = header.usize()?;
        let len = header.usize()?;
        if found != tag || offset != next {
            return Err(ReadError::Corrupt);
        }
        next = offset.checked_add(len).ok_or(ReadError::Corrupt)?;
        *range = (offset, next);
    }
    if next != body_end || header.pos != table_end {
        return Err(ReadError::Corrupt);
    }
    let _scope = crate::allocation::LoadScope::new(limits.max_allocation);
    let a = Budget::new(limits.max_allocation);
    let mut sections = ranges
        .iter()
        .map(|&(start, end)| body.get(start..end).map(In::new).ok_or(ReadError::Corrupt));
    let mut section = || sections.next().unwrap_or(Err(ReadError::Corrupt));
    fn whole<'a, T>(
        mut i: In<'a>,
        a: &Budget,
        get: impl FnOnce(&mut In<'a>, &Budget) -> R<T>,
    ) -> R<T> {
        let value = get(&mut i, a)?;
        i.finish()?;
        Ok(value)
    }
    let (element_count, settings) = whole(section()?, &a, |i, a| {
        Ok((i.usize()?, SceneSettings::get(i, a)?))
    })?;
    let nodes = whole(section()?, &a, Codec::get)?;
    let meshes = whole(section()?, &a, Codec::get)?;
    let materials = whole(section()?, &a, Codec::get)?;
    let textures = whole(section()?, &a, Codec::get)?;
    let cameras = whole(section()?, &a, Codec::get)?;
    let lights = whole(section()?, &a, Codec::get)?;
    let bones = whole(section()?, &a, Codec::get)?;
    let skin_deformers = whole(section()?, &a, Codec::get)?;
    let skin_clusters = whole(section()?, &a, Codec::get)?;
    let blend_deformers = whole(section()?, &a, Codec::get)?;
    let blend_channels = whole(section()?, &a, Codec::get)?;
    let blend_shapes = whole(section()?, &a, Codec::get)?;
    let poses = whole(section()?, &a, Codec::get)?;
    let animation = whole(section()?, &a, get_animation)?;
    let unsupported = whole(section()?, &a, Codec::get)?;
    let warnings = whole(section()?, &a, Codec::get)?;
    Ok(Scene {
        element_count,
        animation,
        nodes,
        meshes,
        materials,
        textures,
        cameras,
        lights,
        bones,
        skin_deformers,
        skin_clusters,
        blend_deformers,
        blend_channels,
        blend_shapes,
        poses,
        settings,
        unsupported,
        warnings,
    })
}

type Encode = fn(&mut Out<'_>, &Scene) -> io::Result<()>;

/// Section encoders in [`TAGS`] order. The first destructures [`Scene`]
/// without `..`, so a new field fails to compile until it is encoded.
const SECTIONS: [Encode; SECTION_COUNT] = [
    |o, s| {
        let Scene {
            element_count,
            animation: _,
            nodes: _,
            meshes: _,
            materials: _,
            textures: _,
            cameras: _,
            lights: _,
            bones: _,
            skin_deformers: _,
            skin_clusters: _,
            blend_deformers: _,
            blend_channels: _,
            blend_shapes: _,
            poses: _,
            settings,
            unsupported: _,
            warnings: _,
        } = s;
        element_count.put(o)?;
        settings.put(o)
    },
    |o, s| s.nodes.put(o),
    |o, s| s.meshes.put(o),
    |o, s| s.materials.put(o),
    |o, s| s.textures.put(o),
    |o, s| s.cameras.put(o),
    |o, s| s.lights.put(o),
    |o, s| s.bones.put(o),
    |o, s| s.skin_deformers.put(o),
    |o, s| s.skin_clusters.put(o),
    |o, s| s.blend_deformers.put(o),
    |o, s| s.blend_channels.put(o),
    |o, s| s.blend_shapes.put(o),
    |o, s| s.poses.put(o),
    |o, s| put_animation(o, &s.animation),
    |o, s| s.unsupported.put(o),
    |o, s| s.warnings.put(o),
];

impl Scene {
    /// Write this scene as a `fbx-rs-scene-cache/1` sidecar under `key`;
    /// see [`crate::cache`]. Deterministic: equal scenes write equal bytes.
    ///
    /// The scene is encoded twice (once to size the sections, once to write
    /// them), so nothing proportional to the scene is buffered. The writer
    /// receives large sequential writes and is flushed at the end.
    ///
    /// # Errors
    /// Errors of `w`.
    pub fn write_cache<W: Write>(&self, key: &CacheKey, mut w: W) -> io::Result<()> {
        let mut lengths = [0u64; SECTION_COUNT];
        for (length, encode) in lengths.iter_mut().zip(SECTIONS) {
            let mut counter = Out::counter();
            encode(&mut counter, self)?;
            *length = counter.len;
        }
        let key = key.as_bytes();
        let mut o = Out::writer(&mut w);
        o.raw(&MAGIC)?;
        o.u32(FORMAT_VERSION)?;
        o.u32(u32::try_from(key.len()).map_err(|_| invalid("cache key length"))?)?;
        o.raw(key)?;
        o.u32(u32::try_from(SECTION_COUNT).map_err(|_| invalid("section count"))?)?;
        let mut offset = o
            .len
            .checked_add(wide(SECTION_COUNT * 20))
            .ok_or_else(|| invalid("cache size"))?;
        for (tag, &length) in TAGS.iter().zip(&lengths) {
            o.raw(tag)?;
            o.u64(offset)?;
            o.u64(length)?;
            offset = offset
                .checked_add(length)
                .ok_or_else(|| invalid("cache size"))?;
        }
        for (encode, &length) in SECTIONS.iter().zip(&lengths) {
            let start = o.len;
            encode(&mut o, self)?;
            if o.len.checked_sub(start) != Some(length) {
                return Err(invalid("scene cache section changed size between passes"));
            }
        }
        o.flush_buffer()?;
        let checksum = o.adler;
        drop(o);
        w.write_all(&checksum.to_le_bytes())?;
        w.write_all(&TRAILER_TAG)?;
        w.flush()
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// `usize` widened to the stored `u64`; lossless on every supported target.
fn wide(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

const BUFFER: usize = 1 << 16;

/// Encoder sink: counts bytes, or buffers them into the writer while
/// maintaining the running Adler-32.
struct Out<'w> {
    w: Option<&'w mut dyn Write>,
    buf: Vec<u8>,
    len: u64,
    adler: u32,
}

impl<'w> Out<'w> {
    fn counter() -> Self {
        Self {
            w: None,
            buf: Vec::new(),
            len: 0,
            adler: 1,
        }
    }
    fn writer(w: &'w mut dyn Write) -> Self {
        Self {
            w: Some(w),
            buf: Vec::with_capacity(BUFFER),
            len: 0,
            adler: 1,
        }
    }
    fn advance(&mut self, n: usize) -> io::Result<()> {
        self.len = self
            .len
            .checked_add(wide(n))
            .ok_or_else(|| invalid("cache size"))?;
        Ok(())
    }
    fn flush_buffer(&mut self) -> io::Result<()> {
        if let Some(w) = self.w.as_mut() {
            if !self.buf.is_empty() {
                self.adler = miniz_oxide::mz_adler32_oxide(self.adler, &self.buf);
                w.write_all(&self.buf)?;
                self.buf.clear();
            }
        }
        Ok(())
    }
    fn raw(&mut self, b: &[u8]) -> io::Result<()> {
        self.advance(b.len())?;
        if self.w.is_none() {
            return Ok(());
        }
        if self.buf.len().saturating_add(b.len()) > BUFFER {
            self.flush_buffer()?;
        }
        if b.len() > BUFFER {
            if let Some(w) = self.w.as_mut() {
                self.adler = miniz_oxide::mz_adler32_oxide(self.adler, b);
                w.write_all(b)?;
            }
        } else {
            self.buf.extend_from_slice(b);
        }
        Ok(())
    }
    fn u8(&mut self, v: u8) -> io::Result<()> {
        self.raw(&[v])
    }
    fn u32(&mut self, v: u32) -> io::Result<()> {
        self.raw(&v.to_le_bytes())
    }
    fn u64(&mut self, v: u64) -> io::Result<()> {
        self.raw(&v.to_le_bytes())
    }
    fn count(&mut self, n: usize) -> io::Result<()> {
        self.u64(wide(n))
    }
    /// A plain-old-data array as one contiguous block of `WIDTH`-byte values.
    fn block<T: Copy, const WIDTH: usize>(
        &mut self,
        values: &[T],
        bytes: impl Fn(T) -> [u8; WIDTH],
    ) -> io::Result<()> {
        self.count(values.len())?;
        if self.w.is_none() {
            return self.advance(values.len().saturating_mul(WIDTH));
        }
        let mut chunk = [0u8; 4096];
        for part in values.chunks(4096 / WIDTH) {
            let mut end = 0;
            for (slot, &v) in chunk.as_chunks_mut::<WIDTH>().0.iter_mut().zip(part) {
                *slot = bytes(v);
                end += WIDTH;
            }
            self.raw(&chunk[..end])?;
        }
        Ok(())
    }
    fn f64s(&mut self, v: &[f64]) -> io::Result<()> {
        self.block(v, |x: f64| x.to_bits().to_le_bytes())
    }
    fn u32s(&mut self, v: &[u32]) -> io::Result<()> {
        self.block(v, u32::to_le_bytes)
    }
    fn bytes(&mut self, v: &[u8]) -> io::Result<()> {
        self.count(v.len())?;
        self.raw(v)
    }
}

/// Decoder over one section (or the header): every read is bounds-checked.
struct In<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> In<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, pos: 0 }
    }
    fn take(&mut self, n: usize) -> R<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(ReadError::Corrupt)?;
        let s = self.b.get(self.pos..end).ok_or(ReadError::Corrupt)?;
        self.pos = end;
        Ok(s)
    }
    fn arr<const N: usize>(&mut self) -> R<[u8; N]> {
        self.take(N)?
            .first_chunk::<N>()
            .copied()
            .ok_or(ReadError::Corrupt)
    }
    fn remaining(&self) -> usize {
        self.b.len().saturating_sub(self.pos)
    }
    fn finish(&self) -> R<()> {
        if self.pos == self.b.len() {
            Ok(())
        } else {
            Err(ReadError::Corrupt)
        }
    }
    fn u8(&mut self) -> R<u8> {
        Ok(self.arr::<1>()?[0])
    }
    fn u32(&mut self) -> R<u32> {
        Ok(u32::from_le_bytes(self.arr()?))
    }
    fn u64(&mut self) -> R<u64> {
        Ok(u64::from_le_bytes(self.arr()?))
    }
    fn usize(&mut self) -> R<usize> {
        usize::try_from(self.u64()?).map_err(|_| ReadError::Corrupt)
    }
    /// An element count, refused unless `count × min` bytes remain.
    fn count(&mut self, min: usize) -> R<usize> {
        let n = self.usize()?;
        match n.checked_mul(min.max(1)) {
            Some(bytes) if bytes <= self.remaining() => Ok(n),
            _ => Err(ReadError::Corrupt),
        }
    }
    fn block<T, const WIDTH: usize>(
        &mut self,
        a: &Budget,
        value: impl Fn([u8; WIDTH]) -> T,
    ) -> R<Vec<T>> {
        let n = self.count(WIDTH)?;
        let raw = self.take(n.checked_mul(WIDTH).ok_or(ReadError::Corrupt)?)?;
        let mut v = Vec::new();
        a.reserve(&mut v, n).map_err(alloc_error)?;
        let (chunks, _) = raw.as_chunks::<WIDTH>();
        v.extend(chunks.iter().map(|&c| value(c)));
        Ok(v)
    }
    fn f64s(&mut self, a: &Budget) -> R<Vec<f64>> {
        self.block(a, |b| f64::from_bits(u64::from_le_bytes(b)))
    }
    fn u32s(&mut self, a: &Budget) -> R<Vec<u32>> {
        self.block(a, u32::from_le_bytes)
    }
    fn bytes(&mut self, a: &Budget) -> R<Vec<u8>> {
        let n = self.count(1)?;
        let raw = self.take(n)?;
        let mut v = Vec::new();
        a.reserve(&mut v, n).map_err(alloc_error)?;
        v.extend_from_slice(raw);
        Ok(v)
    }
}

/// One encoded value. `MIN` is a lower bound of its encoded size, used to
/// refuse element counts the remaining bytes cannot hold.
trait Codec: Sized {
    const MIN: usize;
    fn put(&self, o: &mut Out<'_>) -> io::Result<()>;
    fn get(i: &mut In<'_>, a: &Budget) -> R<Self>;
}

impl Codec for u64 {
    const MIN: usize = 8;
    fn put(&self, o: &mut Out<'_>) -> io::Result<()> {
        o.u64(*self)
    }
    fn get(i: &mut In<'_>, _: &Budget) -> R<Self> {
        i.u64()
    }
}
impl Codec for usize {
    const MIN: usize = 8;
    fn put(&self, o: &mut Out<'_>) -> io::Result<()> {
        o.u64(wide(*self))
    }
    fn get(i: &mut In<'_>, _: &Budget) -> R<Self> {
        i.usize()
    }
}
impl Codec for i64 {
    const MIN: usize = 8;
    fn put(&self, o: &mut Out<'_>) -> io::Result<()> {
        o.raw(&self.to_le_bytes())
    }
    fn get(i: &mut In<'_>, _: &Budget) -> R<Self> {
        Ok(i64::from_le_bytes(i.arr()?))
    }
}
impl Codec for u32 {
    const MIN: usize = 4;
    fn put(&self, o: &mut Out<'_>) -> io::Result<()> {
        o.u32(*self)
    }
    fn get(i: &mut In<'_>, _: &Budget) -> R<Self> {
        i.u32()
    }
}
impl Codec for i32 {
    const MIN: usize = 4;
    fn put(&self, o: &mut Out<'_>) -> io::Result<()> {
        o.raw(&self.to_le_bytes())
    }
    fn get(i: &mut In<'_>, _: &Budget) -> R<Self> {
        Ok(i32::from_le_bytes(i.arr()?))
    }
}
impl Codec for f64 {
    const MIN: usize = 8;
    fn put(&self, o: &mut Out<'_>) -> io::Result<()> {
        o.u64(self.to_bits())
    }
    fn get(i: &mut In<'_>, _: &Budget) -> R<Self> {
        Ok(f64::from_bits(i.u64()?))
    }
}
impl Codec for f32 {
    const MIN: usize = 4;
    fn put(&self, o: &mut Out<'_>) -> io::Result<()> {
        o.u32(self.to_bits())
    }
    fn get(i: &mut In<'_>, _: &Budget) -> R<Self> {
        Ok(f32::from_bits(i.u32()?))
    }
}
impl Codec for bool {
    const MIN: usize = 1;
    fn put(&self, o: &mut Out<'_>) -> io::Result<()> {
        o.u8(u8::from(*self))
    }
    fn get(i: &mut In<'_>, _: &Budget) -> R<Self> {
        match i.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ReadError::Corrupt),
        }
    }
}
impl Codec for String {
    const MIN: usize = 8;
    fn put(&self, o: &mut Out<'_>) -> io::Result<()> {
        o.bytes(self.as_bytes())
    }
    fn get(i: &mut In<'_>, a: &Budget) -> R<Self> {
        String::from_utf8(i.bytes(a)?).map_err(|_| ReadError::Corrupt)
    }
}
impl<T: Codec> Codec for Option<T> {
    const MIN: usize = 1;
    fn put(&self, o: &mut Out<'_>) -> io::Result<()> {
        match self {
            None => o.u8(0),
            Some(v) => {
                o.u8(1)?;
                v.put(o)
            }
        }
    }
    fn get(i: &mut In<'_>, a: &Budget) -> R<Self> {
        match i.u8()? {
            0 => Ok(None),
            1 => Ok(Some(T::get(i, a)?)),
            _ => Err(ReadError::Corrupt),
        }
    }
}
impl<T: Codec> Codec for Vec<T> {
    const MIN: usize = 8;
    fn put(&self, o: &mut Out<'_>) -> io::Result<()> {
        o.count(self.len())?;
        self.iter().try_for_each(|v| v.put(o))
    }
    fn get(i: &mut In<'_>, a: &Budget) -> R<Self> {
        let n = i.count(T::MIN)?;
        let mut v = Vec::new();
        a.reserve(&mut v, n).map_err(alloc_error)?;
        for _ in 0..n {
            v.push(T::get(i, a)?);
        }
        Ok(v)
    }
}
impl<T: Codec + Copy + Default, const N: usize> Codec for [T; N] {
    const MIN: usize = T::MIN * N;
    fn put(&self, o: &mut Out<'_>) -> io::Result<()> {
        self.iter().try_for_each(|v| v.put(o))
    }
    fn get(i: &mut In<'_>, a: &Budget) -> R<Self> {
        let mut out = [T::default(); N];
        for v in &mut out {
            *v = T::get(i, a)?;
        }
        Ok(out)
    }
}
impl<A: Codec, B: Codec> Codec for (A, B) {
    const MIN: usize = A::MIN + B::MIN;
    fn put(&self, o: &mut Out<'_>) -> io::Result<()> {
        self.0.put(o)?;
        self.1.put(o)
    }
    fn get(i: &mut In<'_>, a: &Budget) -> R<Self> {
        Ok((A::get(i, a)?, B::get(i, a)?))
    }
}

/// Explicit `u8` code tables for the public enums; `as` casts are denied.
macro_rules! enum_codec {
    ($ty:ty { $($variant:path = $code:literal),+ $(,)? }) => {
        impl Codec for $ty {
            const MIN: usize = 1;
            fn put(&self, o: &mut Out<'_>) -> io::Result<()> {
                o.u8(match self { $($variant => $code),+ })
            }
            fn get(i: &mut In<'_>, _: &Budget) -> R<Self> {
                match i.u8()? {
                    $($code => Ok($variant),)+
                    _ => Err(ReadError::Corrupt),
                }
            }
        }
    };
}

enum_codec!(Axis {
    Axis::PositiveX = 0,
    Axis::NegativeX = 1,
    Axis::PositiveY = 2,
    Axis::NegativeY = 3,
    Axis::PositiveZ = 4,
    Axis::NegativeZ = 5,
});
enum_codec!(LightKind {
    LightKind::Point = 0,
    LightKind::Directional = 1,
    LightKind::Spot = 2,
    LightKind::Area = 3,
    LightKind::Volume = 4,
});
enum_codec!(SkinningMethod {
    SkinningMethod::Linear = 0,
    SkinningMethod::Rigid = 1,
    SkinningMethod::DualQuaternion = 2,
    SkinningMethod::BlendedDqLinear = 3,
});
enum_codec!(Interpolation {
    Interpolation::ConstantPrevious = 0,
    Interpolation::ConstantNext = 1,
    Interpolation::Linear = 2,
    Interpolation::Cubic = 3,
});
enum_codec!(ExtrapolationMode {
    ExtrapolationMode::Constant = 0,
    ExtrapolationMode::Repeat = 1,
    ExtrapolationMode::Mirror = 2,
    ExtrapolationMode::Slope = 3,
    ExtrapolationMode::RepeatRelative = 4,
});
enum_codec!(UnsupportedKind {
    UnsupportedKind::Skin = 0,
    UnsupportedKind::SkinCluster = 1,
    UnsupportedKind::BlendShape = 2,
    UnsupportedKind::BlendShapeChannel = 3,
    UnsupportedKind::ShapeGeometry = 4,
    UnsupportedKind::LegacyShape = 5,
    UnsupportedKind::AnimationStack = 6,
    UnsupportedKind::AnimationLayer = 7,
    UnsupportedKind::AnimationCurveNode = 8,
    UnsupportedKind::AnimationCurve = 9,
    UnsupportedKind::Pose = 10,
    UnsupportedKind::CurveOrSurfaceGeometry = 11,
    UnsupportedKind::Cache = 12,
    UnsupportedKind::Constraint = 13,
    UnsupportedKind::Audio = 14,
});

fn axis_code(axis: Axis) -> u8 {
    match axis {
        Axis::PositiveX => 0,
        Axis::NegativeX => 1,
        Axis::PositiveY => 2,
        Axis::NegativeY => 3,
        Axis::PositiveZ => 4,
        Axis::NegativeZ => 5,
    }
}

/// Struct codecs: `put` destructures without `..` and `get` builds a struct
/// literal, so adding a field fails to compile until the format carries it.
/// Fields are encoded in declaration order.
macro_rules! struct_codec {
    ($ty:ident { $($field:ident: $fty:ty),+ $(,)? }) => {
        impl Codec for $ty {
            const MIN: usize = 0 $(+ <$fty as Codec>::MIN)+;
            fn put(&self, o: &mut Out<'_>) -> io::Result<()> {
                let $ty { $($field),+ } = self;
                $(<$fty as Codec>::put($field, o)?;)+
                Ok(())
            }
            fn get(i: &mut In<'_>, a: &Budget) -> R<Self> {
                Ok($ty { $($field: <$fty as Codec>::get(i, a)?),+ })
            }
        }
    };
}

/// Byte vectors and large numeric arrays, written as one block.
struct Bytes;
struct F64s;
struct U32s;

/// Codecs for fields that are not themselves [`Codec`] types.
trait FieldCodec<T> {
    const MIN: usize = 8;
    fn put(v: &T, o: &mut Out<'_>) -> io::Result<()>;
    fn get(i: &mut In<'_>, a: &Budget) -> R<T>;
}
impl FieldCodec<Vec<u8>> for Bytes {
    fn put(v: &Vec<u8>, o: &mut Out<'_>) -> io::Result<()> {
        o.bytes(v)
    }
    fn get(i: &mut In<'_>, a: &Budget) -> R<Vec<u8>> {
        i.bytes(a)
    }
}
impl FieldCodec<Vec<f64>> for F64s {
    fn put(v: &Vec<f64>, o: &mut Out<'_>) -> io::Result<()> {
        o.f64s(v)
    }
    fn get(i: &mut In<'_>, a: &Budget) -> R<Vec<f64>> {
        i.f64s(a)
    }
}
impl FieldCodec<Vec<u32>> for U32s {
    fn put(v: &Vec<u32>, o: &mut Out<'_>) -> io::Result<()> {
        o.u32s(v)
    }
    fn get(i: &mut In<'_>, a: &Budget) -> R<Vec<u32>> {
        i.u32s(a)
    }
}

/// Like `struct_codec!`, with a per-field codec: `field: Type` uses
/// [`Codec`], `field: Type as Codec2` uses that [`FieldCodec`].
macro_rules! record_codec {
    (@put $o:ident, $field:ident, $fty:ty, $via:ty) => { <$via as FieldCodec<$fty>>::put($field, $o)? };
    (@put $o:ident, $field:ident, $fty:ty) => { <$fty as Codec>::put($field, $o)? };
    (@get $i:ident, $a:ident, $fty:ty, $via:ty) => { <$via as FieldCodec<$fty>>::get($i, $a)? };
    (@get $i:ident, $a:ident, $fty:ty) => { <$fty as Codec>::get($i, $a)? };
    (@min $fty:ty, $via:ty) => { <$via as FieldCodec<$fty>>::MIN };
    (@min $fty:ty) => { <$fty as Codec>::MIN };
    ($ty:ident { $($field:ident: $fty:ty $(as $via:ty)?),+ $(,)? }) => {
        impl Codec for $ty {
            const MIN: usize = 0 $(+ record_codec!(@min $fty $(, $via)?))+;
            fn put(&self, o: &mut Out<'_>) -> io::Result<()> {
                let $ty { $($field),+ } = self;
                $(record_codec!(@put o, $field, $fty $(, $via)?);)+
                Ok(())
            }
            fn get(i: &mut In<'_>, a: &Budget) -> R<Self> {
                Ok($ty { $($field: record_codec!(@get i, a, $fty $(, $via)?)),+ })
            }
        }
    };
}

struct_codec!(ElementInfo {
    name: String,
    source_id: Option<i64>,
});
struct_codec!(Transform {
    translation: [f64; 3],
    rotation: [f64; 4],
    scale: [f64; 3],
});
impl Codec for Matrix {
    const MIN: usize = 96;
    fn put(&self, o: &mut Out<'_>) -> io::Result<()> {
        let Matrix(m) = self;
        m.put(o)
    }
    fn get(i: &mut In<'_>, a: &Budget) -> R<Self> {
        Ok(Matrix(<[f64; 12]>::get(i, a)?))
    }
}
impl<const N: usize> Codec for VertexAttribute<N> {
    const MIN: usize = 16;
    fn put(&self, o: &mut Out<'_>) -> io::Result<()> {
        let VertexAttribute { values, indices } = self;
        o.f64s(values)?;
        o.u32s(indices)
    }
    fn get(i: &mut In<'_>, a: &Budget) -> R<Self> {
        Ok(VertexAttribute {
            values: i.f64s(a)?,
            indices: i.u32s(a)?,
        })
    }
}
struct_codec!(Face {
    begin: u32,
    count: u32,
});
struct_codec!(UvSet {
    name: String,
    uv: VertexAttribute<2>,
    tangent: Option<VertexAttribute<3>>,
    bitangent: Option<VertexAttribute<3>>,
});
struct_codec!(ColorSet {
    name: String,
    color: VertexAttribute<4>,
});
record_codec!(MeshPart {
    index: u32,
    num_triangles: u64,
    face_indices: Vec<u32> as U32s,
});
record_codec!(Mesh {
    base: ElementInfo,
    num_vertices: usize,
    faces: Vec<Face>,
    positions: VertexAttribute<3>,
    normals: Option<VertexAttribute<3>>,
    uv_sets: Vec<UvSet>,
    color_sets: Vec<ColorSet>,
    face_material: Vec<u32> as U32s,
    materials: Vec<usize>,
    parts: Vec<MeshPart>,
    triangles: Vec<u32> as U32s,
    instances: Vec<usize>,
    reversed_winding: bool,
    generated_normals: bool,
    skin_deformers: Vec<usize>,
    blend_deformers: Vec<usize>,
});
struct_codec!(Node {
    base: ElementInfo,
    parent: Option<usize>,
    children: Vec<usize>,
    local: Transform,
    node_to_parent: Matrix,
    node_to_world: Matrix,
    geometry_to_node: Matrix,
    geometry_to_world: Matrix,
    visible: bool,
    mesh: Option<usize>,
    camera: Option<usize>,
    light: Option<usize>,
    materials: Vec<usize>,
    bone: Option<usize>,
    bind_pose: Option<usize>,
});
struct_codec!(MaterialMap {
    value: [f64; 4],
    value_int: i64,
    value_components: u64,
    texture: Option<usize>,
    has_value: bool,
    texture_enabled: bool,
    feature_disabled: bool,
});
struct_codec!(MaterialFeature {
    enabled: bool,
    is_explicit: bool,
});
record_codec!(SourceProperty {
    name: Vec<u8> as Bytes,
    property_type: u8 as Byte,
    flags: u32,
    value_int: i64,
    value_real: [f64; 4],
    value_str: Vec<u8> as Bytes,
    value_blob: Vec<u8> as Bytes,
});
record_codec!(MaterialTexture {
    material_prop: Vec<u8> as Bytes,
    shader_prop: Vec<u8> as Bytes,
    texture: usize,
});

struct Byte;
impl FieldCodec<u8> for Byte {
    const MIN: usize = 1;
    fn put(v: &u8, o: &mut Out<'_>) -> io::Result<()> {
        o.u8(*v)
    }
    fn get(i: &mut In<'_>, _: &Budget) -> R<u8> {
        i.u8()
    }
}

/// `(name, value)` lists whose names come from a fixed static table; the
/// name is stored as its `u8` index in that table.
struct Named<const TABLE: usize>;
const FBX_MAP_NAMES: usize = 0;
const PBR_MAP_NAMES: usize = 1;
const FEATURE_NAMES: usize = 2;

fn name_table(table: usize) -> &'static [&'static str] {
    match table {
        FBX_MAP_NAMES => &crate::scene_material::FBX_MAPS,
        PBR_MAP_NAMES => &crate::scene_material::PBR_MAPS,
        _ => &crate::scene_material::FEATURES,
    }
}

impl<const TABLE: usize, T: Codec> FieldCodec<Vec<(&'static str, T)>> for Named<TABLE> {
    fn put(v: &Vec<(&'static str, T)>, o: &mut Out<'_>) -> io::Result<()> {
        let names = name_table(TABLE);
        o.count(v.len())?;
        for (name, value) in v {
            let index = names
                .iter()
                .position(|n| n == name)
                .and_then(|i| u8::try_from(i).ok())
                .ok_or_else(|| invalid("material name outside the static name table"))?;
            o.u8(index)?;
            value.put(o)?;
        }
        Ok(())
    }
    fn get(i: &mut In<'_>, a: &Budget) -> R<Vec<(&'static str, T)>> {
        let names = name_table(TABLE);
        let n = i.count(1 + T::MIN)?;
        let mut v = Vec::new();
        a.reserve(&mut v, n).map_err(alloc_error)?;
        for _ in 0..n {
            let name = *names.get(usize::from(i.u8()?)).ok_or(ReadError::Corrupt)?;
            v.push((name, T::get(i, a)?));
        }
        Ok(v)
    }
}

record_codec!(Material {
    base: ElementInfo,
    shading_model: String,
    fbx: Vec<(&'static str, MaterialMap)> as Named<FBX_MAP_NAMES>,
    pbr: Vec<(&'static str, MaterialMap)> as Named<PBR_MAP_NAMES>,
    features: Vec<(&'static str, MaterialFeature)> as Named<FEATURE_NAMES>,
    properties: Vec<SourceProperty>,
    textures: Vec<MaterialTexture>,
});
struct_codec!(TextureLayer {
    texture: usize,
    blend_mode: i64,
    alpha: f64,
});
record_codec!(Texture {
    base: ElementInfo,
    layered: bool,
    layers: Vec<TextureLayer>,
    file_textures: Vec<usize>,
    filename: String,
    relative_filename: String,
    absolute_filename: String,
    content: Vec<u8> as Bytes,
    uv_set: String,
    wrap: [i64; 2],
});
struct_codec!(Camera {
    base: ElementInfo,
    projection_mode: i64,
    field_of_view_deg: [f64; 2],
    near_plane: f64,
    far_plane: f64,
    aspect_ratio: f64,
    orthographic_size: [f64; 2],
    focal_length_mm: f64,
});
struct_codec!(Light {
    base: ElementInfo,
    kind: LightKind,
    color: [f64; 3],
    intensity: f64,
    local_direction: [f64; 3],
    decay: i64,
    inner_angle: f64,
    outer_angle: f64,
    cast_light: bool,
    cast_shadows: bool,
});
struct_codec!(CoordinateAxes {
    right: Axis,
    up: Axis,
    front: Axis,
});
record_codec!(SceneSettings {
    axes: CoordinateAxes,
    unit_meters: f64,
    original_axis_up: Option<Axis>,
    original_unit_meters: f64,
    frames_per_second: f64,
    ambient_color: [f64; 3],
    version: u32,
    ascii: bool,
    creator: String,
    original_application: [String; 3] as Strings3,
    original_file_path: String,
});
/// `[String; 3]`, element by element (`String` is not `Copy`).
struct Strings3;
impl FieldCodec<[String; 3]> for Strings3 {
    const MIN: usize = 24;
    fn put(v: &[String; 3], o: &mut Out<'_>) -> io::Result<()> {
        v.iter().try_for_each(|s| s.put(o))
    }
    fn get(i: &mut In<'_>, a: &Budget) -> R<[String; 3]> {
        Ok([String::get(i, a)?, String::get(i, a)?, String::get(i, a)?])
    }
}
struct_codec!(Unsupported {
    kind: UnsupportedKind,
    source_id: i64,
});
struct_codec!(Bone {
    name: String,
    source_id: Option<i64>,
    instances: Vec<usize>,
    radius: f64,
    relative_length: f64,
    is_root: bool,
});
struct_codec!(SkinWeight {
    cluster_index: usize,
    weight: f64,
});
struct_codec!(SkinVertex {
    weight_begin: usize,
    num_weights: usize,
    dq_weight: f64,
});
record_codec!(SkinDeformer {
    name: String,
    source_id: Option<i64>,
    skinning_method: SkinningMethod,
    clusters: Vec<usize>,
    vertices: Vec<SkinVertex>,
    weights: Vec<SkinWeight>,
    max_weights_per_vertex: usize,
    dq_vertices: Vec<u32> as U32s,
    dq_weights: Vec<f64> as F64s,
});
record_codec!(SkinCluster {
    name: String,
    source_id: Option<i64>,
    bone_node: Option<usize>,
    geometry_to_bone: Matrix,
    mesh_node_to_bone: Matrix,
    bind_to_world: Matrix,
    geometry_to_world: Matrix,
    geometry_to_world_transform: Transform,
    vertices: Vec<u32> as U32s,
    weights: Vec<f64> as F64s,
});
struct_codec!(BlendDeformer {
    name: String,
    source_id: Option<i64>,
    channels: Vec<usize>,
});
struct_codec!(BlendKeyframe {
    shape: usize,
    target_weight: f64,
    effective_weight: f64,
});
struct_codec!(BlendChannel {
    name: String,
    source_id: Option<i64>,
    weight: f64,
    keyframes: Vec<BlendKeyframe>,
    target_shape: Option<usize>,
});
record_codec!(BlendShape {
    name: String,
    source_id: Option<i64>,
    offset_vertices: Vec<u32> as U32s,
    position_offsets: Vec<[f64; 3]>,
    normal_offsets: Vec<[f64; 3]>,
    offset_weights: Vec<f64> as F64s,
});
struct_codec!(BonePose {
    bone_node: usize,
    bone_to_world: Matrix,
    bone_to_parent: Matrix,
});
struct_codec!(Pose {
    name: String,
    source_id: Option<i64>,
    is_bind_pose: bool,
    bone_poses: Vec<BonePose>,
});

struct_codec!(Tangent { dx: f32, dy: f32 });
struct_codec!(Keyframe {
    ticks: i64,
    time: f64,
    value: f64,
    interpolation: Interpolation,
    left: Tangent,
    right: Tangent,
});
struct_codec!(Extrapolation {
    mode: ExtrapolationMode,
    repeat_count: i32,
});
struct_codec!(AnimationCurve {
    element: usize,
    source_id: i64,
    keys: Vec<Keyframe>,
    pre_extrapolation: Extrapolation,
    post_extrapolation: Extrapolation,
});
struct_codec!(AnimationValue {
    element: usize,
    source_id: i64,
    default_value: [f64; 3],
    curves: [Option<usize>; 3],
});
record_codec!(AnimationBinding {
    element: usize,
    source_id: i64,
    property: Vec<u8> as Bytes,
    value: usize,
});
struct_codec!(AnimationLayer {
    element: usize,
    source_id: i64,
    name: String,
    weight: f64,
    weight_is_animated: bool,
    blended: bool,
    additive: bool,
    compose_rotation: bool,
    compose_scale: bool,
    values: Vec<usize>,
    bindings: Vec<AnimationBinding>,
});
struct_codec!(AnimationStack {
    element: usize,
    source_id: i64,
    name: String,
    time_begin: f64,
    time_end: f64,
    layers: Vec<usize>,
});
record_codec!(EvaluatedProperty {
    property_type: u8 as Byte,
    flags: u32,
    value_int: i64,
    components: [f64; 4],
    value_str: Vec<u8> as Bytes,
    value_blob: Vec<u8> as Bytes,
});
struct_codec!(NodeProps {
    translation: [f64; 3],
    rotation: [f64; 3],
    scaling: [f64; 3],
    pre_rotation: [f64; 3],
    post_rotation: [f64; 3],
    rotation_offset: [f64; 3],
    rotation_pivot: [f64; 3],
    scaling_offset: [f64; 3],
    scaling_pivot: [f64; 3],
    geometric_translation: [f64; 3],
    geometric_rotation: [f64; 3],
    geometric_scaling: [f64; 3],
});
struct_codec!(Adjust {
    has_adjust_transform: bool,
    has_root_adjust_transform: bool,
    pre_translation: [f64; 3],
    pre_rotation: [f64; 4],
    pre_scale: f64,
    post_rotation: [f64; 4],
    post_scale: f64,
    translation_scale: f64,
    mirror_axis: i64,
});
struct_codec!(NodeEval {
    element: usize,
    parent: Option<usize>,
    props: NodeProps,
    adjust: Adjust,
    rotation_order: i64,
    use_rotation_space: bool,
    translation_scale: Option<usize>,
    inherited_scale: Option<usize>,
    is_scale_helper: bool,
    own_helper: Option<usize>,
    inherit_helper: Option<usize>,
});

/// Map key `(element, property name)`.
fn put_key(o: &mut Out<'_>, (element, name): &(usize, Vec<u8>)) -> io::Result<()> {
    element.put(o)?;
    o.bytes(name)
}
fn get_key(i: &mut In<'_>, a: &Budget) -> R<(usize, Vec<u8>)> {
    Ok((i.usize()?, i.bytes(a)?))
}

/// An ordered map as its entries in ascending key order. Reading requires
/// strictly ascending keys, so a valid sidecar has one encoding.
fn put_map<V>(
    o: &mut Out<'_>,
    map: &Map<(usize, Vec<u8>), V>,
    put: impl Fn(&mut Out<'_>, &V) -> io::Result<()>,
) -> io::Result<()> {
    o.count(map.len())?;
    for (key, value) in map.iter() {
        put_key(o, key)?;
        put(o, value)?;
    }
    Ok(())
}
fn get_map<'a, V>(
    i: &mut In<'a>,
    a: &Budget,
    min: usize,
    get: impl Fn(&mut In<'a>, &Budget) -> R<V>,
) -> R<Map<(usize, Vec<u8>), V>> {
    let n = i.count(16 + min)?;
    let mut map = Map::new();
    let mut previous: Option<(usize, &'a [u8])> = None;
    for _ in 0..n {
        let element = i.usize()?;
        let len = i.count(1)?;
        let name = i.take(len)?;
        if previous.is_some_and(|p| p >= (element, name)) {
            return Err(ReadError::Corrupt);
        }
        previous = Some((element, name));
        let mut owned = Vec::new();
        a.reserve(&mut owned, len).map_err(alloc_error)?;
        owned.extend_from_slice(name);
        let value = get(i, a)?;
        map.try_insert((element, owned), value, a)
            .map_err(alloc_error)?;
    }
    Ok(map)
}

fn put_animation(o: &mut Out<'_>, animation: &Animation) -> io::Result<()> {
    let Animation {
        allocation_limit,
        curves,
        values,
        layers,
        stacks,
        default_stack,
        properties,
        nodes,
        node_order,
        creation,
        ids,
        channels,
        clusters,
        connections,
    } = animation;
    allocation_limit.put(o)?;
    curves.put(o)?;
    values.put(o)?;
    layers.put(o)?;
    stacks.put(o)?;
    default_stack.put(o)?;
    put_map(o, properties, |o, v| v.put(o))?;
    nodes.put(o)?;
    node_order.put(o)?;
    creation.put(o)?;
    ids.put(o)?;
    channels.put(o)?;
    clusters.put(o)?;
    put_map(o, connections, |o, (element, name)| {
        element.put(o)?;
        o.bytes(name)
    })
}

fn get_animation(i: &mut In<'_>, a: &Budget) -> R<Animation> {
    Ok(Animation {
        allocation_limit: Codec::get(i, a)?,
        curves: Codec::get(i, a)?,
        values: Codec::get(i, a)?,
        layers: Codec::get(i, a)?,
        stacks: Codec::get(i, a)?,
        default_stack: Codec::get(i, a)?,
        properties: get_map(i, a, EvaluatedProperty::MIN, EvaluatedProperty::get)?,
        nodes: Codec::get(i, a)?,
        node_order: Codec::get(i, a)?,
        creation: Codec::get(i, a)?,
        ids: Codec::get(i, a)?,
        channels: Codec::get(i, a)?,
        clusters: Codec::get(i, a)?,
        connections: get_map(i, a, 16, get_key)?,
    })
}
