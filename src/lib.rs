use rand::Rng;
use std::ops::Deref;
use std::ptr;
use std::sync::atomic::{compiler_fence, AtomicUsize, Ordering};

/// An atomically reference counted shared pointer
///
/// See the documentation for [`Arc`](std::sync::Arc) in the standard library.
/// This one has different weak pointers.
pub struct Arc<T: ?Sized> {
    ptr: *const Inner<T>,
}

/// A weak pointer to an atomically reference counted shared pointer
///
/// Can be upgraded to an [`Arc`], and will usually do the right thing.
/// Does not prevent the pointed-to memory from being dropped or deallocated.
#[derive(Copy, Clone)]
pub struct Weak<T: ?Sized> {
    provenance: usize,
    ptr: *const Inner<T>,
}

struct Inner<T: ?Sized> {
    // the low bit is used to locking, the rest are random provenance id
    provenance: AtomicUsize,

    // reference count of Arcs. Weak refs are uncounted
    ref_count: AtomicUsize,

    data: T,
}

impl<T: ?Sized> Drop for Inner<T> {
    fn drop(&mut self) {
        // using a volatile write followed by a fence should actually zero the memory
        // and not get optimized out
        unsafe {
            ptr::write_volatile(&mut self.provenance, AtomicUsize::default());
        }
        compiler_fence(Ordering::SeqCst);
    }
}

impl<T: ?Sized> Inner<T> {
    fn weak(&self) -> Weak<T> {
        let provenance = self.provenance.load(Ordering::Relaxed);
        let provenance = provenance ^ (provenance & 1); //clear low bit
        Weak {
            provenance: provenance,
            ptr: self as *const Inner<T>,
        }
    }

    fn lock(&self, exp: usize) -> bool {
        loop {
            match self
                .provenance
                .compare_exchange(exp, exp | 1, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return true,
                Err(v) if v == exp | 1 => continue,
                Err(_) => return false,
            }
        }
    }
}

impl<T: ?Sized> Weak<T> {
    /// Attempts to get a strong reference to the pointed-to memory. Will probably fail and return None
    /// if there are no strong pointers left.
    pub fn upgrade(&self) -> Option<Arc<T>> {
        let exp = self.provenance;

        let inner = unsafe { &(*self.ptr) };

        if !inner.lock(exp) {
            return None;
        }

        // increment ref count
        inner.ref_count.fetch_add(1, Ordering::SeqCst);

        // release the lock
        inner.provenance.store(exp, Ordering::SeqCst);

        Some(Arc { ptr: self.ptr })
    }
}

impl<T: ?Sized> Drop for Arc<T> {
    fn drop(&mut self) {
        {
            let inner = unsafe { &(*self.ptr) };

            // we need to load provenance before decrementing ref count.
            // otherwise, another thread could deallocate before the load happens
            let exp = inner.provenance.load(Ordering::SeqCst);
            let exp = exp ^ (exp & 1);

            if inner.ref_count.fetch_sub(1, Ordering::SeqCst) > 1 {
                return;
            }

            // if the lock fails, another thread must have dropped Inner already
            // that can happen if this gets interrupted while a weak pointer
            // upgrades and then drops (hitting 0 again)
            if !inner.lock(exp) {
                return;
            }

            // if the ref count isn't 0, a weak pointer managed to upgrade.
            // it can deal with deallocating when it hits 0 again.
            if inner.ref_count.load(Ordering::SeqCst) != 0 {
                inner.provenance.store(exp, Ordering::SeqCst);
                return;
            }

            // setting provenance to 0 isn't strictly necessary here, since Inner::drop does it
            inner.provenance.store(0, Ordering::SeqCst);
        }

        unsafe {
            Box::from_raw(self.ptr as *mut Inner<T>);
        }
    }
}

impl<T> Arc<T> {
    /// Create a new shared reference
    pub fn new(val: T) -> Self {
        let mut rng = rand::thread_rng();
        let provenance: usize = rng.gen();
        let provenance = provenance ^ (provenance & 1);
        let inner = Box::new(Inner {
            provenance: AtomicUsize::new(provenance),
            ref_count: AtomicUsize::new(1),
            data: val,
        });

        let inner = Box::into_raw(inner) as *const Inner<T>;
        Arc { ptr: inner }
    }
}

impl<T: ?Sized> Arc<T> {
    /// Gets a weak reference to the same memory
    pub fn downgrade(this: &Self) -> Weak<T> {
        let inner = unsafe { &(*this.ptr) };

        inner.weak()
    }
}

impl<T: ?Sized> Deref for Arc<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        let inner = unsafe { &(*self.ptr) };

        &inner.data
    }
}

impl<T: ?Sized> Clone for Arc<T> {
    fn clone(&self) -> Self {
        let inner = unsafe { &(*self.ptr) };

        inner.ref_count.fetch_add(1, Ordering::SeqCst);

        Arc { ptr: self.ptr }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn use_after_free() {
        let arc = Arc::new(50);
        let weak = Arc::downgrade(&arc);

        assert_eq!(50, *weak.upgrade().unwrap());

        drop(arc);

        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn use_after_clone() {
        let arc = Arc::new(55);
        let weak = Arc::downgrade(&arc);

        let cloned = arc.clone();
        assert_eq!(55, *weak.upgrade().unwrap());

        drop(arc);

        assert_eq!(55, *weak.upgrade().unwrap());

        drop(cloned);

        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn revive() {
        let arc = Arc::new(21);
        let weak = Arc::downgrade(&arc);
        let strong = weak.upgrade().unwrap();
        drop(arc);

        assert_eq!(21, *weak.upgrade().unwrap());

        drop(strong);

        assert!(weak.upgrade().is_none());
    }
}
