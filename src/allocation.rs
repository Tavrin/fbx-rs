//! Exact reservation with a precharged capacity envelope. This accounts for
//! Rust-visible capacities, not allocator metadata or process RSS.
use std::mem::size_of;

/// Default allocation envelope (4 GiB), saturated to the address space
/// on targets with a narrower `usize` (wasm32).
#[cfg(target_pointer_width = "64")]
pub(crate) const DEFAULT_MAX_ALLOCATION: usize = 4 << 30;
#[cfg(not(target_pointer_width = "64"))]
pub(crate) const DEFAULT_MAX_ALLOCATION: usize = usize::MAX;

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
        #[cfg(test)]
        crate::api_alloc::FAIL_AFTER.with(|f| {
            if let Some(left) = f.get() {
                if left == 0 {
                    return Err(Error::Failed);
                }
                f.set(Some(left - 1));
            }
            Ok(())
        })?;
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
    charge_live(requested)?;
    *used = limit;
    if let Err(e) = allocate(v, additional, *used) {
        release_live(requested);
        *used = before;
        return Err(e);
    }
    let actual = v.capacity().checked_mul(width).ok_or(Error::Limit)?;
    if actual > requested {
        charge_live(actual - requested)?;
    } else {
        release_live(requested - actual);
    }
    release_live(old);
    if actual > available {
        return Err(Error::Limit);
    }
    *used = before
        .checked_sub(old)
        .and_then(|n| n.checked_add(actual))
        .ok_or(Error::Limit)?;
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

// One envelope spans parser, projection and public ownership conversion. Storage
// not explicitly released remains charged conservatively until load returns.
#[derive(Clone, Copy)]
struct Live {
    bytes: usize,
    limit: usize,
    generation: usize,
}
thread_local! {
    static LIVE: std::cell::Cell<Option<Live>> = const { std::cell::Cell::new(None) };
}
static GENERATION: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
pub(crate) struct LoadScope {
    previous_limit: Option<usize>,
}
impl LoadScope {
    pub(crate) fn new(limit: usize) -> Self {
        let previous_limit = LIVE.with(|cell| {
            let previous = cell.get();
            let live = match previous {
                Some(live) => Live {
                    limit: live.limit.min(limit),
                    ..live
                },
                None => Live {
                    bytes: 0,
                    limit,
                    generation: GENERATION
                        .fetch_update(
                            std::sync::atomic::Ordering::Relaxed,
                            std::sync::atomic::Ordering::Relaxed,
                            |n| n.checked_add(1),
                        )
                        .unwrap_or(0),
                },
            };
            cell.set(Some(live));
            previous.map(|live| live.limit)
        });
        Self { previous_limit }
    }
}
impl Drop for LoadScope {
    fn drop(&mut self) {
        LIVE.with(|cell| match (self.previous_limit, cell.get()) {
            (Some(limit), Some(live)) => cell.set(Some(Live { limit, ..live })),
            _ => cell.set(None),
        });
    }
}
pub(crate) fn load_active() -> bool {
    LIVE.with(|cell| cell.get().is_some())
}
fn charge_live(bytes: usize) -> Result<(), Error> {
    LIVE.with(|cell| {
        if let Some(mut live) = cell.get() {
            live.bytes = live
                .bytes
                .checked_add(bytes)
                .filter(|&n| n <= live.limit)
                .ok_or(Error::Limit)?;
            cell.set(Some(live));
        }
        Ok(())
    })
}
pub(crate) fn release_live(bytes: usize) {
    let _ = LIVE.try_with(|cell| {
        if let Some(mut live) = cell.get() {
            // A bookkeeping inconsistency must retain the charge, never grant
            // unearned capacity. All normal releases are exact capacities.
            if let Some(left) = live.bytes.checked_sub(bytes) {
                live.bytes = left;
                cell.set(Some(live));
            }
        }
    });
}
#[derive(Debug, Default)]
pub(crate) struct Release {
    bytes: usize,
    generation: Option<std::num::NonZeroUsize>,
}
impl Clone for Release {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl Release {
    pub(crate) fn update(&mut self, bytes: usize) {
        self.bytes = bytes;
        self.generation = LIVE.with(|c| {
            c.get()
                .and_then(|v| std::num::NonZeroUsize::new(v.generation))
        });
    }
    pub(crate) fn new(bytes: usize) -> Self {
        Self {
            bytes,
            generation: LIVE.with(|c| {
                c.get()
                    .and_then(|v| std::num::NonZeroUsize::new(v.generation))
            }),
        }
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        if self.generation.is_some()
            && LIVE
                .try_with(|c| {
                    c.get()
                        .and_then(|v| std::num::NonZeroUsize::new(v.generation))
                })
                .unwrap_or(None)
                == self.generation
        {
            release_live(self.bytes);
        }
    }
}

/// Fixed-capacity scratch allocated by the checked reservation helpers. Slice
/// access prevents capacity changes; an owning iterator retains the refund
/// token until the backing buffer has actually been freed.
pub(crate) struct Temporary<T> {
    values: Vec<T>,
    release: Release,
}
impl<T> Temporary<T> {
    pub(crate) fn new(values: Vec<T>) -> Result<Self, crate::Error> {
        let bytes = values
            .capacity()
            .checked_mul(size_of::<T>())
            .ok_or(crate::Error::LimitExceeded(crate::Limit::Allocation))?;
        Ok(Self {
            values,
            release: Release::new(bytes),
        })
    }
}
impl<T> std::ops::Deref for Temporary<T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}
impl<T> std::ops::DerefMut for Temporary<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.values
    }
}
impl<'a, T> IntoIterator for &'a Temporary<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}
impl<'a, T> IntoIterator for &'a mut Temporary<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.iter_mut()
    }
}
pub(crate) struct TemporaryIter<T> {
    values: std::vec::IntoIter<T>,
    _release: Release,
}
impl<T> Iterator for TemporaryIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.values.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.values.size_hint()
    }
}
impl<T> ExactSizeIterator for TemporaryIter<T> {}
impl<T> IntoIterator for Temporary<T> {
    type Item = T;
    type IntoIter = TemporaryIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        TemporaryIter {
            values: self.values.into_iter(),
            _release: self.release,
        }
    }
}
