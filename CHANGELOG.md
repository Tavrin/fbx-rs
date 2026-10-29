# Changelog

All notable changes to this crate are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the crate uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `load` and `load_with_limits`: read a binary or ASCII FBX 7.1 to 7.7 file
  into an owned static `Scene` (nodes and transforms, meshes with UV and
  color sets and triangulation, materials, textures, cameras, lights,
  settings), converted to right-handed Y-up with 1 m units.
- `Scene::unsupported` lists skins, blend shapes, animation and other content
  that is not converted.
- `syntax::parse`: bounded binary and ASCII FBX 7.x parser returning the
  node/property tree. FBX 6.1 and older are rejected.
- `Limits` on input size, nesting, node, property, array and allocation
  counts for untrusted input.
