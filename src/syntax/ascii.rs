//! Experimental ASCII parser; array values retain source numeric classes.
use super::{allocation_error, push, AsciiValue, Document, Error, Limits, Node, Property};
use std::borrow::Cow;

struct Cursor<'a> {
    input: &'a [u8],
    pos: usize,
}
impl<'a> Cursor<'a> {
    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }
    fn horizontal(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r')) {
            self.pos += 1;
        }
    }
    fn whitespace(&mut self) {
        loop {
            while self.peek().is_some_and(|b| b.is_ascii_whitespace()) {
                self.pos += 1;
            }
            if self.peek() != Some(b';') {
                break;
            }
            while !matches!(self.peek(), None | Some(b'\n')) {
                self.pos += 1;
            }
        }
    }
    fn string(&mut self, limits: &Limits, total: &mut usize) -> Result<Cow<'a, [u8]>, Error> {
        self.pos += 1;
        let start = self.pos;
        while !matches!(self.peek(), None | Some(b'"')) {
            self.pos += 1;
        }
        let raw = self.input.get(start..self.pos).ok_or(Error::Truncated)?;
        if self.peek().is_none() {
            return Err(Error::Truncated);
        }
        self.pos += 1;
        if raw.len() > limits.max_string_length {
            return Err(Error::LimitExceeded {
                what: "string length",
            });
        }
        if !raw.contains(&b'&') {
            return Ok(Cow::Borrowed(raw));
        }
        let mut out = Vec::new();
        crate::allocation::reserve(&mut out, raw.len(), total, limits.max_allocation)
            .map_err(allocation_error)?;
        let mut i = 0;
        while i < raw.len() {
            let rest = &raw[i..];
            let entity = [
                (b"&quot;".as_slice(), b'"'),
                (b"&cr;".as_slice(), b'\r'),
                (b"&lf;".as_slice(), b'\n'),
            ]
            .into_iter()
            .find(|(s, _)| rest.starts_with(s));
            if let Some((s, c)) = entity {
                out.push(c);
                i += s.len();
            } else {
                out.push(raw[i]);
                i += 1;
            }
        }
        Ok(Cow::Owned(out))
    }
}
fn number(token: &[u8]) -> Result<AsciiValue<'_>, Error> {
    // Fast path for the common short integer token (index arrays): an
    // optional sign and 1..=18 digits cannot overflow i64, cannot be a special
    // spelling and parse exactly as `str::parse::<i64>` would.
    let (sign, digits) = match token.first() {
        Some(b'-') => (-1, &token[1..]),
        Some(b'+') => (1, &token[1..]),
        _ => (1, token),
    };
    if (1..=18).contains(&digits.len()) && digits.iter().all(u8::is_ascii_digit) {
        let magnitude = digits
            .iter()
            .fold(0i64, |n, &d| n * 10 + i64::from(d - b'0'));
        return Ok(if magnitude == 0 && sign < 0 {
            AsciiValue::NegativeZero
        } else {
            AsciiValue::I64(sign * magnitude)
        });
    }
    match decimal(token) {
        Decimal::Exact(value) => return Ok(AsciiValue::F64(value.to_bits())),
        // A plain decimal spelling: the checks below cannot apply, so parse
        // directly (the token is ASCII and `str::parse::<f64>` accepts it).
        Decimal::General => {
            let text = std::str::from_utf8(token).map_err(|_| Error::InvalidFormat)?;
            return text
                .parse::<f64>()
                .map(|n| AsciiValue::F64(n.to_bits()))
                .map_err(|_| Error::UnsupportedSyntax);
        }
        Decimal::Other => {}
    }
    let text = std::str::from_utf8(token).map_err(|_| Error::InvalidFormat)?;
    if let Some(value) = special_number(token) {
        return Ok(AsciiValue::F64(value.to_bits()));
    }
    // Never accept a partial special spelling or an arbitrary MSVC suffix.
    if token.contains(&b'#') {
        return Err(Error::UnsupportedSyntax);
    }
    if token.iter().any(|b| matches!(b, b'.' | b'e' | b'E')) {
        text.parse::<f64>()
            .map(|n| AsciiValue::F64(n.to_bits()))
            .map_err(|_| Error::UnsupportedSyntax)
    } else {
        let value = text.parse::<i64>().map_err(|_| Error::InvalidFormat)?;
        if value == 0 && token.first() == Some(&b'-') {
            Ok(AsciiValue::NegativeZero)
        } else {
            Ok(AsciiValue::I64(value))
        }
    }
}
/// Classification of an ASCII number token by [`decimal`].
enum Decimal {
    /// The value, computed exactly by one IEEE operation.
    Exact(f64),
    /// A plain decimal float spelling the general parser must round.
    General,
    /// Anything else (integers, special values, unusual spellings).
    Other,
}
/// Classify a token `[sign] digits [. digits] [e|E [sign] 1-4 digits]` with a
/// point or an exponent. Its value is exact under Clinger's fast path when it
/// has at most 19 significant digits, a mantissa of at most 2^53 and a power
/// of ten of magnitude at most 22: both operands are then exact, so the single
/// multiplication or division rounds correctly and equals `str::parse::<f64>`.
fn decimal(token: &[u8]) -> Decimal {
    match exact_decimal(token) {
        Ok(Some(v)) => Decimal::Exact(v),
        Ok(None) => Decimal::General,
        Err(()) => Decimal::Other,
    }
}
/// `Ok(Some)` exact value, `Ok(None)` valid spelling without an exact fast
/// path, `Err` not a plain decimal spelling.
fn exact_decimal(token: &[u8]) -> Result<Option<f64>, ()> {
    const POW10: [f64; 23] = [
        1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
        1e17, 1e18, 1e19, 1e20, 1e21, 1e22,
    ];
    let (negative, mut rest) = match token.first().ok_or(())? {
        b'-' => (true, &token[1..]),
        b'+' => (false, &token[1..]),
        _ => (false, token),
    };
    let mut mantissa = 0u64;
    let mut digits = [0usize; 2];
    let mut significant = 0usize;
    let mut scale = 0i32;
    let mut fraction = false;
    let mut float = false;
    while let Some((&b, tail)) = rest.split_first() {
        match b {
            b'0'..=b'9' => {
                digits[usize::from(fraction)] += 1;
                if mantissa != 0 || b != b'0' {
                    significant += 1;
                }
                if significant <= 19 {
                    mantissa = mantissa * 10 + u64::from(b - b'0');
                    if fraction {
                        scale -= 1;
                    }
                }
            }
            b'.' if !fraction => {
                fraction = true;
                float = true;
            }
            b'e' | b'E' => {
                float = true;
                let (sign, exponent) = match tail.first().ok_or(())? {
                    b'-' => (-1, &tail[1..]),
                    b'+' => (1, &tail[1..]),
                    _ => (1, tail),
                };
                if !(1..=4).contains(&exponent.len()) || !exponent.iter().all(u8::is_ascii_digit) {
                    return Err(());
                }
                let e = exponent
                    .iter()
                    .fold(0i32, |n, &d| n * 10 + i32::from(d - b'0'));
                scale += sign * e;
                rest = &[];
                continue;
            }
            _ => return Err(()),
        }
        rest = tail;
    }
    // Require digits on both sides of a point, as every FBX writer emits;
    // rarer spellings keep the general parser's acceptance rules.
    if digits[0] == 0 || (fraction && digits[1] == 0) || !float {
        return Err(());
    }
    if significant > 19 || mantissa > 1 << 53 {
        return Ok(None);
    }
    let magnitude = if mantissa == 0 {
        0.0
    } else {
        let Some(&power) = usize::try_from(scale.unsigned_abs())
            .ok()
            .and_then(|k| POW10.get(k))
        else {
            return Ok(None);
        };
        if scale < 0 {
            mantissa as f64 / power
        } else {
            mantissa as f64 * power
        }
    };
    Ok(Some(if negative { -magnitude } else { magnitude }))
}

// ufbx 0.23.1 ufbxi_parse_inf_nan: one legacy mantissa digit, INF/NAN/IND,
// optional legacy trailing digits; signed quiet NaN, optional alphanumeric
// standard NaN payload. Require consumption of the complete token.
pub(super) fn special_number(token: &[u8]) -> Option<f64> {
    let (negative, s) = match token.first()? {
        b'-' => (true, &token[1..]),
        b'+' => (false, &token[1..]),
        _ => (false, token),
    };
    let bits = if s.len() >= 3 && s[0].is_ascii_digit() && &s[1..3] == b".#" {
        let kind = s.get(3..6)?;
        if !s[6..].iter().all(u8::is_ascii_digit) {
            return None;
        }
        if kind.eq_ignore_ascii_case(b"inf") {
            0x7ff0_0000_0000_0000
        } else if kind.eq_ignore_ascii_case(b"nan") || kind.eq_ignore_ascii_case(b"ind") {
            0x7ff8_0000_0000_0000
        } else {
            return None;
        }
    } else if s.eq_ignore_ascii_case(b"inf") || s.eq_ignore_ascii_case(b"infinity") {
        0x7ff0_0000_0000_0000
    } else if s.get(..3).is_some_and(|n| n.eq_ignore_ascii_case(b"nan")) {
        let tail = &s[3..];
        if !tail.is_empty()
            && !(tail.starts_with(b"(")
                && tail.ends_with(b")")
                && tail[1..tail.len() - 1]
                    .iter()
                    .all(u8::is_ascii_alphanumeric))
        {
            return None;
        }
        0x7ff8_0000_0000_0000
    } else {
        return None;
    };
    Some(f64::from_bits(bits | if negative { 1 << 63 } else { 0 }))
}

/// Version from a leading magic comment such as `; FBX 7.3.0 project file`.
/// Mirrors the accepted shape: `;`, optional blanks, `FBX`, blanks, `d.d.d`.
fn magic_version(input: &[u8], mut pos: usize) -> Option<u32> {
    while input.get(pos).is_some_and(|b| b.is_ascii_whitespace()) {
        pos += 1;
    }
    if input.get(pos) != Some(&b';') {
        return None;
    }
    pos += 1;
    while matches!(input.get(pos), Some(b' ' | b'\t')) {
        pos += 1;
    }
    if !input.get(pos..)?.starts_with(b"FBX") {
        return None;
    }
    pos += 3;
    while matches!(input.get(pos), Some(b' ' | b'\t')) {
        pos += 1;
    }
    let digits = input.get(pos..pos + 5)?;
    let digit = |b: u8| b.is_ascii_digit().then(|| u32::from(b - b'0'));
    if digits[1] != b'.' || digits[3] != b'.' {
        return None;
    }
    Some(digit(digits[0])? * 1000 + digit(digits[2])? * 100 + digit(digits[4])? * 10)
}

pub(super) fn parse<'a>(input: &'a [u8], limits: &Limits) -> Result<Document<'a>, Error> {
    let mut c = Cursor {
        input,
        pos: usize::from(input.starts_with(b"\xef\xbb\xbf")) * 3,
    };
    let magic = magic_version(input, c.pos);
    if let Some(v) = magic {
        if !super::supported_version(v) {
            return Err(Error::UnsupportedVersion { v });
        }
    }
    let mut doc = Document {
        version: magic.unwrap_or(0),
        nodes: Vec::new(),
        properties: Vec::new(),
    };
    let mut parents: Vec<usize> = Vec::new();
    let mut total = 0usize;
    let mut elements = 0usize;
    loop {
        c.whitespace();
        if c.peek().is_none() {
            if !parents.is_empty() {
                return Err(Error::Truncated);
            }
            break;
        }
        if c.peek() == Some(b'}') {
            c.pos += 1;
            if parents.pop().is_none() {
                return Err(Error::InvalidFormat);
            }
            continue;
        }
        let start = c.pos;
        while !matches!(c.peek(), None | Some(b':' | b'\n' | b'{' | b'}')) {
            c.pos += 1;
        }
        if c.peek() != Some(b':') {
            return Err(Error::InvalidFormat);
        }
        let mut end = c.pos;
        while end > start && input[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        let name = &input[start..end];
        c.pos += 1;
        if name.is_empty() {
            return Err(Error::InvalidFormat);
        }
        if name.len() > limits.max_string_length {
            return Err(Error::LimitExceeded {
                what: "string length",
            });
        }
        if parents.len() >= limits.max_depth {
            return Err(Error::TooDeep);
        }
        if doc.nodes.len() >= limits.max_nodes {
            return Err(Error::LimitExceeded { what: "nodes" });
        }
        let prop_start = doc.properties.len();
        c.horizontal();
        // `ufbxi_ascii_next_token` skips newlines before the first value,
        // including old `Key:` arrays. An identifier followed by a colon is
        // the next node, so an empty property list must stop before it.
        if matches!(c.peek(), Some(b'\n' | b';')) {
            let saved = c.pos;
            c.whitespace();
            let value = c.pos;
            if c.peek()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            {
                while c.peek().is_some_and(|b| {
                    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'(' | b')')
                }) {
                    c.pos += 1;
                }
                c.whitespace();
                c.pos = if c.peek() == Some(b':') { saved } else { value };
            }
        }
        if c.peek() == Some(b',') {
            c.pos += 1;
            c.whitespace();
        }
        let mut has_child = false;
        loop {
            c.horizontal();
            match c.peek() {
                None | Some(b'\n' | b';' | b'}') => break,
                Some(b'{') => {
                    c.pos += 1;
                    has_child = true;
                    break;
                }
                Some(b'*') => {
                    c.pos += 1;
                    let start_count = c.pos;
                    while c.peek().is_some_and(|b| b.is_ascii_digit()) {
                        c.pos += 1;
                    }
                    let count_text = std::str::from_utf8(&input[start_count..c.pos])
                        .map_err(|_| Error::InvalidFormat)?;
                    let count = count_text
                        .parse::<usize>()
                        .map_err(|_| Error::LimitExceeded {
                            what: "array elements",
                        })?;
                    elements = elements.checked_add(count).ok_or(Error::LimitExceeded {
                        what: "array elements",
                    })?;
                    if elements > limits.max_array_elements {
                        return Err(Error::LimitExceeded {
                            what: "array elements",
                        });
                    }
                    c.whitespace();
                    if c.peek().is_none() {
                        return Err(Error::Truncated);
                    }
                    if c.peek() != Some(b'{') {
                        return Err(Error::InvalidFormat);
                    }
                    c.pos += 1;
                    c.whitespace();
                    if b"a:".starts_with(&input[c.pos..]) {
                        return Err(Error::Truncated);
                    }
                    if !input[c.pos..].starts_with(b"a:") {
                        return Err(Error::InvalidFormat);
                    }
                    c.pos += 2;
                    let mut values = Vec::new();
                    crate::allocation::reserve(
                        &mut values,
                        count,
                        &mut total,
                        limits.max_allocation,
                    )
                    .map_err(allocation_error)?;
                    loop {
                        c.whitespace();
                        if c.peek() == Some(b'}') {
                            c.pos += 1;
                            break;
                        }
                        if c.peek().is_none() {
                            return Err(Error::Truncated);
                        }
                        if values.len() >= count {
                            return Err(Error::InvalidFormat);
                        }
                        let begin = c.pos;
                        while !matches!(
                            c.peek(),
                            None | Some(b',' | b' ' | b'\t' | b'\r' | b'\n' | b'}' | b';')
                        ) {
                            c.pos += 1;
                        }
                        let token = &input[begin..c.pos];
                        if token.len() > limits.max_string_length {
                            return Err(Error::LimitExceeded {
                                what: "string length",
                            });
                        }
                        let value = if token
                            .first()
                            .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
                            && token.iter().all(|b| {
                                b.is_ascii_alphanumeric() || matches!(*b, b'_' | b'-' | b'(' | b')')
                            }) {
                            AsciiValue::Bare(token)
                        } else {
                            number(token)?
                        };
                        values.push(value);
                        c.whitespace();
                        if c.peek() == Some(b',') {
                            c.pos += 1;
                        } else if c.peek().is_none() {
                            return Err(Error::Truncated);
                        } else if c.peek() != Some(b'}') {
                            return Err(Error::InvalidFormat);
                        }
                    }
                    if values.len() != count {
                        return Err(Error::InvalidFormat);
                    }
                    if doc.properties.len() >= limits.max_properties {
                        return Err(Error::LimitExceeded { what: "properties" });
                    }
                    push(
                        &mut doc.properties,
                        Property::AsciiArray(values),
                        &mut total,
                        limits.max_allocation,
                    )?;
                    break;
                }
                _ => {}
            }
            if doc.properties.len() >= limits.max_properties {
                return Err(Error::LimitExceeded { what: "properties" });
            }
            let prop = if c.peek() == Some(b'"') {
                Property::String(c.string(limits, &mut total)?)
            } else {
                let begin = c.pos;
                while !matches!(
                    c.peek(),
                    None | Some(b',' | b' ' | b'\t' | b'\r' | b'\n' | b'{' | b'}' | b';')
                ) {
                    c.pos += 1;
                }
                let token = &input[begin..c.pos];
                if token.len() > limits.max_string_length {
                    return Err(Error::LimitExceeded {
                        what: "string length",
                    });
                }
                if token
                    .first()
                    .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
                    && token.iter().all(|b| {
                        b.is_ascii_alphanumeric() || matches!(*b, b'_' | b'-' | b'(' | b')')
                    })
                {
                    Property::AsciiBare(token)
                } else {
                    match number(token)? {
                        AsciiValue::I64(n) => Property::I64(n),
                        AsciiValue::F64(bits) => Property::F64(bits),
                        AsciiValue::Bare(_) => return Err(Error::InvalidFormat),
                        AsciiValue::NegativeZero => Property::AsciiNegativeZero,
                    }
                }
            };
            push(&mut doc.properties, prop, &mut total, limits.max_allocation)?;
            c.horizontal();
            if c.peek() == Some(b',') {
                c.pos += 1;
                c.whitespace();
                continue;
            }
            if doc.version < 7000 && matches!(c.peek(), Some(b'\n' | b';')) {
                let saved = c.pos;
                c.whitespace();
                if c.peek() == Some(b',') {
                    c.pos += 1;
                    c.whitespace();
                    continue;
                }
                c.pos = saved;
            }
            if !matches!(c.peek(), None | Some(b'\n' | b';' | b'{' | b'}')) {
                return Err(Error::InvalidFormat);
            }
        }
        let node_index = doc.nodes.len();
        if magic.is_none()
            && name == b"FBXVersion"
            && parents
                .last()
                .and_then(|&i| doc.nodes.get(i))
                .is_some_and(|n| n.name == b"FBXHeaderExtension")
        {
            if let Some(Property::I64(v)) = doc.properties.get(prop_start) {
                doc.version = u32::try_from(*v).map_err(|_| Error::InvalidFormat)?;
                if !super::supported_version(doc.version) {
                    return Err(Error::UnsupportedVersion { v: doc.version });
                }
            }
        }
        let prop_end = doc.properties.len();
        push(
            &mut doc.nodes,
            Node {
                name,
                parent: parents.last().copied(),
                properties: prop_start..prop_end,
            },
            &mut total,
            limits.max_allocation,
        )?;
        if has_child {
            push(&mut parents, node_index, &mut total, limits.max_allocation)?;
        }
    }
    if doc.version == 0 {
        return Err(Error::InvalidFormat);
    }
    if !super::supported_version(doc.version) {
        return Err(Error::UnsupportedVersion { v: doc.version });
    }
    Ok(doc)
}
