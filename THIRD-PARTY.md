# Third-party notices

## ufbx

fbx-rs is not a clean-room implementation. Its scene conversion reproduces
ufbx 0.23.1 (<https://github.com/ufbx/ufbx>, file `ufbx.c`) operation by
operation, so that results match ufbx bit for bit. The ported code cites the
ufbx function it follows (`ufbxi_...` names) and lives in these files of
`src/`:

- `canon_dom.rs`: retained-DOM projection rules.
- `canon_scene.rs`: element reading, property defaults, scene settings,
  animation-stack structure.
- `scene_attribute.rs`: camera, light and bone updates.
- `scene_geometry.rs`, `scene_mesh.rs`, `scene_surface.rs`: mesh conversion,
  attribute layers and triangulation.
- `scene_material.rs`: material and texture conversion, shader maps,
  relative file names.
- `scene_node.rs`, `scene_transform.rs`, `scene_property.rs`: transforms,
  inheritance, axis and unit conversion, property lookup.
- `canon_schema.rs` is generated from `ufbx.h` (C 0.23.1) and
  `import_profile.rs` holds ufbx load-option values; both are used only by
  the unstable `parity` feature.

The FBX syntax parser (`syntax.rs`, `syntax/ascii.rs`) is written from the
file format and checked against ufbx's retained DOM; it does not port ufbx
code.

ufbx is available under MIT or the Unlicense; fbx-rs uses it under the MIT
licence:

```text
MIT License
Copyright (c) 2020 Samuli Raivio
Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies
of the Software, and to permit persons to whom the Software is furnished to do
so, subject to the following conditions:
The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.
THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

The full ufbx licence file (both alternatives) is kept next to the reference
harness in `reference/LICENSE-ufbx`; the reference harness is not part of the
published crate.

## Dependencies

`miniz_oxide` (MIT OR Zlib OR Apache-2.0) and its dependency `adler2`
(0BSD OR MIT OR Apache-2.0) are regular Cargo dependencies and are not
vendored.
