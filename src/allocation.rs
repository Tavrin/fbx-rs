//! Exact reservation with a precharged capacity envelope. This accounts for
//! Rust-visible capacities, not allocator metadata or process RSS.
use std::mem::size_of;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Limit,
    Failed,
}

// Charge the entire remaining envelope before calling the allocator, so any
// accepted over-capacity is already charged. Refund unused capacity only after
// reservation, before exposing the vector to its caller. An allocator returning
// more than the envelope is rejected without using the allocation.
pub(crate) fn reserve<T>(
    v: &mut Vec<T>,
    additional: usize,
    used: &mut usize,
    limit: usize,
) -> Result<(), Error> {
    reserve_with(v, additional, used, limit, |v, n, _| {
        v.try_reserve_exact(n).map_err(|_| Error::Failed)
    })
}
fn reserve_with<T>(
    v: &mut Vec<T>,
    additional: usize,
    used: &mut usize,
    limit: usize,
    allocate: impl FnOnce(&mut Vec<T>, usize, usize) -> Result<(), Error>,
) -> Result<(), Error> {
    let count = v.len().checked_add(additional).ok_or(Error::Limit)?;
    if count <= v.capacity() {
        return Ok(());
    }
    let width = size_of::<T>();
    let old = v.capacity().checked_mul(width).ok_or(Error::Limit)?;
    let requested = count.checked_mul(width).ok_or(Error::Limit)?;
    let before = *used;
    let available = limit.checked_sub(before).ok_or(Error::Limit)?;
    // Keep the old allocation charged during growth (conservative peak bound).
    if requested > available {
        return Err(Error::Limit);
    }
    *used = limit;
    allocate(v, additional, *used)?;
    let actual = v.capacity().checked_mul(width).ok_or(Error::Limit)?;
    if actual > available {
        return Err(Error::Limit);
    }
    *used = before.checked_sub(old).ok_or(Error::Limit)? + actual;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capacity_envelope_precedes_allocator_and_use() {
        let mut v = Vec::<u8>::new();
        let mut used = 3;
        reserve_with(&mut v, 4, &mut used, 11, |v, n, charged| {
            assert_eq!((n, charged), (4, 11));
            v.try_reserve_exact(8).unwrap();
            Ok(())
        })
        .unwrap();
        assert_eq!(used, 3 + v.capacity());
        let mut called = false;
        assert_eq!(
            reserve_with(&mut Vec::<u64>::new(), 2, &mut used, 15, |_, _, _| {
                called = true;
                Ok(())
            }),
            Err(Error::Limit)
        );
        assert!(!called);
    }
    #[test]
    fn over_capacity_outside_envelope_is_never_exposed() {
        let mut used = 0;
        assert_eq!(
            reserve_with(&mut Vec::<u8>::new(), 4, &mut used, 4, |v, _, charged| {
                assert_eq!(charged, 4);
                v.try_reserve_exact(8).unwrap();
                Ok(())
            }),
            Err(Error::Limit)
        );
        assert_eq!(used, 4);
    }
}
