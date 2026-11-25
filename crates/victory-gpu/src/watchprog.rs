//! What chosen programs write, as the title sees it
//! (`VICTORY_WATCH_PROGRAMS=<name>,<name>`).
//!
//! For a dispatch of a program whose name starts with one of the given
//! names, the buffers it writes (unordered views through tables or root
//! views) are noted once each. Every three seconds of the wall, the first
//! bytes of each, read from title memory where the title reads them, are
//! logged as floats, with how many of their first 4 KB are not zero and
//! whether they changed since the last look. A buffer whose values stay
//! zero, or never change while the scene moves, is one the title's CPU
//! never gets the card's results in: the boat's water heights
//! (`CS_ResolveHeightLookUps`, `CS_BatchReadback`) are the first use.

use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Most buffers watched in all.
const MOST: usize = 24;
/// Bytes looked at per buffer.
const LOOK: usize = 4096;

#[derive(Debug)]
struct Watched {
    program: String,
    slot: String,
    address: u64,
    size: u64,
    last: Option<u64>,
}

struct State {
    watched: Vec<Watched>,
    next: Instant,
}

fn names() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        std::env::var("VICTORY_WATCH_PROGRAMS")
            .map(|v| {
                v.split(',')
                    .map(|n| n.trim().to_owned())
                    .filter(|n| !n.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(State {
            watched: Vec::new(),
            next: Instant::now() + Duration::from_secs(3),
        })
    })
}

/// Whether programs are watched at all.
pub(crate) fn on() -> bool {
    !names().is_empty()
}

/// The watched name `program` starts with, if any.
pub(crate) fn watched(program: &str) -> bool {
    names().iter().any(|n| program.starts_with(n.as_str()))
}

/// A buffer `program` writes, at `address` for `size` bytes, through `slot`.
pub(crate) fn note(program: &str, slot: String, address: u64, size: u64) {
    if address == 0 {
        return;
    }
    let mut s = state().lock().unwrap_or_else(|e| e.into_inner());
    if s.watched.len() >= MOST
        || s.watched
            .iter()
            .any(|w| w.address == address && w.program == program)
    {
        return;
    }
    tracing::info!(
        program,
        slot,
        address = format_args!("{address:#x}"),
        size,
        "watched program writes a buffer (VICTORY_WATCH_PROGRAMS)"
    );
    s.watched.push(Watched {
        program: program.to_owned(),
        slot,
        address,
        size,
        last: None,
    });
}

/// Bytes of this process at `address`, or none where it cannot be read.
fn read(address: u64, len: usize) -> Option<Vec<u8>> {
    use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut bytes = vec![0u8; len];
    let mut got = 0usize;
    // SAFETY: the system checks the range; a page that cannot be read
    // fails the call instead of faulting.
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const _,
            bytes.as_mut_ptr().cast(),
            len,
            &mut got,
        )
    };
    (ok != 0 && got > 0).then(|| {
        bytes.truncate(got);
        bytes
    })
}

/// A present: every three seconds, what the watched buffers hold.
pub(crate) fn sample() {
    if !on() {
        return;
    }
    let mut s = state().lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    if now < s.next {
        return;
    }
    s.next = now + Duration::from_secs(3);
    for w in &mut s.watched {
        let len = LOOK.min(w.size as usize).max(4);
        let Some(bytes) = read(w.address, len) else {
            tracing::info!(program = %w.program, address = format_args!("{:#x}", w.address), "watched buffer: cannot be read");
            continue;
        };
        let words: Vec<u32> = bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let nonzero = words.iter().filter(|&&w| w != 0).count();
        let hash = words.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| {
            (h ^ u64::from(x)).wrapping_mul(0x100_0000_01b3)
        });
        let changed = w
            .last
            .map_or("first look", |l| if l == hash { "same" } else { "changed" });
        w.last = Some(hash);
        let first: Vec<String> = words
            .iter()
            .take(12)
            .map(|&x| {
                let f = f32::from_bits(x);
                if f.is_finite() && (f == 0.0 || (1e-4..1e7).contains(&f.abs())) {
                    format!("{f:.3}")
                } else {
                    format!("{x:#x}")
                }
            })
            .collect();
        tracing::info!(
            program = %w.program,
            slot = %w.slot,
            address = format_args!("{:#x}", w.address),
            "watched buffer: {nonzero}/{} words not zero, {changed}, first: [{}]",
            words.len(),
            first.join(", ")
        );
    }
}
