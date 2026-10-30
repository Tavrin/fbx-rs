use fbx_rs::syntax::{parse, AsciiValue, Error, Limits, Property};
use std::borrow::Cow;

#[test]
fn scalar_nodes_and_entities() {
    let input = b"; fake FBXVersion: 6100\nFBXHeaderExtension: {\n FBXVersion: 7500\n}\nThing: -2, -0.0, \"a&quot;b\\c\"\n";
    let doc = parse(input, &Limits::default()).unwrap();
    assert_eq!(doc.version, 7500);
    assert_eq!(doc.nodes[2].name, b"Thing");
    assert_eq!(doc.properties[1], Property::I64(-2));
    assert_eq!(doc.properties[2], Property::F64((-0.0f64).to_bits()));
    assert_eq!(
        doc.properties[3],
        Property::String(Cow::Borrowed(b"a\"b\\c"))
    );
}
#[test]
fn array_and_unsupported_version() {
    let input = b"FBXHeaderExtension: { FBXVersion: 7400\n}\nData: *2 { a: 1,2 }\n";
    let doc = parse(input, &Limits::default()).unwrap();
    assert_eq!(
        doc.properties[1],
        Property::AsciiArray(vec![AsciiValue::I64(1), AsciiValue::I64(2)])
    );
    let limits = Limits {
        max_array_elements: 1,
        ..Limits::default()
    };
    assert_eq!(
        parse(input, &limits).unwrap_err(),
        Error::LimitExceeded {
            what: "array elements"
        }
    );
    let input = b"FBXHeaderExtension: { FBXVersion: 7300\n}\n";
    assert_eq!(parse(input, &Limits::default()).unwrap().version, 7300);
    let input = b"FBXHeaderExtension: { FBXVersion: 6100\n}\n";
    assert_eq!(
        parse(input, &Limits::default()).unwrap_err(),
        Error::UnsupportedVersion { v: 6100 }
    );
}

#[test]
fn magic_comment_version_takes_precedence() {
    let input = b"; FBX 7.3.0 project file\nFBXHeaderExtension: { FBXVersion: 7500\n}\n";
    assert_eq!(parse(input, &Limits::default()).unwrap().version, 7300);
    let input = b"\n;FBX\t7.7.0 project file\nA: 1\n";
    assert_eq!(parse(input, &Limits::default()).unwrap().version, 7700);
    let input = b"; FBX 6.1.0 project file\nFBXHeaderExtension: { FBXVersion: 7500\n}\n";
    assert_eq!(
        parse(input, &Limits::default()).unwrap_err(),
        Error::UnsupportedVersion { v: 6100 }
    );
}

#[test]
fn special_float_tokens() {
    let input = b"; FBX 7.4.0 project file\nV: 1.#INF, -1.#INF, -1.#IND, 1.#NAN\nW: 1#INF\n";
    let limits = Limits::default();
    assert_eq!(parse(input, &limits).unwrap_err(), Error::UnsupportedSyntax);
    let doc = parse(&input[..input.len() - 9], &limits).unwrap();
    let bits = |x: f64| Property::F64(x.to_bits());
    assert_eq!(
        doc.properties,
        [
            bits(f64::INFINITY),
            bits(f64::NEG_INFINITY),
            bits(f64::from_bits(0xfff8000000000000)),
            bits(f64::NAN)
        ]
    );
}

/// Nested ASCII document: 6 nodes, 7 properties (one array), depth 3,
/// longest string or token 5 bytes, 3 array elements.
const NESTED: &[u8] = b"; FBX 7.4.0 project file\n\
Root: \"hello\", 7 {\n\
\tData: *3 {\n\t\ta: 1,2,3\n\t}\n\
\tEmpty:  {\n\t\tLeaf: T\n\t}\n\
}\n\
Tail: 1.5\n\
Last: \"x\", \"y\"\n";

#[test]
fn every_limit_at_its_boundary_on_a_nested_file() {
    let doc = parse(NESTED, &Limits::default()).unwrap();
    assert_eq!((doc.nodes.len(), doc.properties.len()), (6, 7));
    // max_decompressed_bytes does not apply to ASCII (no compressed arrays).
    let cases: [(&str, usize, Error); 5] = [
        ("depth", 3, Error::TooDeep),
        ("nodes", 6, Error::LimitExceeded { what: "nodes" }),
        ("properties", 7, Error::LimitExceeded { what: "properties" }),
        (
            "string",
            5,
            Error::LimitExceeded {
                what: "string length",
            },
        ),
        (
            "elements",
            3,
            Error::LimitExceeded {
                what: "array elements",
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
                _ => l.max_array_elements = n,
            }
            let result = parse(NESTED, &l);
            if n < boundary {
                assert_eq!(result.unwrap_err(), error, "{field} {n}");
            } else {
                assert!(result.is_ok(), "{field} {n}");
            }
        }
    }
    let allocation = |n| {
        parse(
            NESTED,
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
fn truncation_at_every_record_boundary() {
    // ASCII has no length prefixes: a cut inside an open `{` must report
    // Truncated; a cut between top-level nodes is a shorter valid document.
    let full = parse(NESTED, &Limits::default()).unwrap();
    let names: Vec<&[u8]> = full.nodes.iter().map(|n| n.name).collect();
    for cut in 0..NESTED.len() {
        let result = parse(&NESTED[..cut], &Limits::default());
        if NESTED[cut] != b'\n' {
            continue; // every byte is exercised for panics; lines are checked
        }
        let open = NESTED[..cut].iter().filter(|&&b| b == b'{').count()
            > NESTED[..cut].iter().filter(|&&b| b == b'}').count();
        match result {
            Ok(doc) => {
                assert!(!open, "cut {cut} inside an open node parsed");
                let got: Vec<&[u8]> = doc.nodes.iter().map(|n| n.name).collect();
                assert_eq!(got, names[..got.len()], "cut {cut}");
            }
            Err(e) => assert!(
                open && e == Error::Truncated || cut <= 24 && e == Error::InvalidFormat,
                "cut {cut}: {e:?}"
            ),
        }
    }
}
#[test]
fn arrays_require_declared_count_and_preserve_zero_bits() {
    let input = b"FBXHeaderExtension: { FBXVersion: 7500\n}\nData: *3 { a: 1,\n -0.0, 3.5 }\n";
    let doc = parse(input, &Limits::default()).unwrap();
    assert_eq!(
        doc.properties[1],
        Property::AsciiArray(vec![
            AsciiValue::I64(1),
            AsciiValue::F64((-0.0f64).to_bits()),
            AsciiValue::F64(3.5f64.to_bits())
        ])
    );
    let bad = b"FBXHeaderExtension: { FBXVersion: 7500\n}\nData: *2 { a: 1 }\n";
    assert_eq!(
        parse(bad, &Limits::default()).unwrap_err(),
        Error::InvalidFormat
    );
    for (n, valid) in [(2, false), (3, true), (4, true)] {
        let limits = Limits {
            max_array_elements: n,
            ..Limits::default()
        };
        assert_eq!(parse(input, &limits).is_ok(), valid);
    }
}

#[test]
fn preserves_bare_words() {
    let input = b"FBXHeaderExtension: { FBXVersion: 7500\n}\nShading: T\nFlag: NaN\n";
    let doc = parse(input, &Limits::default()).unwrap();
    assert_eq!(doc.properties[1], Property::AsciiBare(b"T"));
    assert_eq!(doc.properties[2], Property::AsciiBare(b"NaN"));
}

#[test]
fn leading_comma_before_embedded_content() {
    let input = b"FBXHeaderExtension: { FBXVersion: 7500\n}\nContent: ,\n \"QUJD\"\n";
    let doc = parse(input, &Limits::default()).unwrap();
    assert_eq!(doc.properties[1], Property::String(Cow::Borrowed(b"QUJD")));
}

#[test]
fn overflowing_integer_is_not_silently_retyped_as_float() {
    for body in [
        "Value: 9223372036854775808",
        "Value: *1 { a: 9223372036854775808 }",
    ] {
        let input = format!("FBXHeaderExtension: {{ FBXVersion: 7500\n}}\n{body}\n");
        assert_eq!(
            parse(input.as_bytes(), &Limits::default()).unwrap_err(),
            Error::InvalidFormat
        );
    }
}

#[test]
fn preserves_bare_words_inside_arrays() {
    let input = b"FBXHeaderExtension: { FBXVersion: 7500\n}\nFlags: *2 { a: T,F }\n";
    let doc = parse(input, &Limits::default()).unwrap();
    assert_eq!(
        doc.properties[1],
        Property::AsciiArray(vec![AsciiValue::Bare(b"T"), AsciiValue::Bare(b"F")])
    );
}

#[test]
fn preserves_negative_zero_integer_tokens() {
    let input = b"FBXHeaderExtension: { FBXVersion: 7500\n}\nValue: -0\nData: *2 { a: -0,0 }\n";
    let doc = parse(input, &Limits::default()).unwrap();
    assert_eq!(doc.properties[1], Property::AsciiNegativeZero);
    assert_eq!(
        doc.properties[2],
        Property::AsciiArray(vec![AsciiValue::NegativeZero, AsciiValue::I64(0)])
    );
}

#[test]
fn pinned_special_number_spellings_and_nan_sign() {
    let cases = [
        ("+inf", 0x7ff0000000000000),
        ("-INF", 0xfff0000000000000),
        ("+Infinity", 0x7ff0000000000000),
        ("-nan", 0xfff8000000000000),
        ("+NaN(payload123)", 0x7ff8000000000000),
        ("1.#INF", 0x7ff0000000000000),
        ("-1.#IND", 0xfff8000000000000),
        ("9.#nAn000", 0x7ff8000000000000),
        ("+0.#iNf00", 0x7ff0000000000000),
    ];
    for (token, bits) in cases {
        let input = format!("; FBX 7.4.0\nV: {token}\nA: *1 {{ a: {token} }}\n");
        let doc = parse(input.as_bytes(), &Limits::default()).unwrap();
        assert_eq!(doc.properties[0], Property::F64(bits), "{token}");
        assert_eq!(
            doc.properties[1],
            Property::AsciiArray(vec![AsciiValue::F64(bits)]),
            "{token}"
        );
    }
    for token in [
        "1.#QNAN",
        "1.#SNAN",
        "1.#BANANA",
        "1.#INFjunk",
        "12.#INF",
        "1#INF",
        "+infinite",
        "+nan(bad_payload)",
    ] {
        for body in [format!("V: {token}"), format!("A: *1 {{ a: {token} }}")] {
            let input = format!("; FBX 7.4.0\n{body}\n");
            assert!(
                parse(input.as_bytes(), &Limits::default()).is_err(),
                "{token}"
            );
        }
    }
}
