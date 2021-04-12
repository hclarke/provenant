# provenant

A version of `Arc<T>` and `Weak<T>` that's able to free memory as soon as all of the Arcs are dropped, even if `Weak` references remain.

## features

- Memory is freed when the last `Arc` is dropped
- `Weak` is `Copy`

## the magic
It does this by probabilistically tracking provenance at runtime:
- pointed-to memory gets a random id when initialized
- weak pointers get a copy of that id
- weak pointers fail to upgrade if the id doesn't match

the idea is that if the weak pointer's id doesn't match the memory's id, it must have been dropped. it's either zeroed from the drop, or it now has something else in it.

# ⚠️
It's possible for weak pointers to get a false positive, if the backing memory gets used for something else and happens to have the id's bit pattern in the same memory location. good luck :)
