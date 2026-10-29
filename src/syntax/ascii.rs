//! Experimental ASCII parser; array values retain source numeric classes.
use super::{charge, push, AsciiValue, Document, Error, Limits, Node, Property};
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
        charge(total, raw.len(), limits.max_allocation)?;
        let mut out = Vec::new();
        out.try_reserve_exact(raw.len())
            .map_err(|_| Error::AllocationFailed)?;
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
    let text = std::str::from_utf8(token).map_err(|_| Error::InvalidFormat)?;
    // MSVC-style special values such as `1.#INF`, `-1.#INF`, `1.#IND`, `1.#QNAN`.
    if let Some(hash) = token.iter().position(|&b| b == b'#') {
        let (mantissa, special) = token.split_at(hash);
        let special = &special[1..];
        if !mantissa.contains(&b'.')
            || special.is_empty()
            || !special.iter().all(u8::is_ascii_alphabetic)
        {
            return Err(Error::UnsupportedSyntax);
        }
        // `#INF` is infinity; `#IND`, `#QNAN` and `#SNAN` are NaN.
        let value = if special.eq_ignore_ascii_case(b"INF") {
            if mantissa.first() == Some(&b'-') {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }
        } else {
            f64::NAN
        };
        return Ok(AsciiValue::F64(value.to_bits()));
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
                    let bytes = count
                        .checked_mul(std::mem::size_of::<AsciiValue<'_>>())
                        .ok_or(Error::LimitExceeded { what: "allocation" })?;
                    charge(&mut total, bytes, limits.max_allocation)?;
                    let mut values = Vec::new();
                    values
                        .try_reserve_exact(count)
                        .map_err(|_| Error::AllocationFailed)?;
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
                            && token
                                .iter()
                                .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
                        {
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
                    && token
                        .iter()
                        .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
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
