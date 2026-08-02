//! Stable identity for document elements: a generational arena (#1055).
//!
//! Every element used to be identified by its position in a `Vec`, which is why deleting
//! anything tombstoned it instead of removing it — pulling an element out would renumber
//! every element after it, and every stored index anywhere in the document pointed at the
//! wrong thing. The cost was that ~2800 places had to remember to skip `deleted`, and any
//! one that forgot was a live bug (a deleted construction plane went on being pickable, #1051).
//!
//! A [`Key`] is an index plus a **generation**. Lookup is still an array offset, so this is
//! as fast as indexing a `Vec` — no hashing on the hot paths that resolve bodies and faces
//! every frame. Removing an element bumps its slot's generation, so a key held from before
//! stops resolving: a stale reference returns `None` rather than silently naming whichever
//! element moved into that position.

// The full handle API lands before the 35 collections that will use it, so parts of it are
// unreferenced until their conversion arrives. This allow comes off with the last one (#1055).
#![allow(dead_code)]

use std::marker::PhantomData;

/// A handle to a `T` in an [`Arena`]. Cheap to copy, stable across insertions and removals,
/// and typed — a `Key<Body>` cannot be passed where a `Key<Line>` is wanted, which the bare
/// `usize` indices could not prevent.
pub struct Key<T> {
    index: u32,
    generation: u32,
    /// `fn() -> T` rather than `T`, so a `Key<T>` is `Send`/`Sync`/`Copy` whatever `T` is.
    marker: PhantomData<fn() -> T>,
}

impl<T> Key<T> {
    fn new(index: u32, generation: u32) -> Self {
        Self { index, generation, marker: PhantomData }
    }

    /// The slot this key addresses. For diagnostics and stable ordering only — never for
    /// addressing another arena.
    pub fn index(self) -> u32 {
        self.index
    }

    pub fn generation(self) -> u32 {
        self.generation
    }

    /// The key as one integer, for storage layers with no place to put a pair — the SQLite
    /// row id an element is saved under (#1055). Lossless and round-trips through
    /// [`Key::from_bits`]; not an index, and not usable as one.
    pub fn to_bits(self) -> u64 {
        ((self.index as u64) << 32) | self.generation as u64
    }

    pub fn from_bits(bits: u64) -> Self {
        Self::new((bits >> 32) as u32, bits as u32)
    }
}

// Derived impls would demand `T: Clone`/`T: PartialEq` and so on; a key's identity is its two
// numbers, not anything about the value it points at.
impl<T> Clone for Key<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Key<T> {}
impl<T> PartialEq for Key<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}
impl<T> Eq for Key<T> {}
impl<T> std::hash::Hash for Key<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}
impl<T> PartialOrd for Key<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for Key<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.index, self.generation).cmp(&(other.index, other.generation))
    }
}
impl<T> std::fmt::Debug for Key<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}v{}", self.index, self.generation)
    }
}

// Serialized as the pair it is. Hand-written because deriving would put a `T: Serialize`
// bound on a type that holds no `T`.
impl<T> serde::Serialize for Key<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        (self.index, self.generation).serialize(s)
    }
}
impl<'de, T> serde::Deserialize<'de> for Key<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (index, generation) = <(u32, u32)>::deserialize(d)?;
        Ok(Self::new(index, generation))
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
enum Slot<T> {
    Occupied { generation: u32, value: T },
    /// The generation a *future* occupant of this slot will carry.
    Vacant { generation: u32 },
}

/// A collection whose elements keep their identity when their neighbours are removed.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(from = "ArenaRepr<T>", into = "ArenaRepr<T>", bound = "T: Clone + serde::Serialize + serde::de::DeserializeOwned")]
pub struct Arena<T> {
    slots: Vec<Slot<T>>,
    /// Slots to reuse before growing. Reusing keeps the backing array compact after a lot of
    /// churn; the generation bump is what stops the reuse being observable through an old key.
    free: Vec<u32>,
    len: usize,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self { slots: Vec::new(), free: Vec::new(), len: 0 }
    }
}

impl<T> Arena<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many elements are present. Unlike the tombstoned `Vec`s this replaces, this is the
    /// count a user would recognise — nothing removed is still being counted.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn insert(&mut self, value: T) -> Key<T> {
        self.len += 1;
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            let generation = match slot {
                Slot::Vacant { generation } => *generation,
                // A slot on the free list is vacant by construction.
                Slot::Occupied { .. } => unreachable!("free list held an occupied slot"),
            };
            *slot = Slot::Occupied { generation, value };
            return Key::new(index, generation);
        }
        let index = self.slots.len() as u32;
        self.slots.push(Slot::Occupied { generation: 0, value });
        Key::new(index, 0)
    }

    pub fn get(&self, key: Key<T>) -> Option<&T> {
        match self.slots.get(key.index as usize)? {
            Slot::Occupied { generation, value } if *generation == key.generation => Some(value),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, key: Key<T>) -> Option<&mut T> {
        match self.slots.get_mut(key.index as usize)? {
            Slot::Occupied { generation, value } if *generation == key.generation => Some(value),
            _ => None,
        }
    }

    pub fn contains(&self, key: Key<T>) -> bool {
        self.get(key).is_some()
    }

    /// Remove an element for real. The slot's generation moves on, so every key to the old
    /// occupant stops resolving — which is the whole point: a dangling reference reads as
    /// absent instead of resolving to whatever is there now.
    pub fn remove(&mut self, key: Key<T>) -> Option<T> {
        let slot = self.slots.get_mut(key.index as usize)?;
        let Slot::Occupied { generation, .. } = slot else {
            return None;
        };
        if *generation != key.generation {
            return None;
        }
        let next = generation.wrapping_add(1);
        let Slot::Occupied { value, .. } = std::mem::replace(slot, Slot::Vacant { generation: next })
        else {
            unreachable!("just matched an occupied slot")
        };
        self.free.push(key.index);
        self.len -= 1;
        Some(value)
    }

    /// Every live element with its key, in slot order — stable for a given arena, and the
    /// order elements were created in when nothing has been removed.
    pub fn iter(&self) -> impl Iterator<Item = (Key<T>, &T)> + '_ {
        self.slots.iter().enumerate().filter_map(|(i, slot)| match slot {
            Slot::Occupied { generation, value } => Some((Key::new(i as u32, *generation), value)),
            Slot::Vacant { .. } => None,
        })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Key<T>, &mut T)> + '_ {
        self.slots.iter_mut().enumerate().filter_map(|(i, slot)| match slot {
            Slot::Occupied { generation, value } => {
                Some((Key::new(i as u32, *generation), value))
            }
            Slot::Vacant { .. } => None,
        })
    }

    pub fn keys(&self) -> impl Iterator<Item = Key<T>> + '_ {
        self.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> impl Iterator<Item = &T> + '_ {
        self.iter().map(|(_, v)| v)
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> + '_ {
        self.iter_mut().map(|(_, v)| v)
    }
}

impl<T> std::ops::Index<Key<T>> for Arena<T> {
    type Output = T;
    fn index(&self, key: Key<T>) -> &T {
        self.get(key).expect("no element for this key")
    }
}

impl<T> std::ops::IndexMut<Key<T>> for Arena<T> {
    fn index_mut(&mut self, key: Key<T>) -> &mut T {
        self.get_mut(key).expect("no element for this key")
    }
}

impl<T> Arena<T> {
    /// Rebuild from elements that already have keys — a file's contents (#1055). The keys are
    /// preserved exactly, so every reference stored elsewhere in the document still resolves;
    /// slots no entry claims are free for reuse.
    pub fn from_keyed(entries: impl IntoIterator<Item = (Key<T>, T)>) -> Self {
        let entries: Vec<(Key<T>, T)> = entries.into_iter().collect();
        let mut arena = Self::new();
        let Some(top) = entries.iter().map(|(k, _)| k.index).max() else {
            return arena;
        };
        arena.slots = (0..=top).map(|_| Slot::Vacant { generation: 0 }).collect();
        for (key, value) in entries {
            arena.slots[key.index as usize] =
                Slot::Occupied { generation: key.generation, value };
            arena.len += 1;
        }
        arena.free = arena
            .slots
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, Slot::Vacant { .. }))
            .map(|(i, _)| i as u32)
            .collect();
        arena
    }
}

impl<T> FromIterator<T> for Arena<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut arena = Self::new();
        for value in iter {
            arena.insert(value);
        }
        arena
    }
}

/// On-disk shape: the live elements with their keys, and nothing about vacant slots. A
/// document that has had elements removed does not carry the holes into its file, and a
/// reload rebuilds an arena whose keys still match what the document refers to.
#[derive(serde::Serialize, serde::Deserialize)]
struct ArenaRepr<T> {
    entries: Vec<(u32, u32, T)>,
}

impl<T> From<Arena<T>> for ArenaRepr<T> {
    fn from(arena: Arena<T>) -> Self {
        let mut entries = Vec::with_capacity(arena.len);
        for (i, slot) in arena.slots.into_iter().enumerate() {
            if let Slot::Occupied { generation, value } = slot {
                entries.push((i as u32, generation, value));
            }
        }
        Self { entries }
    }
}

impl<T> From<ArenaRepr<T>> for Arena<T> {
    fn from(repr: ArenaRepr<T>) -> Self {
        let highest = repr.entries.iter().map(|(i, ..)| *i).max();
        let mut slots: Vec<Slot<T>> = match highest {
            Some(top) => (0..=top).map(|_| Slot::Vacant { generation: 0 }).collect(),
            None => Vec::new(),
        };
        let mut len = 0;
        for (index, generation, value) in repr.entries {
            slots[index as usize] = Slot::Occupied { generation, value };
            len += 1;
        }
        // Whatever the file did not fill is free to reuse.
        let free = slots
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, Slot::Vacant { .. }))
            .map(|(i, _)| i as u32)
            .collect();
        Self { slots, free, len }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_resolves_to_its_own_element() {
        let mut arena = Arena::new();
        let a = arena.insert("a");
        let b = arena.insert("b");
        assert_eq!(arena.get(a), Some(&"a"));
        assert_eq!(arena.get(b), Some(&"b"));
        assert_eq!(arena.len(), 2);
    }

    /// The whole reason for this type: removing an element must not renumber its neighbours,
    /// which is what forced every deletion to be a tombstone.
    #[test]
    fn removing_one_element_leaves_the_others_addressable() {
        let mut arena = Arena::new();
        let a = arena.insert("a");
        let b = arena.insert("b");
        let c = arena.insert("c");
        assert_eq!(arena.remove(b), Some("b"));
        assert_eq!(arena.get(a), Some(&"a"), "a is untouched");
        assert_eq!(arena.get(c), Some(&"c"), "and so is c, which used to shift");
        assert_eq!(arena.len(), 2, "the removed element is not still counted");
    }

    /// A stale key reads as absent rather than as whatever took the slot — the failure mode
    /// tombstones existed to avoid, now handled by the identity itself.
    #[test]
    fn a_key_to_a_removed_element_never_resolves_again() {
        let mut arena = Arena::new();
        let old = arena.insert("old");
        arena.remove(old);
        assert_eq!(arena.get(old), None);
        // The slot is reused, and the old key still does not resolve to the new occupant.
        let new = arena.insert("new");
        assert_eq!(new.index(), old.index(), "the slot was reused");
        assert_ne!(new, old, "but it is a different key");
        assert_eq!(arena.get(old), None, "the stale key stays dead");
        assert_eq!(arena.get(new), Some(&"new"));
    }

    #[test]
    fn removing_twice_is_not_an_error_and_does_not_double_count() {
        let mut arena = Arena::new();
        let a = arena.insert(1);
        assert_eq!(arena.remove(a), Some(1));
        assert_eq!(arena.remove(a), None);
        assert_eq!(arena.len(), 0);
    }

    #[test]
    fn iteration_yields_only_live_elements_with_their_keys() {
        let mut arena = Arena::new();
        let a = arena.insert("a");
        let b = arena.insert("b");
        let c = arena.insert("c");
        arena.remove(b);
        let seen: Vec<_> = arena.iter().map(|(k, v)| (k, *v)).collect();
        assert_eq!(seen, vec![(a, "a"), (c, "c")]);
    }

    #[test]
    fn a_round_trip_through_serde_keeps_every_key_valid() {
        let mut arena = Arena::new();
        let a = arena.insert("a".to_string());
        let b = arena.insert("b".to_string());
        let c = arena.insert("c".to_string());
        arena.remove(b);

        let json = serde_json::to_string(&arena).unwrap();
        let back: Arena<String> = serde_json::from_str(&json).unwrap();

        assert_eq!(back.get(a), Some(&"a".to_string()), "keys survive the file");
        assert_eq!(back.get(c), Some(&"c".to_string()));
        assert_eq!(back.get(b), None, "and a removed element stays removed");
        assert_eq!(back.len(), 2);
    }

    /// A reloaded arena must not hand a fresh element a key that an existing reference
    /// already uses.
    #[test]
    fn a_reloaded_arena_does_not_reissue_a_live_key() {
        let mut arena = Arena::new();
        let a = arena.insert(1);
        let b = arena.insert(2);
        arena.remove(a);

        let json = serde_json::to_string(&arena).unwrap();
        let mut back: Arena<i32> = serde_json::from_str(&json).unwrap();
        let fresh = back.insert(3);

        assert_ne!(fresh, b, "the live key is not reissued");
        assert_eq!(back.get(b), Some(&2), "and still resolves to its own element");
        assert_eq!(back.get(fresh), Some(&3));
    }

    #[test]
    fn a_key_round_trips_through_its_bit_form() {
        let mut arena = Arena::new();
        let a = arena.insert("a");
        arena.remove(a);
        let b = arena.insert("b");
        for key in [a, b] {
            assert_eq!(Key::<&str>::from_bits(key.to_bits()), key);
        }
        // The two share a slot and differ only by generation; the packing must keep them apart.
        assert_ne!(a.to_bits(), b.to_bits());
    }

    #[test]
    fn indexing_by_key_reads_and_writes_in_place() {
        let mut arena = Arena::new();
        let a = arena.insert(1);
        arena[a] += 41;
        assert_eq!(arena[a], 42);
    }
}
