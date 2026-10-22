//! Checksums in Properties: MD5, SHA-1 and SHA-256 of a file, computed in one pass on a worker
//! thread, and compared with a hash on the clipboard so a download can be verified at a glance.
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use super::{Plugin, PropertyGroup};
use crate::fs::file_item::{FileItem, format_bytes};
use crate::i18n::{tr, trf};
use crate::ui::icons;

/// A worker gives up when Properties has not asked for its file this long, so closing the dialog
/// stops the hashing of a multi-gigabyte image.
const ABANDON_AFTER_MS: u64 = 1500;

pub struct Sha256 {
    state: [u32; 8],
    buffer: Vec<u8>,
    length: u64,
}

const K256: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be,
    0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa,
    0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85,
    0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
    0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f,
    0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

impl Sha256 {
    pub fn new() -> Self {
        Self {
            state: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19],
            buffer: Vec::with_capacity(64),
            length: 0,
        }
    }

    fn block(state: &mut [u32; 8], chunk: &[u8]) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K256[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.length += data.len() as u64;
        let state = &mut self.state;
        update_blocks(&mut self.buffer, data, |block| Self::block(state, block));
    }

    pub fn finish(mut self) -> String {
        let bits = self.length * 8;
        let mut tail = vec![0x80u8];
        tail.resize(1 + (119 - (self.length as usize % 64)) % 64, 0);
        tail.extend_from_slice(&bits.to_be_bytes());
        self.update(&tail);
        self.state.iter().map(|w| format!("{w:08x}")).collect()
    }
}

/// Feeds `data` to `block` in 64-byte pieces, keeping the remainder in `buffer`.
fn update_blocks(buffer: &mut Vec<u8>, mut data: &[u8], mut block: impl FnMut(&[u8])) {
    if !buffer.is_empty() {
        let take = (64 - buffer.len()).min(data.len());
        buffer.extend_from_slice(&data[..take]);
        data = &data[take..];
        if buffer.len() == 64 {
            block(buffer);
            buffer.clear();
        }
    }
    while data.len() >= 64 {
        let (head, rest) = data.split_at(64);
        block(head);
        data = rest;
    }
    buffer.extend_from_slice(data);
}

pub struct Sha1 {
    state: [u32; 5],
    buffer: Vec<u8>,
    length: u64,
}

impl Sha1 {
    pub fn new() -> Self {
        Self { state: [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0], buffer: Vec::with_capacity(64), length: 0 }
    }

    fn block(state: &mut [u32; 5], chunk: &[u8]) {
        let mut w = [0u32; 80];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = *state;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d, e]) {
            *s = s.wrapping_add(v);
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.length += data.len() as u64;
        let state = &mut self.state;
        update_blocks(&mut self.buffer, data, |block| Self::block(state, block));
    }

    pub fn finish(mut self) -> String {
        let bits = self.length * 8;
        let mut tail = vec![0x80u8];
        tail.resize(1 + (119 - (self.length as usize % 64)) % 64, 0);
        tail.extend_from_slice(&bits.to_be_bytes());
        self.update(&tail);
        self.state.iter().map(|w| format!("{w:08x}")).collect()
    }
}

pub struct Md5 {
    state: [u32; 4],
    buffer: Vec<u8>,
    length: u64,
}

const MD5_SHIFTS: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11,
    16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

impl Md5 {
    pub fn new() -> Self {
        Self { state: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476], buffer: Vec::with_capacity(64), length: 0 }
    }

    fn block(state: &mut [u32; 4], chunk: &[u8]) {
        let mut m = [0u32; 16];
        for (i, word) in m.iter_mut().enumerate() {
            *word = u32::from_le_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        let [mut a, mut b, mut c, mut d] = *state;
        for (i, shift) in MD5_SHIFTS.iter().enumerate() {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let k = ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32;
            let sum = a.wrapping_add(f).wrapping_add(k).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(sum.rotate_left(*shift));
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d]) {
            *s = s.wrapping_add(v);
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.length += data.len() as u64;
        let state = &mut self.state;
        update_blocks(&mut self.buffer, data, |block| Self::block(state, block));
    }

    pub fn finish(mut self) -> String {
        let bits = self.length * 8;
        let mut tail = vec![0x80u8];
        tail.resize(1 + (119 - (self.length as usize % 64)) % 64, 0);
        tail.extend_from_slice(&bits.to_le_bytes());
        self.update(&tail);
        self.state.iter().flat_map(|w| w.to_le_bytes()).map(|b| format!("{b:02x}")).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hashes {
    pub md5: String,
    pub sha1: String,
    pub sha256: String,
}

impl Hashes {
    /// The name of the hash that equals `candidate` (a hex string, case and spaces ignored).
    pub fn matching(&self, candidate: &str) -> Option<&'static str> {
        let candidate: String = candidate.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
        [("MD5", &self.md5), ("SHA-1", &self.sha1), ("SHA-256", &self.sha256)].into_iter().find(|(_, h)| **h == candidate).map(|(n, _)| n)
    }
}

/// Whether `text` looks like a hash someone copied: hex digits of an MD5, SHA-1 or SHA-256 length.
pub fn looks_like_hash(text: &str) -> bool {
    let text = text.trim();
    matches!(text.len(), 32 | 40 | 64) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

struct Job {
    done: AtomicU64,
    total: u64,
    /// Milliseconds since `epoch` at which Properties last asked about the file.
    seen: AtomicU64,
    epoch: Instant,
    cancelled: AtomicBool,
    result: Mutex<Option<Result<Hashes, String>>>,
}

impl Job {
    fn touch(&self) {
        self.seen.store(self.epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
    }

    fn abandoned(&self) -> bool {
        self.epoch.elapsed().as_millis() as u64 > self.seen.load(Ordering::Relaxed) + ABANDON_AFTER_MS
    }
}

fn run_job(path: &Path, job: &Job) -> Result<Hashes, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let (mut md5, mut sha1, mut sha256) = (Md5::new(), Sha1::new(), Sha256::new());
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if job.abandoned() {
            job.cancelled.store(true, Ordering::Relaxed);
            return Err(String::new());
        }
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        md5.update(&buf[..n]);
        sha1.update(&buf[..n]);
        sha256.update(&buf[..n]);
        job.done.fetch_add(n as u64, Ordering::Relaxed);
    }
    Ok(Hashes { md5: md5.finish(), sha1: sha1.finish(), sha256: sha256.finish() })
}

type Key = (PathBuf, Option<SystemTime>, u64);

#[derive(Default)]
pub struct Checksum {
    jobs: HashMap<Key, Arc<Job>>,
    /// The text on the clipboard when the file was last looked at, read once per file.
    clipboard: HashMap<Key, Option<String>>,
}

impl Plugin for Checksum {
    fn id(&self) -> &'static str {
        "checksum"
    }

    fn name(&self) -> &'static str {
        tr("Checksums")
    }

    fn description(&self) -> &'static str {
        tr("Shows MD5, SHA-1 and SHA-256 in Properties and compares them with a hash on the clipboard")
    }

    fn icon(&self) -> &'static str {
        icons::SHIELD
    }

    fn busy(&self) -> bool {
        self.jobs.values().any(|j| j.result.lock().is_ok_and(|r| r.is_none()))
    }

    fn properties(&mut self, item: &FileItem) -> Vec<PropertyGroup> {
        if item.is_dir {
            return Vec::new();
        }
        let key: Key = (item.path.clone(), item.modified, item.size);
        // Only the file on screen keeps hashing
        self.jobs.retain(|k, job| *k == key || job.result.lock().is_ok_and(|r| r.is_some()));
        let job = self
            .jobs
            .entry(key.clone())
            .or_insert_with(|| {
                let job = Arc::new(Job {
                    done: AtomicU64::new(0),
                    total: item.size,
                    seen: AtomicU64::new(0),
                    epoch: Instant::now(),
                    cancelled: AtomicBool::new(false),
                    result: Mutex::new(None),
                });
                let worker = Arc::clone(&job);
                let path = item.path.clone();
                std::thread::spawn(move || {
                    let result = run_job(&path, &worker);
                    if let Ok(mut slot) = worker.result.lock() {
                        *slot = Some(result);
                    }
                });
                job
            })
            .clone();
        job.touch();

        let result = job.result.lock().ok().and_then(|r| r.clone());
        match result {
            None => {
                let percent = job.done.load(Ordering::Relaxed).saturating_mul(100).checked_div(job.total).unwrap_or(0);
                let busy = trf("Calculating… {}%", &[&percent]);
                vec![PropertyGroup { title: tr("Checksums"), rows: vec![(tr("Progress"), format!("{busy} ({})", format_bytes(job.total)))] }]
            }
            Some(Err(error)) if error.is_empty() => {
                // The worker gave up while the dialog was away: start over
                self.jobs.remove(&key);
                Vec::new()
            }
            Some(Err(error)) => vec![PropertyGroup { title: tr("Checksums"), rows: vec![(tr("Error"), error)] }],
            Some(Ok(hashes)) => {
                let mut rows = vec![("MD5", hashes.md5.clone()), ("SHA-1", hashes.sha1.clone()), ("SHA-256", hashes.sha256.clone())];
                let clip = self.clipboard.entry(key).or_insert_with(crate::clipboard::get_text);
                if let Some(text) = clip.as_deref().filter(|t| looks_like_hash(t)) {
                    let verdict = match hashes.matching(text) {
                        Some(name) => trf("Match: the clipboard holds the {}", &[&name]),
                        None => tr("No match with the hash on the clipboard").to_owned(),
                    };
                    rows.push((tr("Clipboard"), verdict));
                }
                vec![PropertyGroup { title: tr("Checksums"), rows }]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash_bytes(data: &[u8]) -> Hashes {
        let (mut md5, mut sha1, mut sha256) = (Md5::new(), Sha1::new(), Sha256::new());
        md5.update(data);
        sha1.update(data);
        sha256.update(data);
        Hashes { md5: md5.finish(), sha1: sha1.finish(), sha256: sha256.finish() }
    }

    #[test]
    fn known_digests() {
        let empty = hash_bytes(b"");
        assert_eq!(empty.md5, "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(empty.sha1, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(empty.sha256, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        let abc = hash_bytes(b"abc");
        assert_eq!(abc.md5, "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(abc.sha1, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(abc.sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn padding_boundaries_and_chunked_updates() {
        // 55, 56 and 64 bytes straddle where the length no longer fits the last block
        let sha = |n: usize| hash_bytes(&vec![b'a'; n]).sha256;
        assert_eq!(sha(55), "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318");
        assert_eq!(sha(56), "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a");
        assert_eq!(sha(64), "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb");
        let data: Vec<u8> = (0..1000u32).map(|i| (i * 7) as u8).collect();
        let mut chunked = Sha256::new();
        for piece in data.chunks(37) {
            chunked.update(piece);
        }
        assert_eq!(chunked.finish(), hash_bytes(&data).sha256);
        let million = vec![b'a'; 1_000_000];
        assert_eq!(hash_bytes(&million).sha1, "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
        assert_eq!(hash_bytes(&million).md5, "7707d6ae4e027c70eea2a935c2296f21");
    }

    #[test]
    fn clipboard_comparison() {
        let h = hash_bytes(b"abc");
        assert_eq!(h.matching("  BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD\n"), Some("SHA-256"));
        assert_eq!(h.matching("900150983cd24fb0d6963f7d28e17f72"), Some("MD5"));
        assert_eq!(h.matching("00"), None);
        assert!(looks_like_hash("900150983cd24fb0d6963f7d28e17f72 "));
        assert!(!looks_like_hash("hello world"));
        assert!(!looks_like_hash("zz0150983cd24fb0d6963f7d28e17f72"));
    }

    #[test]
    fn hashes_a_file_and_gives_up_when_abandoned() {
        let path = std::env::temp_dir().join(format!("ski-hash-test-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        let job = Job { done: AtomicU64::new(0), total: 3, seen: AtomicU64::new(0), epoch: Instant::now(), cancelled: AtomicBool::new(false), result: Mutex::new(None) };
        let hashes = run_job(&path, &job).unwrap();
        assert_eq!(hashes, hash_bytes(b"abc"));
        assert_eq!(job.done.load(Ordering::Relaxed), 3);
        std::fs::remove_file(&path).ok();
    }
}
