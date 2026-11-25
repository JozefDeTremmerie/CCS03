//! Host copies of textures the title fills, placed in heaps of the card's
//! memory that hold many of them (`VICTORY_TEXTURE_HEAPS=0`: each made
//! with memory of its own, as before).
//!
//! Why: in the opening cutscene's first minutes the title streams in
//! thousands of textures a minute, and each host copy was a committed
//! resource: an allocation of the graphics kernel's
//! (`NtGdiDdDDICreateAllocation`) and a wait for it to be made resident on
//! the card (`NtGdiDdDDIWaitForSynchronizationObjectFromCpu`). The profile
//! of 25 November (RTX 4060 laptop, 8 GB) has those two at 64 % of the pass
//! over the heap's texture slots, which takes 500 to 1,500 ms of each
//! frame then, on the thread that records it. A texture placed in a heap
//! that is already there takes neither: one heap of [`HEAP_BYTES`] is
//! made and waited for once for some dozens of textures.
//!
//! A texture's place is given back when its host copy goes, once the
//! submissions running then have finished (`crate::host::retire`). A heap
//! left empty is let go at a later call, one empty heap kept. Textures over
//! [`LARGEST`], with a clear value or another alignment are made as before.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use windows::Win32::Graphics::Direct3D12::*;
use windows::core::Interface;

/// The bytes of one heap.
const HEAP_BYTES: u64 = 64 << 20;
/// The largest texture placed in a heap.
const LARGEST: u64 = 16 << 20;
/// Places are given in this grain (a texture's own alignment).
const GRAIN: u64 = 64 << 10;

/// Heaps made, textures placed, heaps let go, and the bytes of the heaps
/// alive, since the start.
pub(crate) static STATS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

struct Heap {
    heap: ID3D12Heap,
    /// Free ranges by offset: their length.
    free: BTreeMap<u64, u64>,
    used: u64,
    /// The call at which it was left empty.
    empty_at: Option<u64>,
}

// SAFETY: a D3D12 heap is free-threaded; the rest is plain data.
unsafe impl Send for Heap {}

struct Heaps {
    list: Vec<Option<Heap>>,
    /// The place of each texture in a heap, by the texture's address.
    placed: Option<HashMap<usize, (usize, u64, u64)>>,
    calls: u64,
}

static HEAPS: Mutex<Heaps> = Mutex::new(Heaps {
    list: Vec::new(),
    placed: None,
    calls: 0,
});

fn heaps() -> std::sync::MutexGuard<'static, Heaps> {
    HEAPS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether textures are placed in shared heaps.
pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_TEXTURE_HEAPS").is_none_or(|v| v != "0"))
}

impl Heaps {
    /// Lets go of heaps left empty at an earlier call (their last texture
    /// is gone by now), keeping one empty heap.
    fn purge(&mut self) {
        let mut kept = false;
        for entry in &mut self.list {
            let Some(heap) = entry else { continue };
            match heap.empty_at {
                Some(at) if heap.used == 0 && at < self.calls => {
                    if kept {
                        STATS[2].fetch_add(1, Relaxed);
                        STATS[3].fetch_sub(HEAP_BYTES, Relaxed);
                        *entry = None;
                    } else {
                        kept = true;
                    }
                }
                _ => {}
            }
        }
    }

    /// A free range of `size` bytes: the heap's index and the offset.
    fn take(&mut self, size: u64) -> Option<(usize, u64)> {
        for (index, entry) in self.list.iter_mut().enumerate() {
            let Some(heap) = entry else { continue };
            let found = heap.free.iter().find(|&(_, &len)| len >= size).map(|(&at, &len)| (at, len));
            if let Some((at, len)) = found {
                heap.free.remove(&at);
                if len > size {
                    heap.free.insert(at + size, len - size);
                }
                heap.used += size;
                heap.empty_at = None;
                return Some((index, at));
            }
        }
        None
    }

    /// Gives `[at, at + size)` of heap `index` back.
    fn give_back(&mut self, index: usize, at: u64, size: u64) {
        let calls = self.calls;
        let Some(Some(heap)) = self.list.get_mut(index) else { return };
        let given = size;
        let (mut at, mut size) = (at, size);
        if let Some((&before, &len)) = heap.free.range(..at).next_back()
            && before + len == at
        {
            heap.free.remove(&before);
            at = before;
            size += len;
        }
        if let Some(&len) = heap.free.get(&(at + size)) {
            heap.free.remove(&(at + size));
            size += len;
        }
        heap.free.insert(at, size);
        heap.used = heap.used.saturating_sub(given);
        if heap.free.len() == 1 && heap.free.get(&0) == Some(&HEAP_BYTES) {
            heap.used = 0;
            heap.empty_at = Some(calls);
        }
    }
}

/// A host texture of description `desc` placed in a shared heap, in state
/// `state`; `None` if it is not one for a heap (then it is made as before).
pub(crate) fn place(device: &ID3D12Device, desc: &D3D12_RESOURCE_DESC, state: D3D12_RESOURCE_STATES) -> Option<ID3D12Resource> {
    if !on() {
        return None;
    }
    // SAFETY: a plain query about one description.
    let info = unsafe { device.GetResourceAllocationInfo(0, std::slice::from_ref(desc)) };
    if info.SizeInBytes == u64::MAX || info.SizeInBytes > LARGEST || info.Alignment > GRAIN {
        return None;
    }
    let size = info.SizeInBytes.max(1).next_multiple_of(GRAIN);
    let (index, at, heap) = {
        let mut all = heaps();
        all.calls += 1;
        all.purge();
        let (index, at) = match all.take(size) {
            Some(found) => found,
            None => {
                let heap = make_heap(device)?;
                STATS[0].fetch_add(1, Relaxed);
                STATS[3].fetch_add(HEAP_BYTES, Relaxed);
                let made = Heap {
                    heap,
                    free: BTreeMap::from([(0, HEAP_BYTES)]),
                    used: 0,
                    empty_at: None,
                };
                match all.list.iter().position(Option::is_none) {
                    Some(empty) => all.list[empty] = Some(made),
                    None => all.list.push(Some(made)),
                }
                all.take(size)?
            }
        };
        let heap = all.list[index].as_ref()?.heap.clone();
        (index, at, heap)
    };
    let mut texture: Option<ID3D12Resource> = None;
    // SAFETY: a texture at its own free range of the heap, which nothing
    // else uses; the out pointer is ours.
    let made = unsafe { device.CreatePlacedResource(&heap, at, desc, state, None, &mut texture) };
    let mut all = heaps();
    match (made, texture) {
        (Ok(()), Some(texture)) => {
            all.placed.get_or_insert_with(HashMap::new).insert(texture.as_raw() as usize, (index, at, size));
            if STATS[1].fetch_add(1, Relaxed) == 0 {
                tracing::info!("host copies of textures the title fills are placed in shared heaps of the card's memory (VICTORY_TEXTURE_HEAPS=0: each with its own)");
            }
            Some(texture)
        }
        (made, _) => {
            static SAID: std::sync::Once = std::sync::Once::new();
            SAID.call_once(|| tracing::warn!(error = ?made.err(), "a texture could not be placed in a shared heap; made with its own memory"));
            all.give_back(index, at, size);
            None
        }
    }
}

fn make_heap(device: &ID3D12Device) -> Option<ID3D12Heap> {
    let desc = |flags: D3D12_HEAP_FLAGS| D3D12_HEAP_DESC {
        SizeInBytes: HEAP_BYTES,
        Properties: D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_DEFAULT,
            ..Default::default()
        },
        Alignment: 0,
        Flags: flags,
    };
    for flags in [
        D3D12_HEAP_FLAG_ALLOW_ONLY_NON_RT_DS_TEXTURES | D3D12_HEAP_FLAG_CREATE_NOT_ZEROED,
        D3D12_HEAP_FLAG_ALLOW_ONLY_NON_RT_DS_TEXTURES,
    ] {
        let mut made: Option<ID3D12Heap> = None;
        let d = desc(flags);
        // SAFETY: plain creation; the out pointer is ours.
        if unsafe { device.CreateHeap::<ID3D12Heap>(&raw const d, &raw mut made) }.is_ok() && made.is_some() {
            return made;
        }
    }
    static SAID: std::sync::Once = std::sync::Once::new();
    SAID.call_once(|| tracing::warn!("no heap for host textures: they are made with their own memory"));
    None
}

/// A texture's place in a heap, given back when this is dropped.
struct Place(usize, u64, u64);

impl Drop for Place {
    fn drop(&mut self) {
        heaps().give_back(self.0, self.1, self.2);
    }
}

/// The host copy `texture` goes: its place in a heap, if it has one, is
/// free again once the submissions running now have finished.
pub(crate) fn release(texture: &ID3D12Resource) {
    let place = heaps().placed.as_mut().and_then(|p| p.remove(&(texture.as_raw() as usize)));
    if let Some((index, at, size)) = place {
        crate::host::retire(Place(index, at, size));
    }
}
