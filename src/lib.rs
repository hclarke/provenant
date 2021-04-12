use rand::Rng;
use std::ops::Deref;
use std::ptr;
use std::sync::atomic::{compiler_fence, AtomicUsize, Ordering};

pub struct Arc<T: ?Sized> {
    ptr: *const Inner<T>,
}

#[derive(Copy, Clone)]
pub struct Weak<T: ?Sized> {
    provenance: usize,
    ptr: *const Inner<T>,
}

struct Inner<T: ?Sized> {
    provenance: AtomicUsize,
    ref_count: AtomicUsize,
    data: T,
}

impl<T: ?Sized> Drop for Inner<T> {
    fn drop(&mut self) {
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
    pub fn upgrade(&self) -> Option<Arc<T>> {
        let exp = self.provenance;

        let inner = unsafe { &(*self.ptr) };

        if !inner.lock(exp) {
            return None;
        }

        inner.ref_count.fetch_add(1, Ordering::SeqCst);

        inner.provenance.store(exp, Ordering::SeqCst);

        Some(Arc { ptr: self.ptr })
    }
}

impl<T: ?Sized> Drop for Arc<T> {
    fn drop(&mut self) {
        {
            let inner = unsafe { &(*self.ptr) };

            if inner.ref_count.fetch_sub(1, Ordering::SeqCst) > 1 {
                return;
            }

            let exp = inner.provenance.load(Ordering::Relaxed);
            let exp = exp ^ (exp & 1);

            if !inner.lock(exp) {
                return;
            }

            if inner.ref_count.load(Ordering::SeqCst) != 0 {
                inner.provenance.store(exp, Ordering::SeqCst);
                return;
            }
        }
        //TODO: atomic fence?
        unsafe {
            Box::from_raw(self.ptr as *mut Inner<T>);
        }
    }
}

impl<T> Arc<T> {
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
