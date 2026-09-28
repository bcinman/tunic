//! Private latest-value ownership handoff for prepared chains.
//!
//! Publishers may lock, allocate, and reclaim storage off the real-time thread.
//! The single reader adopts a chain with one bounded atomic slot exchange.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::dsp::PreparedChain;

const SLOT_COUNT: usize = 3;
const PUBLISHER_SLOT: usize = 0;
const SHARED_SLOT: usize = 1;
const READER_SLOT: usize = 2;
const SLOT_MASK: usize = 0b11;
const PENDING_BIT: usize = 0b100;

pub(super) fn channel() -> (ChainReader, ChainPublisher) {
    let exchange = Arc::new(ChainExchange::new());
    (
        ChainReader {
            exchange: Arc::clone(&exchange),
            slot: READER_SLOT,
        },
        ChainPublisher { exchange },
    )
}

#[derive(Clone)]
pub(super) struct ChainPublisher {
    exchange: Arc<ChainExchange>,
}

impl ChainPublisher {
    pub(super) fn publish(&self, chain: PreparedChain) {
        let mut publisher_slot = self
            .exchange
            .publisher_slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: the publisher mutex grants exclusive ownership of this slot.
        unsafe { &mut *self.exchange.slots[*publisher_slot].get() }.replace(chain);
        let previous_shared = self
            .exchange
            .shared
            .swap(shared_state(*publisher_slot, true), Ordering::AcqRel);
        *publisher_slot = slot_index(previous_shared);
        // SAFETY: the atomic swap transferred the previous shared slot to this publisher.
        let reclaimed = unsafe { &mut *self.exchange.slots[*publisher_slot].get() }.take();
        drop(publisher_slot);
        drop(reclaimed);
    }
}

pub(super) struct ChainReader {
    exchange: Arc<ChainExchange>,
    slot: usize,
}

impl ChainReader {
    pub(super) fn try_adopt(&mut self, current: &mut PreparedChain) -> bool {
        if !has_pending(self.exchange.shared.load(Ordering::Acquire)) {
            return false;
        }
        let adopted = self
            .exchange
            .shared
            .swap(shared_state(self.slot, false), Ordering::AcqRel);
        debug_assert!(has_pending(adopted));
        self.slot = slot_index(adopted);
        std::mem::swap(current, self.previous_mut());
        true
    }

    pub(super) fn previous_mut(&mut self) -> &mut PreparedChain {
        // SAFETY: this reader exclusively owns `self.slot` until its next atomic swap.
        unsafe { &mut *self.exchange.slots[self.slot].get() }
            .as_mut()
            .expect("reader slot contains the previous chain after adoption")
    }
}

struct ChainExchange {
    slots: [UnsafeCell<Option<PreparedChain>>; SLOT_COUNT],
    shared: AtomicUsize,
    publisher_slot: Mutex<usize>,
}

impl ChainExchange {
    fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| UnsafeCell::new(None)),
            shared: AtomicUsize::new(shared_state(SHARED_SLOT, false)),
            publisher_slot: Mutex::new(PUBLISHER_SLOT),
        }
    }
}

// SAFETY: each slot has exactly one owner: the mutex-serialized publisher, the
// shared atomic state, or the single reader. Ownership moves through atomic
// swaps before a new owner accesses a slot.
unsafe impl Sync for ChainExchange {}

const fn shared_state(slot: usize, pending: bool) -> usize {
    slot | if pending { PENDING_BIT } else { 0 }
}

const fn slot_index(state: usize) -> usize {
    state & SLOT_MASK
}

const fn has_pending(state: usize) -> bool {
    state & PENDING_BIT != 0
}
