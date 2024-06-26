use std::{
    collections::{HashMap, HashSet},
    hash,
    marker::PhantomData,
};

pub type FxHasher = ConstSafeBuildHasherDefault<rustc_hash::FxHasher>;
pub type FxHashMap<K, V> = HashMap<K, V, FxHasher>;
pub type FxHashSet<T> = HashSet<T, FxHasher>;

pub struct ConstSafeBuildHasherDefault<H>(PhantomData<fn(H) -> H>);

impl<H> Default for ConstSafeBuildHasherDefault<H> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H> Copy for ConstSafeBuildHasherDefault<H> {}

impl<H> Clone for ConstSafeBuildHasherDefault<H> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<H> ConstSafeBuildHasherDefault<H> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<H: Default + hash::Hasher> hash::BuildHasher for ConstSafeBuildHasherDefault<H> {
    type Hasher = H;

    fn build_hasher(&self) -> Self::Hasher {
        H::default()
    }
}

pub const fn new_const_hash_map<K, V, H>() -> HashMap<K, V, ConstSafeBuildHasherDefault<H>> {
    HashMap::with_hasher(ConstSafeBuildHasherDefault::new())
}

pub const fn new_const_hash_set<T, H>() -> HashSet<T, ConstSafeBuildHasherDefault<H>> {
    HashSet::with_hasher(ConstSafeBuildHasherDefault::new())
}
