//! A client connection: a reader task that decodes packets, and a writer
//! task that encrypts headers and coalesces writes. Everything else talks
//! to the client through an [`Outbound`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc};
use tracing::{debug, warn};
use wow_srp::vanilla_header::{DecrypterHalf, EncrypterHalf, HeaderCrypto};
use wow_world_messages::Guid;
use wow_world_messages::errors::ExpectedOpcodeError;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::{MSG_MOVE_TELEPORT_ACK_Client, SMSG_PONG, ServerMessage};

/// Packets written to all clients since start-up.
pub static PACKETS_OUT: AtomicU64 = AtomicU64::new(0);
/// Packets read from all clients since start-up.
pub static PACKETS_IN: AtomicU64 = AtomicU64::new(0);

/// Encode a server message as `[size BE][opcode LE][body]`, the shape the
/// writer task encrypts. Encode once and share the frame between clients.
/// None for a message too big for the size field (wow_messages would cut
/// the size short, and the client's stream would be out of step).
pub fn encode<M: ServerMessage>(m: &M) -> Option<Arc<[u8]>> {
    let body = m.size_without_header() as usize;
    if body > crate::game::update::MAX_FRAME_BODY {
        warn!(
            "not sending opcode {:#x}: its {body} bytes don't fit a frame",
            M::OPCODE
        );
        return None;
    }
    let mut buf = Vec::with_capacity(m.size_without_header() as usize + 4);
    m.write_unencrypted_server(&mut buf).ok()?;
    Some(Arc::from(buf))
}

/// What a queue carries. A frame is stamped with the number of the last
/// chunk its client's feed had published when it was sent ([`Feed`]): the
/// writer writes it after the feed's chunks up to that one, and before the
/// rest.
enum Item {
    One(Arc<[u8]>, u64),
    /// What a cork kept for a client its feed doesn't carry, in order.
    Corked(Vec<Arc<[u8]>>, u64),
    /// From here on the client reads `feed`'s chunks after `after`, as the
    /// map's client `me`.
    Sub {
        feed: Arc<Feed>,
        me: usize,
        after: Arc<Chunk>,
        account: Arc<Account>,
    },
    /// The feed's chunks up to this one, and no more.
    Unsub(u64),
}

/// A set of a map's clients, by their index on the map (the map's own
/// numbering, `map::ClientTable`): a bit each.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Clients(Vec<u64>);

impl Clients {
    pub fn insert(&mut self, i: usize) {
        let w = i / 64;
        if self.0.len() <= w {
            self.0.resize(w + 1, 0);
        }
        self.0[w] |= 1 << (i % 64);
    }

    pub fn remove(&mut self, i: usize) {
        if let Some(w) = self.0.get_mut(i / 64) {
            *w &= !(1 << (i % 64));
        }
    }

    pub fn contains(&self, i: usize) -> bool {
        self.0.get(i / 64).is_some_and(|w| w & (1 << (i % 64)) != 0)
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|w| *w == 0)
    }

    pub fn len(&self) -> usize {
        self.0.iter().map(|w| w.count_ones() as usize).sum()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// The indices, in order.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.iter().enumerate().flat_map(|(w, &word)| {
            let mut word = word;
            std::iter::from_fn(move || {
                if word == 0 {
                    return None;
                }
                let bit = word.trailing_zeros() as usize;
                word &= word - 1;
                Some(w * 64 + bit)
            })
        })
    }

    /// Only those in `other` too, and `also` (whether `other` has it or
    /// not) if it is here.
    pub fn keep_common(&mut self, other: &Clients, also: Option<usize>) {
        let kept = also.filter(|&i| self.contains(i));
        self.0.truncate(other.0.len());
        for (a, b) in self.0.iter_mut().zip(&other.0) {
            *a &= b;
        }
        if let Some(i) = kept {
            self.insert(i);
        }
    }

    /// How many would be left by [`Clients::keep_common`], without the
    /// change.
    pub fn common(&self, other: &Clients, also: Option<usize>) -> usize {
        let n: usize = self
            .0
            .iter()
            .zip(&other.0)
            .map(|(a, b)| (a & b).count_ones() as usize)
            .sum();
        match also {
            Some(i) if self.contains(i) && !other.contains(i) => n + 1,
            _ => n,
        }
    }

    /// `other`'s too.
    pub fn add_all(&mut self, other: &Clients) {
        if self.0.len() < other.0.len() {
            self.0.resize(other.0.len(), 0);
        }
        for (a, b) in self.0.iter_mut().zip(&other.0) {
            *a |= b;
        }
    }

    /// These, but not `other`'s (in place).
    pub fn take_all(&mut self, other: &Clients) {
        for (a, b) in self.0.iter_mut().zip(&other.0) {
            *a &= !b;
        }
    }

    /// Each here that isn't in `other`, in order.
    pub fn iter_without<'a>(&'a self, other: &'a Clients) -> impl Iterator<Item = usize> + 'a {
        self.0.iter().enumerate().flat_map(move |(w, &word)| {
            let mut word = word & !other.0.get(w).copied().unwrap_or(0);
            std::iter::from_fn(move || {
                if word == 0 {
                    return None;
                }
                let bit = word.trailing_zeros() as usize;
                word &= word - 1;
                Some(w * 64 + bit)
            })
        })
    }

    /// Whether any of `other` is here too.
    fn meets(&self, other: &Clients) -> bool {
        self.0.iter().zip(&other.0).any(|(a, b)| a & b != 0)
    }

    /// Whether any here is in neither `a` nor `b`.
    fn beyond(&self, a: &Clients, b: &Clients) -> bool {
        self.0.iter().enumerate().any(|(w, &x)| {
            let a = a.0.get(w).copied().unwrap_or(0);
            let b = b.0.get(w).copied().unwrap_or(0);
            x & !(a | b) != 0
        })
    }

    /// These, but not `other`'s.
    fn without(&self, other: &Clients) -> Clients {
        let mut out = self.clone();
        for (a, b) in out.0.iter_mut().zip(&other.0) {
            *a &= !b;
        }
        out
    }
}

/// What a cork kept for each of its feed's clients alone, by index: each
/// frame before the broadcast its `u32` numbers (after the last, if past
/// it).
type Directs = ahash::AHashMap<usize, Vec<(u32, Arc<[u8]>)>>;

/// A frame to a set of a map's clients.
struct Broadcast {
    frame: Arc<[u8]>,
    to: Arc<Clients>,
}

/// A map's broadcasts, published once for all its clients: the map's cost
/// for a batch of them is one chunk, not a hand-off to each client. Each
/// client's writer reads the chunks itself, taking what is for it (its
/// bit in each broadcast's set) in order with what was sent to it alone,
/// and the waker ([`Feed::wake`], a thread of its own) charges each
/// client's budget for its share and wakes the writers that have one.
///
/// A client is a feed's from the first broadcast that reaches it (or a
/// direct send under the map's cork once it is) until its index is let go
/// ([`Feed::unsubscribe`]): a `Sub` and an `Unsub` in its queue, in order
/// with the rest, mark where its reading begins and ends. What anyone sends
/// it uncorked is stamped with the feed's last chunk, and written after it
/// ([`Item`]); what the map's cork keeps for it goes in the cork's chunk, in
/// its place among the broadcasts.
pub struct Feed {
    /// The map's table's: which numbering the sets are in.
    id: usize,
    /// The last chunk published.
    tail: parking_lot::Mutex<Arc<Chunk>>,
    /// Its number.
    published: AtomicU64,
    clients: parking_lot::Mutex<FeedClients>,
    /// The waker has been told of a chunk it hasn't looked at.
    signalled: AtomicBool,
    /// The last chunk the waker has looked at.
    waker_at: parking_lot::Mutex<Arc<Chunk>>,
}

/// A feed's clients.
#[derive(Default)]
struct FeedClients {
    /// Each client's queue by its index, and the chunk it reads after
    /// (shared with the waker, copied when it changes: rarely).
    subs: Arc<Vec<Option<Reader>>>,
    subscribed: Clients,
    /// Held ([`Outbound::hold`]): their broadcasts go into the hold.
    held: Clients,
    /// Indices with no client (a set's guid no longer on the map).
    gone: Clients,
}

#[derive(Clone)]
struct Reader {
    inner: std::sync::Weak<OutboundInner>,
    after: u64,
    account: Arc<Account>,
}

/// A client's share of a feed, while it reads it: bytes charged by the
/// waker, and written by the writer. What is due and not written counts
/// against its budget.
pub struct Account {
    due: AtomicU64,
    done: AtomicU64,
    /// Still read: false once the client has left the feed.
    open: AtomicBool,
}

impl Default for Account {
    fn default() -> Self {
        Self {
            due: AtomicU64::new(0),
            done: AtomicU64::new(0),
            open: AtomicBool::new(true),
        }
    }
}

impl Account {
    fn lag(&self) -> usize {
        let done = self.done.load(Ordering::Acquire);
        self.due.load(Ordering::Acquire).saturating_sub(done) as usize
    }
}

/// A batch of a map's broadcasts, and what was sent to the feed's clients
/// alone among them, by index, each before the broadcast its `u32` numbers
/// (after the last, if past it).
pub struct Chunk {
    n: u64,
    log: Vec<Broadcast>,
    directs: Directs,
    next: std::sync::OnceLock<Arc<Chunk>>,
}

impl Chunk {
    fn first() -> Arc<Chunk> {
        Arc::new(Chunk {
            n: 0,
            log: Vec::new(),
            directs: Default::default(),
            next: std::sync::OnceLock::new(),
        })
    }
}

/// A long chain let go a link at a time, not by recursion.
impl Drop for Chunk {
    fn drop(&mut self) {
        let mut next = self.next.take();
        while let Some(n) = next {
            match Arc::try_unwrap(n) {
                Ok(mut c) => next = c.next.take(),
                Err(_) => break,
            }
        }
    }
}

impl std::fmt::Debug for Feed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Feed")
            .field("id", &self.id)
            .field("published", &self.published.load(Ordering::Relaxed))
            .finish()
    }
}

impl Feed {
    /// A feed for the table `id`.
    pub fn new(id: usize) -> Arc<Feed> {
        let first = Chunk::first();
        Arc::new(Feed {
            id,
            tail: parking_lot::Mutex::new(first.clone()),
            published: AtomicU64::new(0),
            clients: Default::default(),
            signalled: AtomicBool::new(false),
            waker_at: parking_lot::Mutex::new(first),
        })
    }

    pub fn id(&self) -> usize {
        self.id
    }

    /// The number of the last chunk published.
    pub fn published(&self) -> u64 {
        self.published.load(Ordering::Acquire)
    }

    /// Publish a chunk, and tell the waker.
    fn publish(self: &Arc<Self>, log: Vec<Broadcast>, directs: Directs) {
        if log.is_empty() && directs.is_empty() {
            return;
        }
        let mut tail = self.tail.lock();
        let chunk = Arc::new(Chunk {
            n: tail.n + 1,
            log,
            directs,
            next: std::sync::OnceLock::new(),
        });
        let _ = tail.next.set(chunk.clone());
        self.published.store(chunk.n, Ordering::Release);
        *tail = chunk;
        drop(tail);
        if !self.signalled.swap(true, Ordering::AcqRel) {
            waker::signal(self.clone());
        }
    }

    /// `out` is the client `me` of this feed's map from now on: what it is
    /// sent from here waits for the chunks before it.
    fn subscribe(self: &Arc<Self>, me: usize, out: &Outbound) {
        let inner = &out.inner;
        let mut sub = inner.sub.lock();
        if let Some(s) = sub.as_ref() {
            if Arc::ptr_eq(&s.feed, self) && s.me == me {
                return;
            }
            // Another map's still: its chunks so far, then this one's.
            s.account.open.store(false, Ordering::Release);
            let _ = inner.tx.send(Item::Unsub(s.feed.published()));
        }
        let after = self.tail.lock().clone();
        let account = Arc::new(Account::default());
        let n = after.n;
        let _ = inner.tx.send(Item::Sub {
            feed: self.clone(),
            me,
            after,
            account: account.clone(),
        });
        *sub = Some(Subscription {
            feed: self.clone(),
            me,
            account: account.clone(),
        });
        let holding = inner.holding.load(Ordering::Acquire);
        drop(sub);
        let mut c = self.clients.lock();
        let subs = Arc::make_mut(&mut c.subs);
        if subs.len() <= me {
            subs.resize(me + 1, None);
        }
        subs[me] = Some(Reader {
            inner: Arc::downgrade(inner),
            after: n,
            account,
        });
        c.subscribed.insert(me);
        c.gone.remove(me);
        if holding {
            c.held.insert(me);
        } else {
            c.held.remove(me);
        }
    }

    /// The index `me` is let go (its player has left the map, or their
    /// client is another): its client reads the chunks published so far,
    /// and no more.
    pub fn unsubscribe(&self, me: usize) {
        let reader = {
            let mut c = self.clients.lock();
            c.subscribed.remove(me);
            c.held.remove(me);
            c.gone.remove(me);
            if c.subs.get(me).is_some_and(Option::is_some) {
                Arc::make_mut(&mut c.subs)[me].take()
            } else {
                None
            }
        };
        let Some(reader) = reader else {
            return;
        };
        reader.account.open.store(false, Ordering::Release);
        let Some(inner) = reader.inner.upgrade() else {
            return;
        };
        let mut sub = inner.sub.lock();
        if sub
            .as_ref()
            .is_some_and(|s| std::ptr::eq(Arc::as_ptr(&s.feed), self) && s.me == me)
        {
            let _ = inner.tx.send(Item::Unsub(self.published()));
            *sub = None;
        }
    }

    /// `i` is a client's again (a player given the index, or back in the
    /// map): looked for at the next broadcast that reaches it.
    pub fn found(&self, i: usize) {
        self.clients.lock().gone.remove(i);
    }

    /// The waker's turn: each chunk since its last, each client charged for
    /// its share and woken if it has one.
    fn wake(&self) {
        let mut at = self.waker_at.lock();
        // Each chunk's share, by index and by the chunk's number: a client
        // is charged only for the chunks after the one it reads after.
        let mut shares: Vec<(u64, Vec<(usize, u64)>)> = Vec::new();
        let mut due: Vec<u64> = Vec::new();
        let mut touched = Clients::default();
        while let Some(next) = at.next.get().cloned() {
            due.clear();
            for b in &next.log {
                let n = b.frame.len() as u64;
                for i in b.to.iter() {
                    if due.len() <= i {
                        due.resize(i + 1, 0);
                    }
                    due[i] += n;
                }
            }
            let share: Vec<(usize, u64)> = due
                .iter()
                .enumerate()
                .filter(|(_, d)| **d > 0)
                .map(|(i, d)| (i, *d))
                .collect();
            for &(i, _) in &share {
                touched.insert(i);
            }
            for &i in next.directs.keys() {
                touched.insert(i);
            }
            shares.push((next.n, share));
            *at = next;
        }
        drop(at);
        // Taken after the chunks: a client subscribed before the chunk that
        // first reached it was published.
        let subs = self.clients.lock().subs.clone();
        let mut charge = vec![0u64; subs.len()];
        for (n, share) in &shares {
            for &(i, d) in share {
                if let Some(Some(r)) = subs.get(i)
                    && *n > r.after
                {
                    charge[i] += d;
                }
            }
        }
        for i in touched.iter() {
            let Some(r) = subs.get(i).and_then(Option::as_ref) else {
                continue;
            };
            if !r.account.open.load(Ordering::Acquire) {
                continue;
            }
            let Some(inner) = r.inner.upgrade() else {
                continue;
            };
            if inner.shared.closed.load(Ordering::Acquire) {
                continue;
            }
            let d = charge.get(i).copied().unwrap_or(0);
            if d > 0 {
                r.account.due.fetch_add(d, Ordering::AcqRel);
                let queued = inner.shared.queued.load(Ordering::Relaxed);
                if queued.saturating_add(r.account.lag()) > inner.shared.capacity {
                    Outbound { inner }.overflow();
                    continue;
                }
            }
            inner.shared.ready.notify_one();
        }
    }
}

/// The thread that charges and wakes a feed's clients once it publishes.
mod waker {
    use super::Feed;
    use std::sync::{Arc, OnceLock, mpsc};

    static TX: OnceLock<parking_lot::Mutex<mpsc::Sender<Arc<Feed>>>> = OnceLock::new();

    pub(super) fn signal(feed: Arc<Feed>) {
        let tx = TX.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Arc<Feed>>();
            std::thread::Builder::new()
                .name("feed waker".into())
                .spawn(move || {
                    while let Ok(feed) = rx.recv() {
                        let _z = crate::zone!("feed: wake");
                        feed.signalled
                            .store(false, std::sync::atomic::Ordering::Release);
                        feed.wake();
                    }
                })
                .expect("the feed's waker");
            parking_lot::Mutex::new(tx)
        });
        let _ = tx.lock().send(feed);
    }
}

/// A client's place in a feed, as its senders know it.
struct Subscription {
    feed: Arc<Feed>,
    me: usize,
    account: Arc<Account>,
}

/// While one is alive, what this thread queues for a client waits, in the
/// order it was sent, and goes when the last cork on the thread is dropped
/// (or at [`uncork`]): a map's run of movement packets and its update are
/// one chunk of its feed ([`Feed`]), with what it sent each of its clients
/// alone among the broadcasts, and each client's writer takes its share:
/// the map's cost is a packet's, and a chunk's a batch, not a hand-off to
/// each client. What it sends the clients of no feed, or of another, goes
/// to each in one push. Only the push waits: the budget is taken (a
/// broadcast's by the waker, once published), a hold keeps what it keeps
/// and a closed client takes nothing, as uncorked. What other threads send
/// isn't corked, so a map uncorks before it hands a client, or word of one,
/// to another task (see `map::run`).
pub struct Cork {
    /// When what it holds was first parked ([`Cork::park`]).
    since: Option<std::time::Instant>,
    _not_send: std::marker::PhantomData<*const ()>,
}

impl Cork {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        CORK.with_borrow_mut(|c| c.depth += 1);
        CORKED.set(true);
        Cork {
            since: None,
            _not_send: std::marker::PhantomData,
        }
    }

    /// The feed of the map whose cork this is: what the cork keeps for its
    /// clients goes in its chunk. Another feed's, kept so far, is pushed
    /// first.
    pub fn feed(&self, feed: &Arc<Feed>) {
        set_feed(feed);
    }

    /// A cork again over what was parked: what it held stays, and what is
    /// sent now goes after it. Under another cork, what was parked is
    /// pushed first.
    pub fn resume(parked: Parked) -> Self {
        if CORKED.get() {
            drop(parked);
            return Cork::new();
        }
        let mut parked = parked;
        let since = parked.since;
        let state = std::mem::take(&mut *parked.state);
        CORK.with_borrow_mut(|c| {
            let depth = c.depth;
            *c = state;
            c.depth = depth;
        });
        let mut cork = Cork::new();
        cork.since = Some(since);
        cork
    }

    /// Stop corking without pushing: what the cork holds is kept for a
    /// later cork on whichever thread ([`Cork::resume`]), or pushed when
    /// the parked state is dropped. None if it holds nothing, or another
    /// cork on the thread is alive (which keeps it).
    pub fn park(self) -> Option<Parked> {
        let me = std::mem::ManuallyDrop::new(self);
        let since = me.since.unwrap_or_else(std::time::Instant::now);
        let state = CORK.with_borrow_mut(|c| {
            c.depth -= 1;
            if c.depth > 0 {
                return None;
            }
            let state = std::mem::take(c);
            Some(state)
        });
        let state = state?;
        CORKED.set(false);
        if state.is_empty() {
            return None;
        }
        Some(Parked {
            state: Box::new(state),
            since,
        })
    }
}

impl Drop for Cork {
    fn drop(&mut self) {
        let last = CORK.with_borrow_mut(|c| {
            c.depth -= 1;
            c.depth == 0
        });
        if last {
            uncork();
            CORKED.set(false);
        }
    }
}

/// What a cork held, parked between a map's bursts ([`Cork::park`]): sent
/// on with a later cork, or pushed when dropped.
pub struct Parked {
    state: Box<CorkState>,
    since: std::time::Instant,
}

impl Parked {
    /// When it was first parked.
    pub fn since(&self) -> std::time::Instant {
        self.since
    }
}

impl Drop for Parked {
    fn drop(&mut self) {
        push(&mut self.state);
    }
}

/// Push what this thread's cork holds now, the cork staying on.
pub fn uncork() {
    // Uncorked, nothing waits: the last cork's drop pushed it.
    if !CORKED.get() {
        return;
    }
    let mut state = CORK.with_borrow_mut(|c| {
        let depth = c.depth;
        let state = std::mem::take(c);
        c.depth = depth;
        // The cork goes on for the same map.
        c.feed = state.feed.clone();
        state
    });
    push(&mut state);
}

/// Push what `state` holds: what went to clients outside its feed, each
/// client's in one go, then its feed's chunk.
fn push(state: &mut CorkState) {
    if state.is_empty() {
        return;
    }
    let _z = crate::zone!("cork: push");
    // Before the chunk: what they were sent came before its broadcasts (a
    // client of the feed is in a slot only for what it was sent before
    // the cork's first broadcast to it).
    for slot in std::mem::take(&mut state.slots) {
        let inner = slot.inner;
        if inner.shared.closed.load(Ordering::Acquire) || slot.frames.is_empty() {
            continue;
        }
        let sub = inner.sub.lock();
        let stamp = sub.as_ref().map_or(0, |s| s.feed.published());
        let _ = inner.tx.send(Item::Corked(slot.frames, stamp));
        drop(sub);
    }
    state.by_address.clear();
    let log = std::mem::take(&mut state.log);
    let directs = std::mem::take(&mut state.directs);
    if let Some(feed) = state.feed.take() {
        feed.publish(log, directs);
    }
}

/// Send `frame` to the feed's clients `to`, `client` naming the queue of
/// each by its index (none for one no longer there), the first time the
/// feed reaches it. Corked, it goes in the cork's chunk, once, with the
/// set; uncorked, in a chunk of its own, at once. A held client's goes
/// into its hold at once.
pub fn broadcast(
    feed: &Arc<Feed>,
    frame: &Arc<[u8]>,
    to: &Arc<Clients>,
    client: impl Fn(usize) -> Option<Outbound>,
) {
    let _z = crate::zone!("cork: broadcast");
    // Those it reaches for the first time: theirs from now on.
    let new: Vec<usize> = {
        let c = feed.clients.lock();
        if to.beyond(&c.subscribed, &c.gone) {
            to.without(&c.subscribed).without(&c.gone).iter().collect()
        } else {
            Vec::new()
        }
    };
    for i in new {
        match client(i) {
            Some(out) => feed.subscribe(i, &out),
            None => {
                feed.clients.lock().gone.insert(i);
            }
        }
    }
    // Into their holds, behind what waits there.
    let (to, held) = {
        let c = feed.clients.lock();
        if to.meets(&c.held) {
            let held: Vec<Arc<OutboundInner>> = to
                .iter()
                .filter(|&i| c.held.contains(i))
                .filter_map(|i| c.subs.get(i)?.as_ref()?.inner.upgrade())
                .collect();
            (Arc::new(to.without(&c.held)), held)
        } else {
            (to.clone(), Vec::new())
        }
    };
    for inner in held {
        Outbound { inner }.send_frame(frame.clone());
    }
    if to.is_empty() {
        return;
    }
    let b = Broadcast {
        frame: frame.clone(),
        to,
    };
    if !CORKED.get() {
        feed.publish(vec![b], Default::default());
        return;
    }
    set_feed(feed);
    CORK.with_borrow_mut(|c| c.log.push(b));
}

/// The cork's feed `feed`: another's, kept so far, pushed first.
fn set_feed(feed: &Arc<Feed>) {
    if !CORKED.get() {
        return;
    }
    let other = CORK.with_borrow(|c| c.feed.as_ref().is_some_and(|f| !Arc::ptr_eq(f, feed)));
    if other {
        // Another map's feed: what was kept for it goes first.
        uncork();
    }
    CORK.with_borrow_mut(|c| {
        if c.feed.is_none() || other {
            c.feed = Some(feed.clone());
        }
    });
}

thread_local! {
    static CORKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static CORK: std::cell::RefCell<CorkState> = std::cell::RefCell::new(CorkState::default());
}

/// A client corked outside the cork's feed: what was sent to it alone.
struct Slot {
    inner: Arc<OutboundInner>,
    frames: Vec<Arc<[u8]>>,
}

#[derive(Default)]
struct CorkState {
    depth: usize,
    /// The feed the broadcasts go to: the first's.
    feed: Option<Arc<Feed>>,
    /// The broadcasts, in order.
    log: Vec<Broadcast>,
    /// What the feed's clients were sent alone, by index: each before the
    /// broadcast its `u32` numbers.
    directs: Directs,
    /// Each other client corked, in the order first sent to.
    slots: Vec<Slot>,
    /// A client's slot, by its queue's address.
    by_address: ahash::AHashMap<usize, usize>,
}

impl CorkState {
    fn is_empty(&self) -> bool {
        self.log.is_empty() && self.directs.is_empty() && self.slots.is_empty()
    }

    fn push(&mut self, inner: &Arc<OutboundInner>, sub: Option<&Subscription>, frame: Arc<[u8]>) {
        if let (Some(s), Some(feed)) = (sub, &self.feed)
            && Arc::ptr_eq(&s.feed, feed)
        {
            let k = self.log.len() as u32;
            self.directs.entry(s.me).or_default().push((k, frame));
            return;
        }
        let key = Arc::as_ptr(inner) as usize;
        let s = match self.by_address.get(&key) {
            Some(&s) => s,
            None => {
                self.slots.push(Slot {
                    inner: inner.clone(),
                    frames: Vec::new(),
                });
                self.by_address.insert(key, self.slots.len() - 1);
                self.slots.len() - 1
            }
        };
        self.slots[s].frames.push(frame);
    }
}

/// Where packets for one client go: a queue the writer task drains, with
/// a byte budget. A client too slow to keep its queue under the budget is
/// disconnected: dropping some of its packets would leave it out of step
/// with the world.
#[derive(Clone)]
pub struct Outbound {
    inner: Arc<OutboundInner>,
}

struct OutboundInner {
    /// tokio's unbounded `mpsc`: its send is sync, from any thread, and
    /// the writer takes a batch at a time (`recv_many`).
    tx: mpsc::UnboundedSender<Item>,
    shared: Arc<Shared>,
    overflowed: AtomicBool,
    /// Frames wait in `held` (see [`Outbound::hold`]).
    holding: AtomicBool,
    held: parking_lot::Mutex<Vec<Arc<[u8]>>>,
    /// The connection's order counter and the forced movements it hasn't
    /// acknowledged (see [`Outbound::order`]).
    orders: parking_lot::Mutex<Orders>,
    /// The map's feed it reads, while it reads one ([`Feed`]): what is
    /// sent it is stamped under this lock, so the stamps follow its `Sub`
    /// and come before its `Unsub`.
    sub: parking_lot::Mutex<Option<Subscription>>,
    /// The account is a GM's: the ping rule spares it ([`PingRate`]), as
    /// mangos's `HandlePing` spares all but `SEC_PLAYER`.
    gm: AtomicBool,
}

/// A forced movement a client has been sent and hasn't acknowledged, by
/// the unit it moves (Thistle Tea's `pending_movement_acks`): the client's
/// acknowledgement settles it only if it names the same unit and change.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Order {
    Root {
        guid: Guid,
        root: bool,
    },
    Speed {
        guid: Guid,
        kind: SpeedKind,
        speed: f32,
    },
    WaterWalk {
        guid: Guid,
        on: bool,
    },
    FeatherFall {
        guid: Guid,
        on: bool,
    },
    Hover {
        guid: Guid,
        on: bool,
    },
    /// The launch: its direction's cosine and sine, and its horizontal and
    /// vertical speeds, as sent.
    KnockBack {
        guid: Guid,
        impulse: [f32; 4],
    },
    Teleport {
        guid: Guid,
    },
}

/// Which speed a forced speed change sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedKind {
    Run,
    RunBack,
    Swim,
    SwimBack,
    Walk,
    TurnRate,
}

/// How many orders wait for their acknowledgement at most: past it, the
/// oldest goes (a client that never acknowledges keeps no more).
const MAX_PENDING_ORDERS: usize = 64;

#[derive(Debug, Default)]
struct Orders {
    next: u32,
    pending: std::collections::VecDeque<(u32, Order)>,
}

impl Order {
    /// What it moves the client somewhere else with: a teleport, a launch.
    fn relocates(&self) -> bool {
        matches!(self, Order::Teleport { .. } | Order::KnockBack { .. })
    }
}

/// What a queue's two ends share.
struct Shared {
    /// Bytes queued and not yet taken by the writer, which takes back what
    /// it takes: a plain counter (nothing waits on it), one atomic add a
    /// frame.
    queued: AtomicUsize,
    /// The budget's bytes, all of them.
    capacity: usize,
    /// [`Outbound::close`] was called. tokio's senders can't close their
    /// channel, so from then on the queue's far end takes nothing more, and
    /// what was queued is dropped unwritten.
    closed: AtomicBool,
    /// Signalled at the close, for whoever waits on it
    /// ([`Shared::wait_closed`]): the writer, in a write that a client
    /// which doesn't read holds up; the reader, in its read; the session.
    closing: Notify,
    /// The feed's waker: a chunk has something for this client.
    ready: Notify,
}

impl Shared {
    /// Close, and wake whoever waits on it: true if it wasn't closed.
    fn close(&self) -> bool {
        if self.closed.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.closing.notify_waiters();
        true
    }

    /// Completes once the queue is closed.
    async fn wait_closed(&self) {
        let notified = self.closing.notified();
        let mut notified = std::pin::pin!(notified);
        // Waiting from here, so a close between the check and the wait
        // still wakes it.
        notified.as_mut().enable();
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        notified.await;
    }
}

impl std::fmt::Debug for Outbound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Outbound")
            .field("queued", &self.inner.shared.queued.load(Ordering::Relaxed))
            .finish()
    }
}

/// The far end of an [`Outbound`]: what the writer task drains, or what a
/// test reads ([`Outbound::detached`]): its queue, and the chunks of the
/// map's feed it reads ([`Feed`]), merged in the order sent. Once the
/// queue is closed it hands out nothing more.
pub struct Frames {
    rx: mpsc::UnboundedReceiver<Item>,
    /// Frames taken and not yet handed out, each with whether its bytes are
    /// the queue's (what was sent to it alone) rather than the feed's.
    batch: std::collections::VecDeque<(Arc<[u8]>, bool)>,
    shared: Arc<Shared>,
    /// The feed it reads, while it reads one.
    reading: Option<Reading>,
    /// What a wait took from the queue, first at the next fill.
    first: Option<Item>,
    /// Every sender has gone.
    ended: bool,
}

/// A client's place in its feed.
struct Reading {
    feed: Arc<Feed>,
    me: usize,
    /// The last chunk read.
    at: Arc<Chunk>,
    account: Arc<Account>,
}

impl Frames {
    fn closed(&self) -> bool {
        self.shared.closed.load(Ordering::Acquire)
    }

    /// Take what there is, in order: the feed's last chunk is looked at
    /// first, then the queue drained (so that what the map sent before it
    /// published is in hand), each frame after the chunks it was stamped
    /// with, then the feed's chunks up to the one looked at.
    fn fill(&mut self) {
        loop {
            let head = self.reading.as_ref().map(|r| r.feed.published());
            let mut switched = false;
            if let Some(item) = self.first.take() {
                switched = self.take(item);
            }
            while !switched {
                match self.rx.try_recv() {
                    Ok(item) => switched = self.take(item),
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        self.ended = true;
                        break;
                    }
                }
            }
            // Another feed, or none: its last chunk looked at again.
            if switched {
                continue;
            }
            if let Some(h) = head {
                self.read_to(h);
            }
            return;
        }
    }

    /// One item from the queue: true if it changes the feed read.
    fn take(&mut self, item: Item) -> bool {
        match item {
            Item::One(f, stamp) => {
                self.read_to(stamp);
                self.batch.push_back((f, true));
                false
            }
            Item::Corked(frames, stamp) => {
                self.read_to(stamp);
                self.batch.extend(frames.into_iter().map(|f| (f, true)));
                false
            }
            Item::Sub {
                feed,
                me,
                after,
                account,
            } => {
                self.reading = Some(Reading {
                    feed,
                    me,
                    at: after,
                    account,
                });
                true
            }
            Item::Unsub(until) => {
                self.read_to(until);
                self.reading = None;
                true
            }
        }
    }

    /// The feed's chunks up to the one numbered `target`: the broadcasts to
    /// this client, among what was sent to it alone.
    fn read_to(&mut self, target: u64) {
        let Some(r) = &mut self.reading else {
            return;
        };
        let batch = &mut self.batch;
        let mut taken = 0u64;
        while r.at.n < target {
            let Some(next) = r.at.next.get().cloned() else {
                break;
            };
            let mut directs = next.directs.get(&r.me).map(|d| d.iter().peekable());
            for (k, b) in next.log.iter().enumerate() {
                if let Some(d) = &mut directs {
                    while let Some((_, f)) = d.next_if(|(seq, _)| *seq as usize <= k) {
                        batch.push_back((f.clone(), true));
                    }
                }
                if b.to.contains(r.me) {
                    taken += b.frame.len() as u64;
                    batch.push_back((b.frame.clone(), false));
                }
            }
            if let Some(d) = directs {
                batch.extend(d.map(|(_, f)| (f.clone(), true)));
            }
            r.at = next;
        }
        if taken > 0 {
            r.account.done.fetch_add(taken, Ordering::AcqRel);
        }
    }

    /// Wait for something to take: an item in the queue, or the waker's
    /// word of a chunk. `None` once the queue is closed.
    async fn wait(&mut self) -> Option<()> {
        let ready = self.shared.ready.notified();
        tokio::select! {
            biased;
            () = self.shared.wait_closed() => None,
            item = self.rx.recv() => {
                match item {
                    Some(item) => self.first = Some(item),
                    None => self.ended = true,
                }
                Some(())
            }
            () = ready => Some(()),
        }
    }

    /// The next frame, once there is one; `None` once the queue is closed
    /// or every sender has gone.
    pub async fn recv(&mut self) -> Option<Arc<[u8]>> {
        loop {
            if self.closed() {
                return None;
            }
            if self.batch.is_empty() {
                self.fill();
            }
            if let Some((f, _)) = self.batch.pop_front() {
                return Some(f);
            }
            if self.ended {
                return None;
            }
            self.wait().await?;
        }
    }

    /// The next frame, if one is queued.
    pub fn try_recv(&mut self) -> Option<Arc<[u8]>> {
        if self.closed() {
            return None;
        }
        if self.batch.is_empty() {
            self.fill();
        }
        if self.closed() {
            return None;
        }
        self.batch.pop_front().map(|(f, _)| f)
    }

    /// Frames queued and not yet taken.
    pub fn len(&mut self) -> usize {
        if self.closed() {
            return 0;
        }
        self.fill();
        self.batch.len()
    }

    pub fn is_empty(&mut self) -> bool {
        self.len() == 0
    }
}

/// An outbound queue with `budget` bytes.
fn queue(budget: usize) -> (Outbound, Frames) {
    let (tx, rx) = mpsc::unbounded_channel();
    let shared = Arc::new(Shared {
        queued: AtomicUsize::new(0),
        capacity: budget,
        closed: AtomicBool::new(false),
        closing: Notify::new(),
        ready: Notify::new(),
    });
    let outbound = Outbound {
        inner: Arc::new(OutboundInner {
            tx,
            shared: shared.clone(),
            overflowed: AtomicBool::new(false),
            holding: AtomicBool::new(false),
            held: parking_lot::Mutex::new(Vec::new()),
            orders: parking_lot::Mutex::new(Orders::default()),
            sub: parking_lot::Mutex::new(None),
            gm: AtomicBool::new(false),
        }),
    };
    (
        outbound,
        Frames {
            rx,
            batch: Default::default(),
            shared,
            reading: None,
            first: None,
            ended: false,
        },
    )
}

impl Outbound {
    /// The counter for the next forced movement this client is sent (a
    /// root, a forced speed, water walking, a knockback, a teleport): 0
    /// first, then one more each, as mangos-classic's session numbers its
    /// orders (`GetOrderCounter`, `IncrementOrderCounter`) and Thistle
    /// Tea's `movement_counter` stamps them, so the client's
    /// acknowledgement names the order it answers.
    pub fn next_order(&self) -> u32 {
        let mut o = self.inner.orders.lock();
        let counter = o.next;
        o.next = counter.wrapping_add(1);
        counter
    }

    /// [`Outbound::next_order`] for `order`, kept until the client
    /// acknowledges it ([`Outbound::settle`]). A teleport makes the
    /// teleports and launches before it stale, as Thistle Tea's
    /// `MovementControl` drops them: their acknowledgements name a place
    /// the client is no longer bound for.
    pub fn order(&self, order: Order) -> u32 {
        let mut o = self.inner.orders.lock();
        let counter = o.next;
        o.next = counter.wrapping_add(1);
        if matches!(order, Order::Teleport { .. }) {
            o.pending.retain(|(_, p)| !p.relocates());
        }
        if o.pending.len() >= MAX_PENDING_ORDERS {
            o.pending.pop_front();
        }
        o.pending.push_back((counter, order));
        counter
    }

    /// The client acknowledges the order `counter`: settled, and returned,
    /// if one is waiting under it and `matches` it; else nothing changes
    /// (a stale, repeated or forged acknowledgement).
    pub fn settle(&self, counter: u32, matches: impl FnOnce(&Order) -> bool) -> Option<Order> {
        let mut o = self.inner.orders.lock();
        let i = o.pending.iter().position(|(c, _)| *c == counter)?;
        if !matches(&o.pending[i].1) {
            return None;
        }
        o.pending.remove(i).map(|(_, order)| order)
    }

    /// The orders waiting go: the client is sent to another map
    /// (`SMSG_NEW_WORLD`), where none of them applies, as Thistle Tea
    /// clears them there. The counter goes on.
    pub fn forget_orders(&self) {
        self.inner.orders.lock().pending.clear();
    }

    /// The orders waiting for their acknowledgement, oldest first.
    pub fn pending_orders(&self) -> Vec<(u32, Order)> {
        self.inner.orders.lock().pending.iter().copied().collect()
    }

    /// A queue nothing drains, with an unlimited budget, and the receiving
    /// end for tests to read.
    pub fn detached() -> (Self, Frames) {
        queue(usize::MAX >> 1)
    }

    /// A queue with `budget` bytes, and its far end, for a writer
    /// ([`run_writer`]) or a test or benchmark to drain.
    pub fn with_budget(budget: usize) -> (Self, Frames) {
        queue(budget)
    }

    /// Queue a frame the budget has been taken for: false if the client is
    /// gone.
    fn enqueue(&self, frame: Arc<[u8]>) -> bool {
        if self.inner.shared.closed.load(Ordering::Acquire) {
            return false;
        }
        let sub = self.inner.sub.lock();
        if CORKED.get() {
            CORK.with_borrow_mut(|c| c.push(&self.inner, sub.as_ref(), frame));
            return !self.inner.tx.is_closed();
        }
        let stamp = sub.as_ref().map_or(0, |s| s.feed.published());
        self.inner.tx.send(Item::One(frame, stamp)).is_ok()
    }

    /// The bytes of the feed's broadcasts due to the client and not yet
    /// taken by its writer.
    fn lag(&self) -> usize {
        self.inner
            .sub
            .lock()
            .as_ref()
            .map_or(0, |s| s.account.lag())
    }

    /// Queue a framed packet. False if the client is gone or over budget;
    /// over budget, its connection is closed.
    pub fn send_frame(&self, frame: Arc<[u8]>) -> bool {
        if self.inner.holding.load(Ordering::Acquire) {
            let n = frame.len();
            let mut held = self.inner.held.lock();
            // Not released since: it waits behind what is held.
            if self.inner.holding.load(Ordering::Acquire) {
                if !self.take_budget(n) {
                    drop(held);
                    self.overflow();
                    return false;
                }
                held.push(frame);
                return !self.is_closed();
            }
        }
        self.send_frame_unheld(frame)
    }

    /// Queue a frame past any hold: for what names no object a create could
    /// be waiting for, and is waited on (a pong, a query's answer).
    pub fn send_frame_unheld(&self, frame: Arc<[u8]>) -> bool {
        if !self.take_budget(frame.len()) {
            self.overflow();
            return false;
        }
        self.enqueue(frame)
    }

    /// `n` bytes of the budget for a frame: false, and nothing taken, if
    /// they aren't left.
    fn take_budget(&self, n: usize) -> bool {
        let shared = &self.inner.shared;
        let lag = self.lag();
        let before = shared.queued.fetch_add(n, Ordering::Relaxed);
        if before.saturating_add(n).saturating_add(lag) <= shared.capacity {
            return true;
        }
        shared.queued.fetch_sub(n, Ordering::Relaxed);
        false
    }

    /// Over budget: the client is disconnected, and counted once.
    fn overflow(&self) {
        if !self.inner.overflowed.swap(true, Ordering::Relaxed) {
            crate::game::stats::BUDGET.fetch_add(1, Ordering::Relaxed);
            warn!("a client's outbound queue is over budget; disconnecting it");
        }
        self.close();
    }

    /// From now on, what is sent waits, in the order it was sent, for
    /// [`Outbound::release`]. A map holds a client it has queued creates
    /// for, so that nothing it sends naming those objects arrives before
    /// them. True if the client wasn't held already.
    pub fn hold(&self) -> bool {
        let fresh = !self.inner.holding.swap(true, Ordering::AcqRel);
        if fresh && let Some(s) = self.inner.sub.lock().as_ref() {
            s.feed.clients.lock().held.insert(s.me);
        }
        fresh
    }

    /// Whether frames are waiting on a hold.
    pub fn has_held(&self) -> bool {
        self.inner.holding.load(Ordering::Acquire) && !self.inner.held.lock().is_empty()
    }

    /// Queue `first` (the blocks the hold was for), then what waited; what
    /// is sent from now on goes at once again.
    pub fn release(&self, first: Vec<Arc<[u8]>>) {
        let mut held = self.inner.held.lock();
        for frame in first {
            self.send_frame_unheld(frame);
        }
        for frame in held.drain(..) {
            self.enqueue(frame);
        }
        self.inner.holding.store(false, Ordering::Release);
        if let Some(s) = self.inner.sub.lock().as_ref() {
            s.feed.clients.lock().held.remove(s.me);
        }
    }

    pub fn send<M: ServerMessage>(&self, m: &M) -> bool {
        match encode(m) {
            Some(frame) => self.send_frame(frame),
            None => false,
        }
    }

    /// [`Outbound::send`] past any hold ([`Outbound::send_frame_unheld`]).
    pub fn send_unheld<M: ServerMessage>(&self, m: &M) -> bool {
        match encode(m) {
            Some(frame) => self.send_frame_unheld(frame),
            None => false,
        }
    }

    /// Close the queue: nothing more is queued, what is queued and not yet
    /// taken is dropped, the writer task ends at once (a write under way
    /// left unfinished), the reader and the session end, and the
    /// connection closes.
    pub fn close(&self) {
        self.inner.shared.close();
    }

    /// Completes once the queue is closed or its writer has gone: what the
    /// reader and the session wait on beside their own work, to end with
    /// their connection.
    pub async fn closed(&self) {
        tokio::select! {
            () = self.inner.shared.wait_closed() => {}
            () = self.inner.tx.closed() => {}
        }
    }

    /// Bytes queued for the writer, not yet written.
    pub fn queued_bytes(&self) -> usize {
        self.inner
            .shared
            .queued
            .load(Ordering::Relaxed)
            .saturating_add(self.lag())
    }

    /// The account is a GM's (or isn't): its reader spares it the ping
    /// rule.
    pub fn set_gm(&self, gm: bool) {
        self.inner.gm.store(gm, Ordering::Relaxed);
    }

    pub fn is_gm(&self) -> bool {
        self.inner.gm.load(Ordering::Relaxed)
    }

    /// Closed, or the writer has gone.
    pub fn is_closed(&self) -> bool {
        self.inner.shared.closed.load(Ordering::Acquire) || self.inner.tx.is_closed()
    }

    /// Two handles to the same client.
    pub fn same(&self, other: &Outbound) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

/// A packet a client sent: one wow_messages reads, or (for those it lays
/// out differently from 1.12) its opcode and body.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)] // Each packet moves through the channel once.
pub enum Inbound {
    Msg(ClientOpcodeMessage),
    Raw(u16, Vec<u8>),
}

impl From<ClientOpcodeMessage> for Inbound {
    fn from(m: ClientOpcodeMessage) -> Self {
        Inbound::Msg(m)
    }
}

/// The client opcodes read by hand: `CMSG_SET_FACTION_ATWAR`,
/// `CMSG_SET_FACTION_INACTIVE` and `CMSG_SET_WATCHED_FACTION` carry a
/// 32-bit reputation index where wow_messages reads a faction,
/// wow_messages's decompression of `CMSG_UPDATE_ACCOUNT_DATA` fails, it
/// wants ten answers in `CMSG_GMSURVEY_SUBMIT` where the client stops
/// at a zero question.
pub const RAW_OPCODES: &[u32] = &[0x125, 0x317, 0x318, 0x20B, 0x32A];

/// A started connection: its outbound queue, and the packets it reads.
pub struct Connection {
    pub outbound: Outbound,
    pub inbound: mpsc::Receiver<Inbound>,
}

/// Start the reader and writer tasks for an authenticated stream.
pub fn start(stream: TcpStream, crypto: HeaderCrypto) -> Connection {
    let _ = stream.set_nodelay(true);
    let (read, write) = stream.into_split();
    let (encrypter, decrypter) = crypto.split();
    let network = &crate::config::config().network;
    let (outbound, frames) = queue(network.outbound_channel_bytes);
    // A client that takes nothing for as long as one may send nothing is
    // gone.
    tokio::spawn(run_writer(
        write,
        encrypter,
        frames,
        network.client_idle_timeout(),
    ));
    let (in_tx, in_rx) = mpsc::channel(256);
    tokio::spawn(run_reader(read, decrypter, in_tx, outbound.clone()));
    Connection {
        outbound,
        inbound: in_rx,
    }
}

/// Decode packets and pass them on. Pings are answered here, so their
/// round trip measures the network and not the world, and counted here
/// ([`PingRate`]).
pub(crate) async fn run_reader<R: AsyncRead + Unpin + Send>(
    mut read: R,
    mut decrypter: DecrypterHalf,
    tx: mpsc::Sender<Inbound>,
    outbound: Outbound,
) {
    let network = &crate::config::config().network;
    let idle = network.client_idle_timeout();
    let mut bad = BadPacketLimit::new();
    let mut pings = PingRate::new(network.max_overspeed_pings);
    loop {
        // Closed (over budget, taken over, its map gone), the connection
        // ends now, its socket with it: not at the client's next packet.
        let read_one = tokio::time::timeout(idle, read_packet(&mut read, &mut decrypter));
        let got = tokio::select! {
            biased;
            () = outbound.closed() => break,
            got = read_one => got,
        };
        let Ok(msg) = got else {
            debug!("client sent nothing for {idle:?}; disconnecting");
            break;
        };
        let msg = match msg {
            Ok(Some(Inbound::Msg(m))) => m,
            // wow_messages panicked reading it.
            Ok(None) => {
                if bad.exceeded() {
                    break;
                }
                continue;
            }
            Ok(Some(raw)) => {
                PACKETS_IN.fetch_add(1, Ordering::Relaxed);
                if tx.send(raw).await.is_err() {
                    break;
                }
                continue;
            }
            // wow_messages reads a packet's whole body before dispatching on
            // its opcode, so an unknown or unparseable packet is already
            // consumed: skip it.
            Err(ExpectedOpcodeError::Opcode { opcode, size, name }) => {
                debug!("unhandled opcode {name:?} (0x{opcode:X}, {size} bytes)");
                if bad.exceeded() {
                    break;
                }
                continue;
            }
            Err(ExpectedOpcodeError::Parse(p)) => {
                debug!("unparseable client packet: {p:?}");
                if bad.exceeded() {
                    break;
                }
                continue;
            }
            Err(ExpectedOpcodeError::Io(_)) => break,
        };
        PACKETS_IN.fetch_add(1, Ordering::Relaxed);
        if let ClientOpcodeMessage::CMSG_PING(ping) = &msg {
            if !pings.allow(Instant::now()) && !outbound.is_gm() {
                warn!("a client pings too fast; disconnecting it");
                break;
            }
            outbound.send_unheld(&SMSG_PONG {
                sequence_id: ping.sequence_id,
            });
            continue;
        }
        if tx.send(Inbound::Msg(msg)).await.is_err() {
            break;
        }
        if outbound.is_closed() {
            break;
        }
    }
    outbound.close();
}

/// Read one packet: its header decrypted, its body whole, and decoded
/// ([`decode`]); wow_messages reserves no more than a body could hold
/// however much it claims. `None` for a packet that wow_messages panicked
/// on, as a backstop.
async fn read_packet<R: AsyncRead + Unpin + Send>(
    read: &mut R,
    decrypter: &mut DecrypterHalf,
) -> Result<Option<Inbound>, ExpectedOpcodeError> {
    let mut header = [0u8; 6];
    read.read_exact(&mut header).await?;
    let header = decrypter.decrypt_client_header(header);
    let size = header.size.saturating_sub(4) as usize;
    let mut body = vec![0u8; size];
    read.read_exact(&mut body).await?;
    decode(header.opcode, body)
}

/// Decode one client packet's body: those read by hand stay raw or are
/// read here ([`teleport_ack`]), the rest go to wow_messages. `None` for a
/// packet that wow_messages panicked on.
pub fn decode(opcode: u32, body: Vec<u8>) -> Result<Option<Inbound>, ExpectedOpcodeError> {
    if RAW_OPCODES.contains(&opcode) {
        return Ok(Some(Inbound::Raw(opcode as u16, body)));
    }
    if let Some(ack) = teleport_ack(opcode, &body) {
        return Ok(Some(Inbound::Msg(
            ClientOpcodeMessage::MSG_MOVE_TELEPORT_ACK(ack),
        )));
    }
    let size = body.len();
    let mut plain = Vec::with_capacity(6 + size);
    plain.extend_from_slice(&((size + 4) as u16).to_be_bytes());
    plain.extend_from_slice(&opcode.to_le_bytes());
    plain.extend_from_slice(&body);
    match crate::game::supervisor::catch(|| ClientOpcodeMessage::read_unencrypted(&plain[..])) {
        Ok(m) => m.map(|m| Some(Inbound::Msg(m))),
        Err(e) => {
            debug!("wow_messages panicked reading client packet 0x{opcode:X}: {e}; dropped");
            Ok(None)
        }
    }
}

/// `MSG_MOVE_TELEPORT_ACK`'s opcode.
pub const MSG_MOVE_TELEPORT_ACK: u32 = 0xC7;

/// The 1.12.1 client's `MSG_MOVE_TELEPORT_ACK`, read by hand: its mover's
/// whole GUID (8 bytes), then the order's counter and the client's time,
/// 16 bytes, as mangos-classic's `HandleMoveTeleportAckOpcode` reads it
/// (`recv_data >> guid`, an `ObjectGuid`). wow_messages reads a packed
/// GUID there, so it would read a real client's ack as another GUID and
/// counter, and the teleport would never be settled.
///
/// A body of any other size goes on to wow_messages, which reads the
/// packed form older load testers send (9 to 17 bytes). A packed body is
/// 16 bytes only for a GUID with seven of its eight bytes set, which a
/// player's (a 32-bit counter, its high half zero) never has, so 16 bytes
/// is always the whole GUID.
fn teleport_ack(opcode: u32, body: &[u8]) -> Option<MSG_MOVE_TELEPORT_ACK_Client> {
    if opcode != MSG_MOVE_TELEPORT_ACK {
        return None;
    }
    let body: &[u8; 16] = body.try_into().ok()?;
    let word = |at: usize| u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]);
    let guid = u64::from_le_bytes(body[..8].try_into().ok()?);
    Some(MSG_MOVE_TELEPORT_ACK_Client {
        guid: Guid::new(guid),
        movement_counter: word(8),
        time: Duration::from_millis(word(12).into()),
    })
}

/// [`teleport_ack`]'s layout: what a 1.12.1 client sends.
pub fn teleport_ack_body(ack: &MSG_MOVE_TELEPORT_ACK_Client) -> [u8; 16] {
    let mut body = [0u8; 16];
    body[..8].copy_from_slice(&ack.guid.guid().to_le_bytes());
    body[8..12].copy_from_slice(&ack.movement_counter.to_le_bytes());
    body[12..].copy_from_slice(&(ack.time.as_millis() as u32).to_le_bytes());
    body
}

/// The largest auth session the world handshake reads.
pub const MAX_AUTH_SESSION: usize = 8 * 1024;

/// Drain queued frames, encrypt each header in order (ARC4 is a stream
/// cipher) and write each burst at once. The writer ends at the close,
/// whether it waits for frames or is in a write, and when a write makes no
/// progress for `stall`: a client that doesn't read (its window shut, its
/// acknowledgements still coming) holds its socket and its frames no
/// longer than that, nor past the close.
pub async fn run_writer<W>(
    mut write: W,
    mut encrypter: EncrypterHalf,
    mut frames: Frames,
    stall: Duration,
) where
    W: AsyncWrite + Unpin,
{
    let mut scratch: Vec<u8> = Vec::with_capacity(16 * 1024);
    let shared = frames.shared.clone();
    loop {
        // What there is, else a wait for it: none once every sender has
        // gone. Closed, what was taken goes unwritten.
        if frames.closed() {
            break;
        }
        frames.fill();
        if frames.batch.is_empty() {
            if frames.ended || frames.wait().await.is_none() {
                break;
            }
            continue;
        }
        scratch.clear();
        let mut written = 0u64;
        let mut drained = 0usize;
        {
            let z = crate::zone!("writer: burst");
            for (buf, queued) in frames.batch.drain(..) {
                if queued {
                    drained += buf.len();
                }
                written += put(&mut scratch, &mut encrypter, &buf);
            }
            shared.queued.fetch_sub(drained, Ordering::Relaxed);
            z.value(written);
        }
        if scratch.is_empty() {
            continue;
        }
        let wrote = tokio::select! {
            r = write_within(&mut write, &scratch, stall) => r,
            () = shared.wait_closed() => Err(std::io::ErrorKind::ConnectionAborted.into()),
        };
        if let Err(e) = wrote {
            debug!("a client's write ended: {e}");
            break;
        }
        PACKETS_OUT.fetch_add(written, Ordering::Relaxed);
        crate::game::stats::BYTES_OUT.fetch_add(scratch.len() as u64, Ordering::Relaxed);
        // A burst of creates doesn't keep its megabytes for good.
        if scratch.capacity() > SCRATCH_KEPT {
            scratch = Vec::with_capacity(16 * 1024);
        }
    }
    // The senders, the reader and the session see the queue closed, and
    // what it held is let go.
    shared.close();
    drop(scratch);
    drop(frames);
    // A TCP half's shutdown doesn't wait on the client; bounded all the
    // same, as the write was.
    let _ = tokio::time::timeout(stall, write.shutdown()).await;
}

/// A frame onto the burst: 1 if it went. Its first four bytes are the
/// header as the client reads it, size and opcode: copied whole, and the
/// header encrypted where it lands, in the order the frames are written
/// (ARC4 is a stream cipher).
fn put(scratch: &mut Vec<u8>, encrypter: &mut EncrypterHalf, buf: &[u8]) -> u64 {
    if buf.len() < 4 {
        return 0;
    }
    let size = u16::from_be_bytes([buf[0], buf[1]]);
    // A size that isn't the frame's would put the client's stream out of
    // step: what is built refuses one, and nothing gets past here.
    if usize::from(size) != buf.len() - 2 {
        let opcode = u16::from_le_bytes([buf[2], buf[3]]);
        warn!(
            "not writing opcode {opcode:#x}: its size field says {size}, its frame {}",
            buf.len()
        );
        return 0;
    }
    let at = scratch.len();
    scratch.extend_from_slice(buf);
    encrypter.encrypt(&mut scratch[at..at + 4]);
    1
}

/// The most of its buffer a writer keeps between bursts.
const SCRATCH_KEPT: usize = 1 << 20;

/// Write all of `buf`, or fail once a write has taken nothing for `stall`.
async fn write_within<W>(write: &mut W, mut buf: &[u8], stall: Duration) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    while !buf.is_empty() {
        match tokio::time::timeout(stall, write.write(buf)).await {
            Ok(Ok(0)) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(Ok(n)) => buf = &buf[n..],
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                crate::game::stats::STALL.fetch_add(1, Ordering::Relaxed);
                warn!("a client took nothing for {stall:?}; disconnecting it");
                return Err(std::io::ErrorKind::TimedOut.into());
            }
        }
    }
    Ok(())
}

/// mangos's rule for pings (`WorldSocket::HandlePing`): the reader answers
/// them before the session's flood guard, so they are counted here. One
/// less than [`PingRate::SOONEST`] after the last is too soon; more than
/// `max` too soon in a row disconnects (0 never does); one in time starts
/// the count again.
struct PingRate {
    max: u32,
    last: Option<Instant>,
    too_soon: u32,
}

impl PingRate {
    /// The client pings every 30 s.
    const SOONEST: Duration = Duration::from_secs(27);

    fn new(max: u32) -> Self {
        Self {
            max,
            last: None,
            too_soon: 0,
        }
    }

    /// Whether a ping at `now` is let through.
    fn allow(&mut self, now: Instant) -> bool {
        let Some(last) = self.last.replace(now) else {
            return true;
        };
        if now.saturating_duration_since(last) >= Self::SOONEST {
            self.too_soon = 0;
            return true;
        }
        self.too_soon = self.too_soon.saturating_add(1);
        self.max == 0 || self.too_soon <= self.max
    }
}

/// How many unknown or unparseable packets a client may send in a window
/// before it is disconnected.
const BAD_PACKETS_PER_WINDOW: u32 = 32;
const BAD_PACKET_WINDOW: Duration = Duration::from_secs(10);

struct BadPacketLimit {
    window_start: Instant,
    count: u32,
}

impl BadPacketLimit {
    fn new() -> Self {
        Self {
            window_start: Instant::now(),
            count: 0,
        }
    }

    fn exceeded(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.window_start) > BAD_PACKET_WINDOW {
            self.window_start = now;
            self.count = 0;
        }
        self.count += 1;
        if self.count > BAD_PACKETS_PER_WINDOW {
            warn!(
                "client sent {} bad packets in {BAD_PACKET_WINDOW:?}; disconnecting",
                self.count
            );
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, duplex};
    use wow_srp::normalized_string::NormalizedString;
    use wow_srp::vanilla_header::ProofSeed;

    pub(crate) fn paired_crypto() -> (HeaderCrypto, HeaderCrypto) {
        let session_key = [0u8; 40];
        let username = NormalizedString::new("test".to_string()).unwrap();
        let server_seed = ProofSeed::new();
        let server_seed_value = server_seed.seed();
        let client_seed = ProofSeed::new();
        let client_seed_value = client_seed.seed();
        let (client_proof, client) =
            client_seed.into_client_header_crypto(&username, session_key, server_seed_value);
        let server = server_seed
            .into_server_header_crypto(&username, session_key, client_proof, client_seed_value)
            .unwrap();
        (server, client)
    }

    fn frame(opcode: u16, body: &[u8]) -> Arc<[u8]> {
        crate::game::update::frame(opcode, body)
    }

    /// A stall bound no test reaches.
    const LONG: Duration = Duration::from_secs(600);

    /// Review 5's net 1: a writer in a write that a client which doesn't
    /// read holds up ends at the close, with every sender gone, while the
    /// client is still there: before, it stayed in the write, holding the
    /// socket and its frames, until the client went.
    #[tokio::test]
    async fn a_writer_on_a_client_that_doesnt_read_ends_at_its_close() {
        let (server, _) = paired_crypto();
        let (encrypter, _) = server.split();
        let (peer, b) = duplex(64);
        let (out, frames) = queue(1 << 20);
        let writer = tokio::spawn(run_writer(b, encrypter, frames, LONG));
        assert!(out.send_frame(frame(0x1F6, &[0; 16 * 1024])));
        // In the write: the frame taken, the pipe full.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while out.queued_bytes() > 0 {
            assert!(tokio::time::Instant::now() < deadline, "the writer takes");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!writer.is_finished(), "held up in the write");
        out.close();
        drop(out);
        tokio::time::timeout(Duration::from_secs(5), writer)
            .await
            .expect("the writer ends at the close, the client still there")
            .unwrap();
        drop(peer);
    }

    /// A write that takes nothing for the stall bound ends the writer, and
    /// the queue is closed for its senders, with no close asked for.
    #[tokio::test]
    async fn a_write_that_takes_nothing_ends_the_writer() {
        let (server, _) = paired_crypto();
        let (encrypter, _) = server.split();
        let (peer, b) = duplex(64);
        let (out, frames) = queue(1 << 20);
        let stall = Duration::from_millis(200);
        let writer = tokio::spawn(run_writer(b, encrypter, frames, stall));
        assert!(out.send_frame(frame(0x1F6, &[0; 16 * 1024])));
        tokio::time::timeout(Duration::from_secs(5), writer)
            .await
            .expect("the writer gives up")
            .unwrap();
        assert!(out.is_closed());
        tokio::time::timeout(Duration::from_secs(1), out.closed())
            .await
            .expect("and whoever waits on the close is woken");
        drop(peer);
    }

    /// A close wakes whoever waits on it, from before or after it.
    #[tokio::test]
    async fn a_close_wakes_its_waiters() {
        let (out, _frames) = queue(1 << 20);
        let waiting = {
            let out = out.clone();
            tokio::spawn(async move { out.closed().await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiting.is_finished());
        out.close();
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("woken")
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), out.closed())
            .await
            .expect("closed already");
    }

    #[tokio::test]
    async fn the_writer_keeps_the_cipher_in_step_across_bursts() {
        let (server, client) = paired_crypto();
        let (encrypter, _) = server.split();
        let (_, mut decrypter) = client.split();
        let (mut a, b) = duplex(64 * 1024);
        let (out, frames) = queue(1 << 20);
        let writer = tokio::spawn(run_writer(b, encrypter, frames, LONG));
        for burst in [1u16, 3, 2] {
            for i in 0..burst {
                assert!(out.send_frame(frame(100 + i, &[i as u8; 4])));
            }
            for i in 0..burst {
                let mut header = [0u8; 4];
                a.read_exact(&mut header).await.unwrap();
                let h = decrypter.decrypt_server_header(header);
                assert_eq!(h.opcode, 100 + i);
                assert_eq!(h.size, 6);
                let mut body = [0u8; 4];
                a.read_exact(&mut body).await.unwrap();
                assert_eq!(body, [i as u8; 4]);
            }
        }
        assert_eq!(out.queued_bytes(), 0, "the writer gave the budget back");
        drop(out);
        writer.await.unwrap();
    }

    /// Review 5's world finding 1: a body over 64 KB went out with its size
    /// field cut short, and the client read the rest of it as the packets
    /// after it. A frame too big isn't built, and the writer doesn't write
    /// one whose size field isn't its length.
    #[tokio::test]
    async fn a_frame_too_big_for_its_size_field_is_never_sent() {
        use crate::game::update::MAX_FRAME_BODY;
        assert!(frame(9, &vec![0; MAX_FRAME_BODY + 1]).is_empty());
        let fits = frame(9, &vec![0; MAX_FRAME_BODY]);
        assert_eq!(
            usize::from(u16::from_be_bytes([fits[0], fits[1]])),
            fits.len() - 2
        );
        let note = wow_world_messages::vanilla::SMSG_NOTIFICATION {
            notification: "a".repeat(MAX_FRAME_BODY),
        };
        assert!(encode(&note).is_none(), "wow_messages would cut its size");

        let (server, client) = paired_crypto();
        let (encrypter, _) = server.split();
        let (_, mut decrypter) = client.split();
        let (mut a, b) = duplex(256 * 1024);
        let (out, frames) = queue(1 << 20);
        let writer = tokio::spawn(run_writer(b, encrypter, frames, LONG));
        // 70,000 bytes under a size of 0xFFFF, as `frame` built it before.
        let mut lying = vec![0xFF, 0xFF, 0xDD, 0x00];
        lying.extend(std::iter::repeat_n(0xAB, 70_000));
        assert!(out.send_frame(Arc::from(lying)));
        assert!(out.send_frame(frame(100, &[7; 4])));
        let mut header = [0u8; 4];
        a.read_exact(&mut header).await.unwrap();
        let h = decrypter.decrypt_server_header(header);
        assert_eq!((h.opcode, h.size), (100, 6), "the next frame, in step");
        let mut body = [0u8; 4];
        a.read_exact(&mut body).await.unwrap();
        assert_eq!(body, [7; 4]);
        drop(out);
        writer.await.unwrap();
    }

    /// Closed, a queue takes nothing more and hands out nothing more, and
    /// its writer ends and closes the stream.
    #[tokio::test]
    async fn a_closed_queue_ends_its_writer_and_the_stream() {
        let (server, _) = paired_crypto();
        let (encrypter, _) = server.split();
        let (mut a, b) = duplex(64 * 1024);
        let (out, frames) = queue(1 << 20);
        let writer = tokio::spawn(run_writer(b, encrypter, frames, LONG));
        assert!(out.send_frame(frame(1, &[0; 4])));
        let mut first = [0u8; 8];
        a.read_exact(&mut first).await.unwrap();
        out.close();
        assert!(out.is_closed());
        assert!(!out.send_frame(frame(2, &[0; 4])));
        tokio::time::timeout(Duration::from_secs(10), writer)
            .await
            .expect("the writer ends")
            .unwrap();
        let mut rest = Vec::new();
        a.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty(), "nothing after the close: {rest:?}");

        let (out, mut frames) = Outbound::detached();
        assert!(out.send_frame(frame(1, &[0; 4])));
        out.close();
        assert!(frames.try_recv().is_none(), "what was queued is dropped");
        assert!(!out.send_frame(frame(2, &[0; 4])));
        assert!(frames.recv().await.is_none());
    }

    /// A writer that has gone (its client's socket closed) closes the
    /// queue for its senders.
    #[tokio::test]
    async fn a_queue_whose_writer_has_gone_is_closed() {
        let (server, _) = paired_crypto();
        let (encrypter, _) = server.split();
        let (a, b) = duplex(64);
        drop(a);
        let (out, frames) = queue(1 << 20);
        let writer = tokio::spawn(run_writer(b, encrypter, frames, LONG));
        assert!(out.send_frame(frame(1, &[0; 4])));
        tokio::time::timeout(Duration::from_secs(10), writer)
            .await
            .expect("the writer ends")
            .unwrap();
        assert!(out.is_closed());
        assert!(!out.send_frame(frame(2, &[0; 4])));
    }

    #[tokio::test]
    async fn the_reader_hands_on_packets_wow_messages_reads_and_raw_ones() {
        let (server, client) = paired_crypto();
        let (_, decrypter) = server.split();
        let (mut encrypter, _) = client.split();
        let (mut a, b) = duplex(64 * 1024);
        let (tx, mut rx) = mpsc::channel(8);
        let (outbound, _frames) = queue(1 << 20);
        tokio::spawn(run_reader(b, decrypter, tx, outbound));
        // CMSG_SET_WATCHED_FACTION, read by hand: index 3.
        let body = 3u32.to_le_bytes();
        a.write_all(&encrypter.encrypt_client_header(4 + 4, 0x318))
            .await
            .unwrap();
        a.write_all(&body).await.unwrap();
        // CMSG_LOGOUT_REQUEST, which wow_messages reads.
        a.write_all(&encrypter.encrypt_client_header(4, 0x4B))
            .await
            .unwrap();
        match rx.recv().await {
            Some(Inbound::Raw(0x318, b)) => assert_eq!(b, body),
            other => panic!("{other:?}"),
        }
        match rx.recv().await {
            Some(Inbound::Msg(ClientOpcodeMessage::CMSG_LOGOUT_REQUEST)) => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_client_over_budget_is_disconnected() {
        let (out, _frames) = queue(10);
        assert!(out.send_frame(frame(1, &[0; 4])));
        assert!(!out.send_frame(frame(1, &[0; 4])));
        assert!(out.is_closed());
    }

    #[test]
    fn what_is_sent_while_held_follows_what_the_hold_was_for_in_order() {
        let (out, mut rx) = Outbound::detached();
        assert!(out.send_frame(frame(1, &[0; 4])));
        assert!(out.hold());
        assert!(!out.hold(), "held once");
        assert!(!out.has_held());
        assert!(out.send_frame(frame(3, &[0; 4])));
        assert!(out.send_frame(frame(4, &[0; 4])));
        assert!(out.has_held());
        assert_eq!(rx.len(), 1, "the frames wait");
        out.release(vec![frame(2, &[0; 4])]);
        assert!(out.send_frame(frame(5, &[0; 4])));
        let opcodes: Vec<u16> = std::iter::from_fn(|| rx.try_recv())
            .map(|f| u16::from_le_bytes([f[2], f[3]]))
            .collect();
        assert_eq!(opcodes, [1, 2, 3, 4, 5]);
    }

    fn opcodes(rx: &mut Frames) -> Vec<u16> {
        std::iter::from_fn(|| rx.try_recv())
            .map(|f| u16::from_le_bytes([f[2], f[3]]))
            .collect()
    }

    /// Corked, what is sent waits for the cork's end and goes in one push,
    /// in order; the budget is taken at the send, as uncorked.
    #[test]
    fn a_cork_pushes_each_client_its_frames_at_its_end_in_order() {
        let (a, mut ra) = Outbound::detached();
        let (b, mut rb) = Outbound::detached();
        {
            let _cork = Cork::new();
            for i in 1..=3 {
                assert!(a.send_frame(frame(i, &[0; 4])));
                assert!(b.send_frame(frame(10 + i, &[0; 4])));
            }
            assert_eq!(a.queued_bytes(), 3 * 8, "the budget taken at the send");
            assert!(ra.rx.is_empty() && rb.rx.is_empty(), "nothing pushed yet");
            {
                let _inner = Cork::new();
                assert!(a.send_frame(frame(4, &[0; 4])));
            }
            assert!(ra.rx.is_empty(), "an inner cork's end pushes nothing");
        }
        assert_eq!(ra.rx.len(), 1, "one push");
        assert_eq!(opcodes(&mut ra), [1, 2, 3, 4]);
        assert_eq!(opcodes(&mut rb), [11, 12, 13]);
        // Uncorked again: each frame goes at once.
        assert!(a.send_frame(frame(5, &[0; 4])));
        assert_eq!(ra.rx.len(), 1);
    }

    /// `uncork` pushes what waits, the cork staying on.
    #[test]
    fn uncork_pushes_what_waits_and_the_cork_stays() {
        let (a, mut ra) = Outbound::detached();
        let _cork = Cork::new();
        a.send_frame(frame(1, &[0; 4]));
        uncork();
        assert_eq!(opcodes(&mut ra), [1]);
        a.send_frame(frame(2, &[0; 4]));
        assert!(ra.rx.is_empty(), "corked still");
        drop(_cork);
        assert_eq!(opcodes(&mut ra), [2]);
    }

    /// A hold under a cork keeps the order it keeps uncorked: what went
    /// before it, then the hold's blocks, then what waited, then the rest.
    #[test]
    fn a_hold_under_a_cork_keeps_its_order() {
        let (out, mut rx) = Outbound::detached();
        let cork = Cork::new();
        assert!(out.send_frame(frame(1, &[0; 4])));
        assert!(out.hold());
        assert!(out.send_frame(frame(3, &[0; 4])));
        assert!(out.send_unheld(&SMSG_PONG { sequence_id: 0 }));
        assert!(out.send_frame(frame(4, &[0; 4])));
        assert!(out.has_held());
        out.release(vec![frame(2, &[0; 4])]);
        assert!(out.send_frame(frame(5, &[0; 4])));
        drop(cork);
        assert_eq!(opcodes(&mut rx), [1, 0x1DD, 2, 3, 4, 5]);
    }

    /// Over budget corked, a client is disconnected at the send; a closed
    /// client's corked frames are dropped.
    #[test]
    fn a_corked_client_over_budget_or_closed_gets_nothing() {
        let (out, mut rx) = queue(10);
        let cork = Cork::new();
        assert!(out.send_frame(frame(1, &[0; 4])));
        assert!(!out.send_frame(frame(1, &[0; 4])));
        assert!(out.is_closed());
        drop(cork);
        assert!(rx.try_recv().is_none());
        assert!(rx.rx.is_empty(), "nothing pushed to a closed client");
    }

    /// Another thread's sends aren't corked by this one's cork.
    #[test]
    fn a_cork_is_the_threads_own() {
        let (out, mut rx) = Outbound::detached();
        let _cork = Cork::new();
        let other = out.clone();
        std::thread::spawn(move || assert!(other.send_frame(frame(7, &[0; 4]))))
            .join()
            .unwrap();
        assert_eq!(opcodes(&mut rx), [7]);
    }

    fn clients(of: &[usize]) -> Arc<Clients> {
        let mut c = Clients::default();
        for i in of {
            c.insert(*i);
        }
        Arc::new(c)
    }

    /// Waits a little for `f`, polling: what the feed's waker does, it
    /// does on a thread of its own.
    fn soon(mut f: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        f()
    }

    /// A broadcast is kept once; each client gets it in its place among
    /// what was sent it alone, and the waker takes its budget for it once
    /// it is published.
    #[test]
    fn a_broadcast_reaches_each_client_in_its_place() {
        let feed = Feed::new(7);
        let queues: Vec<(Outbound, Frames)> = (0..3).map(|_| Outbound::detached()).collect();
        let table = |i: usize| queues.get(i).map(|(o, _)| o.clone());
        {
            let _cork = Cork::new();
            queues[0].0.send_frame(frame(1, &[0; 4]));
            broadcast(&feed, &frame(2, &[0; 4]), &clients(&[0, 2]), table);
            queues[0].0.send_frame(frame(3, &[0; 4]));
            queues[1].0.send_frame(frame(3, &[0; 4]));
            broadcast(&feed, &frame(4, &[0; 4]), &clients(&[0, 1, 2, 9]), table);
            assert_eq!(feed.published(), 0, "published at the push");
        }
        let mut q = queues;
        assert_eq!(feed.published(), 1, "one chunk");
        assert!(soon(|| q[2].0.queued_bytes() == 16), "its share charged");
        assert_eq!(opcodes(&mut q[0].1), [1, 2, 3, 4]);
        assert_eq!(opcodes(&mut q[1].1), [3, 4]);
        assert_eq!(opcodes(&mut q[2].1), [2, 4]);
        assert_eq!(q[2].0.queued_bytes(), 0, "taken");
        // Uncorked, a chunk of its own at once.
        let table = |i: usize| q.get(i).map(|(o, _)| o.clone());
        broadcast(&feed, &frame(5, &[0; 4]), &clients(&[1]), table);
        assert_eq!(feed.published(), 2);
        assert_eq!(opcodes(&mut q[1].1), [5]);
    }

    /// A broadcast to a held client goes into its hold, before the hold or
    /// after it into the feed, as uncorked.
    #[test]
    fn a_broadcast_keeps_a_holds_order() {
        let feed = Feed::new(1);
        let (a, mut ra) = Outbound::detached();
        let table = |i: usize| (i == 0).then(|| a.clone());
        let cork = Cork::new();
        broadcast(&feed, &frame(1, &[0; 4]), &clients(&[0]), table);
        assert!(a.hold());
        broadcast(&feed, &frame(3, &[0; 4]), &clients(&[0]), table);
        a.send_frame(frame(4, &[0; 4]));
        assert!(a.has_held());
        a.release(vec![frame(2, &[0; 4])]);
        broadcast(&feed, &frame(5, &[0; 4]), &clients(&[0]), table);
        drop(cork);
        assert_eq!(opcodes(&mut ra), [1, 2, 3, 4, 5]);
        // Held before the first broadcast reaches it.
        let feed = Feed::new(2);
        let (b, mut rb) = Outbound::detached();
        let table = |i: usize| (i == 0).then(|| b.clone());
        assert!(b.hold());
        {
            let _cork = Cork::new();
            broadcast(&feed, &frame(7, &[0; 4]), &clients(&[0]), table);
            assert!(b.has_held());
        }
        assert!(rb.try_recv().is_none(), "held");
        b.release(vec![frame(6, &[0; 4])]);
        assert_eq!(opcodes(&mut rb), [6, 7]);
    }

    /// A client that falls behind its feed by more than its budget is
    /// disconnected by the waker; another map's broadcast pushes the first
    /// map's before it.
    #[test]
    fn a_broadcast_over_budget_disconnects_and_maps_dont_mix() {
        let one = Feed::new(1);
        let two = Feed::new(2);
        let (small, mut rs) = queue(10);
        let (big, mut rb) = Outbound::detached();
        {
            let _cork = Cork::new();
            let first = |i: usize| (i == 0).then(|| small.clone());
            broadcast(&one, &frame(1, &[0; 4]), &clients(&[0]), first);
            broadcast(&one, &frame(2, &[0; 4]), &clients(&[0]), first);
            let other = |i: usize| (i == 0).then(|| big.clone());
            broadcast(&two, &frame(3, &[0; 4]), &clients(&[0]), other);
            assert_eq!(one.published(), 1, "pushed before the other's");
        }
        assert!(soon(|| small.is_closed()), "over, by the waker");
        assert!(rs.try_recv().is_none());
        assert_eq!(opcodes(&mut rb), [3]);
    }

    /// Parked, what a cork held waits for the next cork, and goes before
    /// what that one is sent; dropped, it is pushed.
    #[test]
    fn a_parked_cork_goes_on_with_the_next_or_is_pushed_when_dropped() {
        let feed = Feed::new(1);
        let (a, mut ra) = Outbound::detached();
        let table = |i: usize| (i == 0).then(|| a.clone());
        let cork = Cork::new();
        broadcast(&feed, &frame(1, &[0; 4]), &clients(&[0]), table);
        a.send_frame(frame(2, &[0; 4]));
        let parked = cork.park().expect("it holds something");
        assert!(ra.try_recv().is_none(), "parked, not pushed");
        // Elsewhere meanwhile, uncorked: on its way at once.
        assert!(!CORKED.get());
        let parked = std::thread::spawn(move || {
            let cork = Cork::resume(parked);
            cork.park().expect("still")
        })
        .join()
        .unwrap();
        let cork = Cork::resume(parked);
        broadcast(&feed, &frame(3, &[0; 4]), &clients(&[0]), table);
        let parked = cork.park().unwrap();
        assert!(ra.try_recv().is_none());
        drop(parked);
        assert_eq!(opcodes(&mut ra), [1, 2, 3]);
        assert!(Cork::new().park().is_none(), "an empty cork parks nothing");
    }

    /// What anyone sends a client of a feed uncorked comes after the
    /// chunks published before it, and before the rest; corked, in its
    /// place among the cork's broadcasts.
    #[test]
    fn a_feeds_client_gets_what_it_is_sent_in_the_order_sent() {
        let feed = Feed::new(1);
        let (a, mut ra) = Outbound::detached();
        let table = |i: usize| (i == 0).then(|| a.clone());
        broadcast(&feed, &frame(1, &[0; 4]), &clients(&[0]), table);
        a.send_frame(frame(2, &[0; 4]));
        {
            let cork = Cork::new();
            cork.feed(&feed);
            a.send_frame(frame(3, &[0; 4]));
            broadcast(&feed, &frame(4, &[0; 4]), &clients(&[0]), table);
            a.send_frame(frame(5, &[0; 4]));
            // From another thread meanwhile: after what was published.
            let other = a.clone();
            std::thread::spawn(move || other.send_frame(frame(9, &[0; 4])))
                .join()
                .unwrap();
        }
        broadcast(&feed, &frame(6, &[0; 4]), &clients(&[0]), table);
        a.send_frame(frame(7, &[0; 4]));
        assert_eq!(opcodes(&mut ra), [1, 2, 9, 3, 4, 5, 6, 7]);
    }

    /// An index let go is read up to there by its client, and its next
    /// holder reads from where it was given it: neither gets the other's.
    #[test]
    fn an_index_let_go_and_given_again_keeps_each_clients_own() {
        let feed = Feed::new(1);
        let (a, mut ra) = Outbound::detached();
        let (b, mut rb) = Outbound::detached();
        let to_a = |i: usize| (i == 0).then(|| a.clone());
        {
            let _cork = Cork::new();
            broadcast(&feed, &frame(1, &[0; 4]), &clients(&[0]), to_a);
        }
        // A leaves: what was published for it so far, then nothing.
        feed.unsubscribe(0);
        feed.found(0);
        let to_b = |i: usize| (i == 0).then(|| b.clone());
        broadcast(&feed, &frame(2, &[0; 4]), &clients(&[0]), to_b);
        a.send_frame(frame(3, &[0; 4]));
        assert_eq!(opcodes(&mut ra), [1, 3]);
        assert_eq!(opcodes(&mut rb), [2]);
        // To another map's feed: the first's up to its leaving, then the
        // second's.
        let other = Feed::new(2);
        let to_b2 = |i: usize| (i == 5).then(|| b.clone());
        broadcast(&feed, &frame(4, &[0; 4]), &clients(&[0]), to_b);
        broadcast(&other, &frame(5, &[0; 4]), &clients(&[5]), to_b2);
        broadcast(&feed, &frame(6, &[0; 4]), &clients(&[0]), to_b);
        assert_eq!(opcodes(&mut rb), [4, 5]);
    }

    /// A writer reads its feed: the broadcasts reach the socket, woken by
    /// the waker, in order with what was sent alone.
    #[tokio::test]
    async fn a_writer_reads_its_feed() {
        let (server, client) = paired_crypto();
        let (encrypter, _) = server.split();
        let (_, mut decrypter) = client.split();
        let (mut peer, b) = duplex(1 << 16);
        let (out, frames) = queue(1 << 20);
        let writer = tokio::spawn(run_writer(b, encrypter, frames, LONG));
        let feed = Feed::new(1);
        let table = |i: usize| (i == 3).then(|| out.clone());
        out.send_frame(frame(1, &[0; 4]));
        {
            let cork = Cork::new();
            cork.feed(&feed);
            broadcast(&feed, &frame(2, &[0; 4]), &clients(&[3]), table);
            out.send_frame(frame(3, &[0; 4]));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        broadcast(&feed, &frame(4, &[0; 4]), &clients(&[3]), table);
        let mut got = Vec::new();
        for _ in 0..4 {
            let mut header = [0u8; 4];
            tokio::time::timeout(Duration::from_secs(5), peer.read_exact(&mut header))
                .await
                .expect("written")
                .unwrap();
            let h = decrypter.decrypt_server_header(header);
            let mut body = vec![0u8; h.size as usize - 2];
            peer.read_exact(&mut body).await.unwrap();
            got.push(h.opcode);
        }
        assert_eq!(got, [1, 2, 3, 4]);
        out.close();
        drop(out);
        let _ = tokio::time::timeout(Duration::from_secs(5), writer).await;
    }

    #[test]
    fn a_held_client_over_budget_is_disconnected() {
        let (out, _frames) = queue(10);
        out.hold();
        assert!(out.send_frame(frame(1, &[0; 4])));
        assert!(!out.send_frame(frame(1, &[0; 4])));
        assert!(out.is_closed());
    }

    /// mangos's ping rule: a ping every 30 s goes on for ever; more than
    /// `max` in a row each less than 27 s after the last disconnects, and
    /// one in time starts the count again; 0 never disconnects.
    #[test]
    fn pings_too_soon_in_a_row_disconnect_and_one_in_time_forgives() {
        let start = Instant::now();
        let at = |s: u64| start + Duration::from_secs(s);
        let mut steady = PingRate::new(2);
        assert!((0..100).all(|i| steady.allow(at(i * 30))));
        let mut fast = PingRate::new(2);
        assert!(fast.allow(at(0)));
        assert!(fast.allow(at(1)) && fast.allow(at(2)), "two too soon");
        assert!(fast.allow(at(40)), "one in time");
        assert!(fast.allow(at(41)) && fast.allow(at(42)));
        assert!(!fast.allow(at(43)), "the third too soon in a row");
        let mut off = PingRate::new(0);
        assert!((0..100).all(|_| off.allow(start)));
    }

    /// Review 5's net 5: a flood of pings ends the connection, where each
    /// was answered and none counted.
    #[tokio::test]
    async fn a_flood_of_pings_disconnects() {
        let (server, client) = paired_crypto();
        let (_, decrypter) = server.split();
        let (mut encrypter, _) = client.split();
        let (mut a, b) = duplex(1 << 20);
        let (tx, _rx) = mpsc::channel(8);
        let (outbound, mut frames) = queue(1 << 20);
        let reader = tokio::spawn(run_reader(b, decrypter, tx, outbound.clone()));
        for i in 0..1000u32 {
            let mut body = i.to_le_bytes().to_vec();
            body.extend_from_slice(&0u32.to_le_bytes());
            let mut packet = encrypter.encrypt_client_header(4 + 8, 0x1DC).to_vec();
            packet.extend_from_slice(&body);
            if a.write_all(&packet).await.is_err() {
                break;
            }
        }
        tokio::time::timeout(Duration::from_secs(10), reader)
            .await
            .expect("the reader ends")
            .unwrap();
        assert!(outbound.is_closed());
        let max = crate::config::config().network.max_overspeed_pings as usize;
        let mut pongs = 0;
        // Closed, the queue hands out nothing: what it held, counted.
        while let Ok(item) = frames.rx.try_recv() {
            pongs += match item {
                Item::One(..) => 1,
                Item::Corked(c, _) => c.len(),
                Item::Sub { .. } | Item::Unsub(_) => 0,
            };
        }
        assert_eq!(pongs, 1 + max, "the first, and those let through");
    }

    /// A GM's pings aren't counted against them, as mangos kicks only
    /// `SEC_PLAYER` for them: GM tools and test clients ping faster.
    #[tokio::test]
    async fn a_gms_flood_of_pings_is_answered() {
        let (server, client) = paired_crypto();
        let (_, decrypter) = server.split();
        let (mut encrypter, _) = client.split();
        let (mut a, b) = duplex(1 << 20);
        let (tx, _rx) = mpsc::channel(8);
        let (outbound, mut frames) = queue(1 << 20);
        outbound.set_gm(true);
        let reader = tokio::spawn(run_reader(b, decrypter, tx, outbound.clone()));
        for i in 0..100u32 {
            let mut body = i.to_le_bytes().to_vec();
            body.extend_from_slice(&0u32.to_le_bytes());
            let mut packet = encrypter.encrypt_client_header(4 + 8, 0x1DC).to_vec();
            packet.extend_from_slice(&body);
            a.write_all(&packet).await.unwrap();
        }
        let mut pongs = 0;
        while pongs < 100 {
            let item = tokio::time::timeout(Duration::from_secs(5), frames.rx.recv())
                .await
                .expect("a pong for each")
                .expect("the queue open");
            pongs += match item {
                Item::One(..) => 1,
                Item::Corked(c, _) => c.len(),
                Item::Sub { .. } | Item::Unsub(_) => 0,
            };
        }
        assert!(!outbound.is_closed());
        reader.abort();
    }

    #[test]
    fn a_burst_of_bad_packets_disconnects_and_a_few_do_not() {
        let mut limit = BadPacketLimit::new();
        for _ in 0..BAD_PACKETS_PER_WINDOW {
            assert!(!limit.exceeded());
        }
        assert!(limit.exceeded());
    }
}
