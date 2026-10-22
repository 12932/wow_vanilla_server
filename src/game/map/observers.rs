//! Who sees an object, and the map's numbering of its clients that a
//! broadcast to them is kept in (`conn::broadcast`).
//!
//! An object's [`Observers`] is the set of players whose clients know it.
//! Any change to it (any mutable use) counts a generation, and the set's
//! clients by their index on the map ([`Clients`]) are kept until the next:
//! a crowd that sees each other and moves sends a packet to all of them for
//! the cost of one, not of one for each.

use std::cell::RefCell;
use std::sync::Arc;

use ahash::{AHashMap, AHashSet};
use wow_world_messages::Guid;

use crate::game::conn::Clients;

/// The players who see an object.
#[derive(Debug, Default)]
pub struct Observers {
    set: AHashSet<Guid>,
    generation: u64,
    /// The set's clients, as of a generation and a table.
    clients: RefCell<Option<(u64, usize, Arc<Clients>)>>,
}

impl Clone for Observers {
    fn clone(&self) -> Self {
        Self {
            set: self.set.clone(),
            generation: self.generation,
            clients: RefCell::new(None),
        }
    }
}

impl std::ops::Deref for Observers {
    type Target = AHashSet<Guid>;

    fn deref(&self) -> &AHashSet<Guid> {
        &self.set
    }
}

/// Any change, or a chance of one: a new generation.
impl std::ops::DerefMut for Observers {
    fn deref_mut(&mut self) -> &mut AHashSet<Guid> {
        self.generation = self.generation.wrapping_add(1);
        *self.clients.get_mut() = None;
        &mut self.set
    }
}

impl<'a> IntoIterator for &'a Observers {
    type Item = &'a Guid;
    type IntoIter = std::collections::hash_set::Iter<'a, Guid>;

    fn into_iter(self) -> Self::IntoIter {
        self.set.iter()
    }
}

impl FromIterator<Guid> for Observers {
    fn from_iter<I: IntoIterator<Item = Guid>>(iter: I) -> Self {
        Self {
            set: iter.into_iter().collect(),
            ..Default::default()
        }
    }
}

impl Observers {
    /// The set's clients by their index in `table`: kept from the last
    /// time if the set hasn't changed since.
    pub fn clients(&self, table: &ClientTable) -> Arc<Clients> {
        if let Some((generation, id, c)) = &*self.clients.borrow()
            && *generation == self.generation
            && *id == table.id
        {
            #[cfg(debug_assertions)]
            debug_assert_eq!(
                **c,
                self.build(table),
                "an object's observers changed without a new generation"
            );
            return c.clone();
        }
        let c = Arc::new(self.build(table));
        *self.clients.borrow_mut() = Some((self.generation, table.id, c.clone()));
        c
    }

    /// Every one in the set: whether each is still on the map is the
    /// broadcast's to find, by its index.
    fn build(&self, table: &ClientTable) -> Clients {
        let mut c = Clients::default();
        for &g in &self.set {
            c.insert(table.index_of(g));
        }
        c
    }
}

/// The map's numbering of its players' clients, dense, for the sets a
/// broadcast goes to. A player has an index from the first time one is
/// asked for until they leave the map; an index let go is given to another
/// player only after the cork that might hold it has been pushed
/// (`Map::take_player` uncorks first).
#[derive(Debug)]
pub struct ClientTable {
    /// Unique among maps, so a cork knows whose numbering it holds.
    pub id: usize,
    inner: RefCell<TableInner>,
}

#[derive(Debug, Default)]
struct TableInner {
    index: AHashMap<Guid, usize>,
    guids: Vec<Option<Guid>>,
    free: Vec<usize>,
}

impl Default for ClientTable {
    fn default() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
        Self {
            id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            inner: RefCell::new(TableInner::default()),
        }
    }
}

impl ClientTable {
    /// `g`'s index, given one now if they have none.
    pub fn index_of(&self, g: Guid) -> usize {
        let mut t = self.inner.borrow_mut();
        if let Some(&i) = t.index.get(&g) {
            return i;
        }
        let i = match t.free.pop() {
            Some(i) => i,
            None => {
                t.guids.push(None);
                t.guids.len() - 1
            }
        };
        t.guids[i] = Some(g);
        t.index.insert(g, i);
        i
    }

    /// The player with index `i`.
    pub fn guid(&self, i: usize) -> Option<Guid> {
        self.inner.borrow().guids.get(i).copied().flatten()
    }

    /// `g` leaves the map: their index is free.
    pub fn release(&self, g: Guid) {
        let mut t = self.inner.borrow_mut();
        if let Some(i) = t.index.remove(&g) {
            t.guids[i] = None;
            t.free.push(i);
        }
    }

    /// Players with an index.
    pub fn len(&self) -> usize {
        self.inner.borrow().index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
