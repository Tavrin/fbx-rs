use fbx_rs::syntax::{parse, Error, Limits, Property};

// Self-generated records: no corpus assets are embedded.
fn fixture(version: u32, be: bool, prop: &[u8]) -> Vec<u8> {
    fn num(out: &mut Vec<u8>, n: u64, width: usize, be: bool) {
        let bytes = if be { n.to_be_bytes() } else { n.to_le_bytes() };
        if be {
            out.extend_from_slice(&bytes[8 - width..]);
        } else {
            out.extend_from_slice(&bytes[..width]);
        }
    }
    let width = if version >= 7500 { 8 } else { 4 };
    let mut out = b"Kaydara FBX Binary  \x00\x1a".to_vec();
    out.push(u8::from(be));
    num(&mut out, u64::from(version), 4, be);
    let end = 27 + width * 3 + 1 + 1 + prop.len();
    num(&mut out, end as u64, width, be);
    num(&mut out, u64::from(!prop.is_empty()), width, be);
    num(&mut out, prop.len() as u64, width, be);
    out.push(1);
    out.push(b'N');
    out.extend_from_slice(prop);
    out.resize(out.len() + width * 3 + 1, 0);
    out
}
#[test]
fn headers_and_scalar_bits() {
    for version in [7400, 7500] {
        for be in [false, true] {
            let bits = 0x8000_0000u32;
            let mut p = vec![b'F'];
            let encoded = if be {
                bits.to_be_bytes()
            } else {
                bits.to_le_bytes()
            };
            p.extend_from_slice(&encoded);
            let input = fixture(version, be, &p);
            let doc = parse(&input, &Limits::default()).unwrap();
            assert_eq!(doc.version, version);
            assert_eq!(doc.nodes[0].name, b"N");
            assert_eq!(doc.properties[0], Property::F32(bits));
            for cut in 0..input.len() {
                assert!(
                    parse(&input[..cut], &Limits::default()).is_err(),
                    "{version} {be} {cut}"
                );
            }
        }
    }
}
#[test]
fn limits_and_versions() {
    let input = fixture(7400, false, &[b'I', 1, 0, 0, 0]);
    let mut l = Limits {
        max_nodes: 0,
        ..Limits::default()
    };
    assert_eq!(
        parse(&input, &l).unwrap_err(),
        Error::LimitExceeded { what: "nodes" }
    );
    l.max_nodes = 1;
    l.max_properties = 0;
    assert_eq!(
        parse(&input, &l).unwrap_err(),
        Error::LimitExceeded { what: "properties" }
    );
    l.max_properties = 1;
    l.max_depth = 0;
    assert_eq!(parse(&input, &l).unwrap_err(), Error::TooDeep);
    l.max_depth = 1;
    l.max_string_length = 0;
    assert_eq!(
        parse(&input, &l).unwrap_err(),
        Error::LimitExceeded {
            what: "string length"
        }
    );
    for v in [3000, 5700, 5799, 7000, 7099, 7701, 7800] {
        assert_eq!(
            parse(&fixture(v, false, &[]), &Limits::default()).unwrap_err(),
            Error::UnsupportedVersion { v }
        );
    }
}

/// A node record for the nested little-endian builder below.
struct Rec {
    name: &'static [u8],
    props: Vec<Vec<u8>>,
    children: Vec<Rec>,
}

fn rec(name: &'static [u8], props: Vec<Vec<u8>>, children: Vec<Rec>) -> Rec {
    Rec {
        name,
        props,
        children,
    }
}

fn write_nodes(out: &mut Vec<u8>, width: usize, recs: &[Rec]) {
    for r in recs {
        let start = out.len();
        out.resize(start + 3 * width, 0);
        out.push(r.name.len() as u8);
        out.extend_from_slice(r.name);
        let props_start = out.len();
        for p in &r.props {
            out.extend_from_slice(p);
        }
        let props_len = out.len() - props_start;
        if !r.children.is_empty() {
            write_nodes(out, width, &r.children);
        }
        let end = out.len();
        for (i, n) in [end, r.props.len(), props_len].into_iter().enumerate() {
            let at = start + i * width;
            out[at..at + width].copy_from_slice(&(n as u64).to_le_bytes()[..width]);
        }
    }
    out.resize(out.len() + 3 * width + 1, 0);
}

/// Nested document: two top-level nodes, a child with a zlib `i` array, an
/// empty child that ends in a null record, strings and scalars.
fn nested(version: u32) -> Vec<u8> {
    let zlib = [
        0x78, 0x9c, 0x63, 0x64, 0x60, 0x60, 0x60, 0x02, 0x62, 0x00, 0x00, 0x18, 0x00, 0x04,
    ];
    let mut array = vec![b'i'];
    for n in [2u32, 1, zlib.len() as u32] {
        array.extend(n.to_le_bytes());
    }
    array.extend(zlib);
    let tree = vec![
        rec(
            b"Root",
            vec![b"S\x05\0\0\0hello".to_vec(), vec![b'I', 7, 0, 0, 0]],
            vec![
                rec(b"Data", vec![array], vec![]),
                rec(
                    b"Empty",
                    vec![],
                    vec![rec(b"Leaf", vec![vec![b'C', 1]], vec![])],
                ),
            ],
        ),
        rec(
            b"Tail",
            vec![vec![b'D', 0, 0, 0, 0, 0, 0, 0xf0, 0x3f]],
            vec![],
        ),
    ];
    let width = if version >= 7500 { 8 } else { 4 };
    let mut out = b"Kaydara FBX Binary  \x00\x1a\x00".to_vec();
    out.extend(version.to_le_bytes());
    write_nodes(&mut out, width, &tree);
    out
}

#[test]
fn all_7x_versions_and_header_widths() {
    for version in [5800, 6000, 6100, 7100, 7200, 7300, 7400, 7500, 7600, 7700] {
        let input = nested(version);
        let doc = parse(&input, &Limits::default()).unwrap();
        assert_eq!(doc.version, version);
        let names: Vec<&[u8]> = doc.nodes.iter().map(|n| n.name).collect();
        assert_eq!(
            names,
            [&b"Root"[..], b"Data", b"Empty", b"Leaf", b"Tail"],
            "{version}"
        );
        let parents: Vec<Option<usize>> = doc.nodes.iter().map(|n| n.parent).collect();
        assert_eq!(parents, [None, Some(0), Some(0), Some(2), None]);
    }
    // A 7.4 file read with 7.5 header widths (and the reverse) is rejected.
    let mut wrong = nested(7400);
    wrong[23..27].copy_from_slice(&7500u32.to_le_bytes());
    assert!(parse(&wrong, &Limits::default()).is_err());
    let mut wrong = nested(7500);
    wrong[23..27].copy_from_slice(&7400u32.to_le_bytes());
    assert!(parse(&wrong, &Limits::default()).is_err());
}

#[test]
fn truncation_at_every_byte_of_a_nested_file() {
    // Every proper prefix cuts some record (node header, name, property,
    // array payload, child list or null record) and must be rejected.
    for version in [7400, 7500] {
        let input = nested(version);
        assert!(parse(&input, &Limits::default()).is_ok());
        for cut in 0..input.len() {
            let err = parse(&input[..cut], &Limits::default()).unwrap_err();
            assert!(
                matches!(err, Error::Truncated | Error::InvalidFormat),
                "{version} cut {cut}: {err:?}"
            );
        }
    }
}

#[test]
fn every_limit_at_its_boundary_on_a_nested_file() {
    // Measured values of `nested`: 5 nodes, 5 properties, depth 3, longest
    // string 5 bytes, 2 array elements, 8 decompressed bytes.
    let input = nested(7500);
    let doc = parse(&input, &Limits::default()).unwrap();
    assert_eq!((doc.nodes.len(), doc.properties.len()), (5, 5));
    let cases: [(&str, usize, Error); 6] = [
        ("depth", 3, Error::TooDeep),
        ("nodes", 5, Error::LimitExceeded { what: "nodes" }),
        ("properties", 5, Error::LimitExceeded { what: "properties" }),
        (
            "string",
            5,
            Error::LimitExceeded {
                what: "string length",
            },
        ),
        (
            "elements",
            2,
            Error::LimitExceeded {
                what: "array elements",
            },
        ),
        (
            "decompressed",
            8,
            Error::LimitExceeded {
                what: "decompressed bytes",
            },
        ),
    ];
    for (field, boundary, error) in cases {
        for n in [boundary - 1, boundary, boundary + 1] {
            let mut l = Limits::default();
            match field {
                "depth" => l.max_depth = n,
                "nodes" => l.max_nodes = n,
                "properties" => l.max_properties = n,
                "string" => l.max_string_length = n,
                "elements" => l.max_array_elements = n,
                _ => l.max_decompressed_bytes = n,
            }
            let result = parse(&input, &l);
            if n < boundary {
                assert_eq!(result.unwrap_err(), error, "{field} {n}");
            } else {
                assert!(result.is_ok(), "{field} {n}");
            }
        }
    }
    let allocation = |n| {
        parse(
            &input,
            &Limits {
                max_allocation: n,
                ..Limits::default()
            },
        )
    };
    let threshold = (0..1 << 16).find(|&n| allocation(n).is_ok()).unwrap();
    assert_eq!(
        allocation(threshold - 1).unwrap_err(),
        Error::LimitExceeded { what: "allocation" }
    );
    assert!(allocation(threshold + 1).is_ok());
}
#[test]
fn raw_and_compressed_arrays() {
    // zlib stream for little-endian [1, 2].
    let raw = [1, 0, 0, 0, 2, 0, 0, 0];
    let packed = [
        0x78, 0x9c, 0x63, 0x64, 0x60, 0x60, 0x60, 0x02, 0x62, 0x00, 0x00, 0x18, 0x00, 0x04,
    ];
    for (encoding, data) in [(0u32, raw.as_slice()), (1, packed.as_slice())] {
        let mut p = vec![b'i'];
        p.extend(2u32.to_le_bytes());
        p.extend(encoding.to_le_bytes());
        p.extend((data.len() as u32).to_le_bytes());
        p.extend(data);
        let input = fixture(7400, false, &p);
        let doc = parse(&input, &Limits::default()).unwrap();
        match &doc.properties[0] {
            Property::Array { bytes, .. } => assert_eq!(bytes.as_ref(), raw),
            _ => panic!("array expected"),
        }
        let l = Limits {
            max_array_elements: 1,
            ..Limits::default()
        };
        assert_eq!(
            parse(&input, &l).unwrap_err(),
            Error::LimitExceeded {
                what: "array elements"
            }
        );
    }
}

#[test]
fn limit_boundaries_and_corrupt_zlib() {
    let data = [
        0x78, 0x9c, 0x63, 0x64, 0x60, 0x60, 0x60, 0x02, 0x62, 0x00, 0x00, 0x18, 0x00, 0x04,
    ];
    let mut p = vec![b'i'];
    p.extend(2u32.to_le_bytes());
    p.extend(1u32.to_le_bytes());
    p.extend((data.len() as u32).to_le_bytes());
    p.extend(data);
    let input = fixture(7400, false, &p);
    for (n, success) in [(1, false), (2, true), (3, true)] {
        let l = Limits {
            max_array_elements: n,
            ..Limits::default()
        };
        assert_eq!(parse(&input, &l).is_ok(), success);
    }
    for (n, success) in [(7, false), (8, true), (9, true)] {
        let l = Limits {
            max_decompressed_bytes: n,
            ..Limits::default()
        };
        assert_eq!(parse(&input, &l).is_ok(), success);
    }
    for (n, success) in [(0, false), (1, true), (2, true)] {
        for field in 0..4 {
            let mut l = Limits::default();
            match field {
                0 => l.max_nodes = n,
                1 => l.max_properties = n,
                2 => l.max_depth = n,
                _ => l.max_string_length = n,
            }
            assert_eq!(
                parse(&input, &l).is_ok(),
                success,
                "field {field} boundary {n}"
            );
        }
    }
    let threshold = (0..10000)
        .find(|&n| {
            parse(
                &input,
                &Limits {
                    max_allocation: n,
                    ..Limits::default()
                },
            )
            .is_ok()
        })
        .unwrap();
    assert!(parse(
        &input,
        &Limits {
            max_allocation: threshold - 1,
            ..Limits::default()
        }
    )
    .is_err());
    assert!(parse(
        &input,
        &Limits {
            max_allocation: threshold + 1,
            ..Limits::default()
        }
    )
    .is_ok());
    let last = p.len() - 1;
    p[last] ^= 1;
    assert_eq!(
        parse(&fixture(7400, false, &p), &Limits::default()).unwrap_err(),
        Error::BadCompression
    );
}

#[test]
fn preserves_alternate_boolean_type() {
    let input = fixture(7400, false, &[b'B', 1]);
    let doc = parse(&input, &Limits::default()).unwrap();
    assert_eq!(doc.properties[0], Property::BoolAlt(1));
}

#[test]
fn compressed_empty_array() {
    let data = [0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01];
    let mut p = vec![b'i'];
    p.extend(0u32.to_le_bytes());
    p.extend(1u32.to_le_bytes());
    p.extend((data.len() as u32).to_le_bytes());
    p.extend(data);
    let input = fixture(7400, false, &p);
    let doc = parse(&input, &Limits::default()).unwrap();
    assert!(
        matches!(&doc.properties[0], Property::Array { count: 0, bytes, .. } if bytes.is_empty())
    );
}

#[test]
fn partial_binary_magic_is_truncated() {
    let magic = b"Kaydara FBX Binary  \x00\x1a";
    for cut in 0..magic.len() {
        assert_eq!(
            parse(&magic[..cut], &Limits::default()).unwrap_err(),
            Error::Truncated
        );
    }
}

#[test]
fn arrays_in_both_header_widths_and_byte_orders() {
    for version in [7400, 7500] {
        for be in [false, true] {
            let raw: &[u8] = if be {
                &[0, 0, 0, 1, 0, 0, 0, 2]
            } else {
                &[1, 0, 0, 0, 2, 0, 0, 0]
            };
            let compressed: &[u8] = if be {
                &[
                    0x78, 0x9c, 0x63, 0x60, 0x60, 0x60, 0x64, 0x60, 0x60, 0x60, 0x02, 0x00, 0x00,
                    0x0f, 0x00, 0x04,
                ]
            } else {
                &[
                    0x78, 0x9c, 0x63, 0x64, 0x60, 0x60, 0x60, 0x02, 0x62, 0x00, 0x00, 0x18, 0x00,
                    0x04,
                ]
            };
            for (encoding, data) in [(0u32, raw), (1, compressed)] {
                let mut prop = vec![b'i'];
                for value in [2u32, encoding, data.len() as u32] {
                    let bytes = if be {
                        value.to_be_bytes()
                    } else {
                        value.to_le_bytes()
                    };
                    prop.extend(bytes);
                }
                prop.extend(data);
                let input = fixture(version, be, &prop);
                let doc = parse(&input, &Limits::default()).unwrap();
                match &doc.properties[0] {
                    Property::Array {
                        count,
                        big_endian,
                        bytes,
                        ..
                    } => {
                        assert_eq!(*count, 2);
                        assert_eq!(*big_endian, be);
                        assert_eq!(bytes.as_ref(), raw);
                    }
                    _ => panic!("array expected"),
                }
            }
        }
    }
}

#[test]
fn every_scalar_type_and_byte_payload_round_trips() {
    use std::borrow::Cow;
    let cases: Vec<(Vec<u8>, Property<'_>)> = vec![
        (vec![b'Y', 0xfe, 0xff], Property::I16(-2)),
        (vec![b'C', 1], Property::Bool(1)),
        (vec![b'B', 0xff], Property::BoolAlt(0xff)),
        (vec![b'I', 0xfe, 0xff, 0xff, 0xff], Property::I32(-2)),
        (
            vec![b'L', 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
            Property::I64(-2),
        ),
        (vec![b'F', 1, 0, 0xc0, 0x7f], Property::F32(0x7fc00001)),
        (
            vec![b'D', 1, 0, 0, 0, 0, 0, 0xf8, 0x7f],
            Property::F64(0x7ff8000000000001),
        ),
        (
            vec![b'S', 3, 0, 0, 0, b'a', 0, b'b'],
            Property::String(Cow::Borrowed(b"a\0b")),
        ),
        (
            vec![b'R', 3, 0, 0, 0, 0, 0xff, 1],
            Property::Raw(&[0, 0xff, 1]),
        ),
    ];
    for (property, expected) in cases {
        let input = fixture(7400, false, &property);
        let doc = parse(&input, &Limits::default()).unwrap();
        assert_eq!(doc.properties[0], expected);
    }
}
