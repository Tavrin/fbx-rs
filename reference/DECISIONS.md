# Oracle decisions

- Use exactly two qualification profiles: historical C 0.11.0 raw and the C
  0.23.1 bundled in Rust ufbx 0.11.5 with the engine import profile's
  conversion options. A raw 0.23.1 profile, any engine-side projection and
  performance are outside the oracle.
- Generate field walkers/coverage from verified local headers using pinned
  pycparser 2.23. A new reachable type is a build error; schema/options changes
  require explicit regeneration and review. Third-party source is build data.
- Choose complete union storage representations explicitly and compile size
  assertions against every alias. Material maps and shader texture inputs put
  their scalar view first; dumping only that view loses RGB/vector components.
  A self-authored scene exercises nonfirst colour components and prefix names.
- A legacy source Model/Take can own several typed representations. Give the
  primary representation the source key and identify additional attachments or
  layers by owner/role/ordinal. Check identity uniqueness and reference closure;
  repeat determinism alone cannot reveal an aliasing identity error.
- Legacy identity names use the raw DOM blob bytes when present, preserving
  names that the oracle's UTF-8 string view sanitizes. Preserve FileId in the
  exact DOM stream; only the declared creation timestamp is omitted there.
- Sort evaluated property lookup rows and skin/morph sample records by the same
  source identities as typed records. Preserve animation-layer sequence. Oracle
  dense allocation order is not a canonical lookup order.
- Keep all asset bytes, per-input provenance, canonical dumps, diagnostics,
  mutations and receipts under scratch. Commit no corpus fixture. The public
  repository manifest admits only named source files and no data fixtures.
- Include the whole prior public corpus and five pinned embedded/external-media
  regressions, after checking asset bytes against the pinned Git tree and licence
  bytes against the prior retained root grant. Reject private/nonredistributable
  records before running. This does not admit upstream benchmark datasets.
- Retain every parsed projection exactly. Expose oracle DOM folding/sanitization
  boundaries instead of inventing original scalar tags. Authored Euler angles
  and reconstructed curve times/tangents never receive tolerance.
- Compare only explicitly declared evaluated f64/quaternion paths tolerantly.
  Reject all evaluated nonfinite values because no fixture exception is admitted.
  The final-f32 tolerance remains disabled until there is an engine projection.
- Bound the oracle and its supervisor separately. On timeout kill the entire
  process group, adopt descendants as a Linux subreaper, and reap them. Retain
  failed receipts and never count a timeout/partial dump as repeat identity.
- Run CPU-only. Keep process address space at 3.5 GiB; preserve the exact
  import profile allocator options and classify outer budget failures as
  unverified inputs. Queue delays on a shared machine are not oracle failures
  or timings.
- Prove semantic mutation detection with recomputed checksums on real dumps for
  all six domains, in both profiles, plus changed counts. Integrity-only mutation
  failures are insufficient. Also test truncation, forged tolerance, profile
  mixing, pinned option changes, and hygiene rejection with self-authored streams.
