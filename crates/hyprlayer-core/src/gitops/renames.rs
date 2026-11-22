//! Port of git's diffcore-rename.c and diffcore-delta.c; the constants and tie-breaks follow git 2.56.

use std::cmp::{Ordering, Reverse};
use std::collections::HashMap;

use git2::{Oid, Repository};

const MAX_SCORE: u64 = 60_000;
const MIN_SCORE: u64 = 30_000;
const BASENAME_MIN_SCORE: u64 = MIN_SCORE + (MAX_SCORE - MIN_SCORE) / 2;
const CANDIDATES_PER_TARGET: usize = 4;
const HASHBASE: u32 = 107_927;
const MAX_SPAN: u64 = 64;
const BINARY_SNIFF_BYTES: usize = 8000;
const DEFAULT_LIMIT: i64 = 1000;

pub(crate) struct Side<'a> {
    pub(crate) path: &'a str,
    pub(crate) oid: Oid,
    pub(crate) mode: u32,
}

pub(crate) fn rename_detection(repo: &Repository) -> Option<u64> {
    let Some(config) = repo.config().and_then(|mut c| c.snapshot()).ok() else {
        return Some(DEFAULT_LIMIT as u64);
    };
    let string = |key: &str| config.get_string(key).ok();
    let detect = string("status.renames").or_else(|| string("diff.renames"));
    if detect.is_some_and(|v| {
        matches!(
            v.to_ascii_lowercase().as_str(),
            "false" | "no" | "off" | "0" | ""
        )
    }) {
        return None;
    }
    let limit = config
        .get_i64("status.renameLimit")
        .or_else(|_| config.get_i64("diff.renameLimit"))
        .unwrap_or(DEFAULT_LIMIT);
    Some(if limit <= 0 { u64::MAX } else { limit as u64 })
}

fn is_regular(mode: u32) -> bool {
    mode & 0o170000 == 0o100000
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

struct Counted {
    size: u64,
    spans: Vec<(u32, u64)>,
}

fn span_hash(a1: u32, a2: u32) -> u32 {
    a1.wrapping_add(a2.wrapping_mul(0x61)) % HASHBASE
}

fn is_binary(repo: &Repository, path: &str, data: &[u8]) -> bool {
    let attr = repo
        .get_attr(
            std::path::Path::new(path),
            "diff",
            git2::AttrCheckFlags::FILE_THEN_INDEX,
        )
        .ok()
        .flatten();
    let driver_binary = match git2::AttrValue::from_string(attr) {
        git2::AttrValue::False => Some(true),
        git2::AttrValue::String(driver) => repo
            .config()
            .and_then(|mut c| c.snapshot())
            .and_then(|c| c.get_bool(&format!("diff.{driver}.binary")))
            .ok(),
        _ => None,
    };
    driver_binary.unwrap_or_else(|| data[..data.len().min(BINARY_SNIFF_BYTES)].contains(&0))
}

fn count_spans(data: &[u8], binary: bool) -> Counted {
    let text = !binary;
    let mut spans: HashMap<u32, u64> = HashMap::new();
    let (mut n, mut a1, mut a2) = (0u64, 0u32, 0u32);
    let mut bytes = data.iter().peekable();
    while let Some(&c) = bytes.next() {
        if text && c == b'\r' && bytes.peek() == Some(&&b'\n') {
            continue;
        }
        let old1 = a1;
        a1 = (a1 << 7) ^ (a2 >> 25);
        a2 = (a2 << 7) ^ (old1 >> 25);
        a1 = a1.wrapping_add(u32::from(c));
        n += 1;
        if n < MAX_SPAN && c != b'\n' {
            continue;
        }
        *spans.entry(span_hash(a1, a2)).or_default() += n;
        (n, a1, a2) = (0, 0, 0);
    }
    if n > 0 {
        *spans.entry(span_hash(a1, a2)).or_default() += n;
    }
    let mut spans: Vec<(u32, u64)> = spans.into_iter().collect();
    spans.sort_unstable();
    Counted {
        size: data.len() as u64,
        spans,
    }
}

fn copied(src: &[(u32, u64)], dst: &[(u32, u64)]) -> u64 {
    let (mut i, mut j, mut sum) = (0, 0, 0);
    while i < src.len() && j < dst.len() {
        match src[i].0.cmp(&dst[j].0) {
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
            Ordering::Equal => {
                sum += src[i].1.min(dst[j].1);
                i += 1;
                j += 1;
            }
        }
    }
    sum
}

fn similarity(src: &Counted, dst: &Counted, minimum: u64) -> u64 {
    let max = src.size.max(dst.size);
    let delta = max - src.size.min(dst.size);
    if max * (MAX_SCORE - minimum) < delta * MAX_SCORE || dst.size == 0 {
        return 0;
    }
    copied(&src.spans, &dst.spans) * MAX_SCORE / max
}

type Cache<'a> = HashMap<(Oid, &'a str), Option<Counted>>;

fn load<'a>(cache: &mut Cache<'a>, repo: &Repository, side: &Side<'a>) -> bool {
    is_regular(side.mode)
        && cache
            .entry((side.oid, side.path))
            .or_insert_with(|| {
                repo.find_blob(side.oid)
                    .ok()
                    .map(|b| count_spans(b.content(), is_binary(repo, side.path, b.content())))
            })
            .is_some()
}

fn unique_names<'a>(names: impl Iterator<Item = (&'a str, usize)>) -> HashMap<&'a str, usize> {
    let mut seen: HashMap<&str, Option<usize>> = HashMap::new();
    for (name, i) in names {
        seen.entry(name)
            .and_modify(|v| *v = None)
            .or_insert(Some(i));
    }
    seen.into_iter()
        .filter_map(|(k, v)| Some((k, v?)))
        .collect()
}

#[derive(Clone, Copy)]
struct Candidate {
    src: usize,
    dst: usize,
    score: u64,
    same_name: bool,
}

fn rank(a: &Option<Candidate>, b: &Option<Candidate>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(a), Some(b)) => b.score.cmp(&a.score).then(b.same_name.cmp(&a.same_name)),
    }
}

fn record_if_better(slots: &mut [Option<Candidate>], candidate: Option<Candidate>) {
    let worst = (1..slots.len()).fold(0, |worst, i| {
        if rank(&slots[i], &slots[worst]) == Ordering::Greater {
            i
        } else {
            worst
        }
    });
    if rank(&slots[worst], &candidate) == Ordering::Greater {
        slots[worst] = candidate;
    }
}

pub(crate) fn pair(
    repo: &Repository,
    sources: &[Side<'_>],
    targets: &[Side<'_>],
    limit: u64,
) -> Vec<(usize, usize)> {
    let mut used = vec![false; sources.len()];
    let mut done = vec![false; targets.len()];
    let mut pairs = Vec::new();

    let mut by_oid: HashMap<Oid, Vec<usize>> = HashMap::new();
    for (s, side) in sources.iter().enumerate() {
        by_oid.entry(side.oid).or_default().push(s);
    }
    for (d, target) in targets.iter().enumerate() {
        let best = by_oid
            .get(&target.oid)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&s| {
                let source = &sources[s];
                !used[s]
                    && (source.mode == target.mode
                        || (is_regular(source.mode) && is_regular(target.mode)))
            })
            .max_by_key(|&s| {
                (
                    basename(sources[s].path) == basename(target.path),
                    Reverse(s),
                )
            });
        if let Some(s) = best {
            used[s] = true;
            done[d] = true;
            pairs.push((s, d));
        }
    }

    let mut counted: Cache<'_> = HashMap::new();
    let mut score = |s: usize, d: usize, minimum: u64| -> u64 {
        if !load(&mut counted, repo, &sources[s]) || !load(&mut counted, repo, &targets[d]) {
            return 0;
        }
        let (src, dst) = (&sources[s], &targets[d]);
        match (
            &counted[&(src.oid, src.path)],
            &counted[&(dst.oid, dst.path)],
        ) {
            (Some(src), Some(dst)) => similarity(src, dst, minimum),
            _ => 0,
        }
    };

    let src_names = unique_names(
        (0..sources.len())
            .filter(|&s| !used[s])
            .map(|s| (basename(sources[s].path), s)),
    );
    let dst_names = unique_names(
        (0..targets.len())
            .filter(|&d| !done[d])
            .map(|d| (basename(targets[d].path), d)),
    );
    for (name, &s) in &src_names {
        if let Some(&d) = dst_names.get(name) {
            if score(s, d, BASENAME_MIN_SCORE) >= BASENAME_MIN_SCORE {
                used[s] = true;
                done[d] = true;
                pairs.push((s, d));
            }
        }
    }

    let srcs: Vec<usize> = (0..sources.len()).filter(|&s| !used[s]).collect();
    let dsts: Vec<usize> = (0..targets.len()).filter(|&d| !done[d]).collect();
    let cells = (srcs.len() as u64).saturating_mul(dsts.len() as u64);
    if srcs.is_empty() || dsts.is_empty() || cells > limit.saturating_mul(limit) {
        return pairs;
    }

    let mut matrix: Vec<Option<Candidate>> = Vec::with_capacity(dsts.len() * CANDIDATES_PER_TARGET);
    for &d in &dsts {
        let mut slots = [None; CANDIDATES_PER_TARGET];
        for &s in &srcs {
            let candidate = Some(Candidate {
                src: s,
                dst: d,
                score: score(s, d, MIN_SCORE),
                same_name: basename(sources[s].path) == basename(targets[d].path),
            });
            record_if_better(&mut slots, candidate);
        }
        matrix.extend(slots);
    }
    matrix.sort_by(rank);
    for candidate in matrix
        .into_iter()
        .map_while(|c| c.filter(|c| c.score >= MIN_SCORE))
    {
        if done[candidate.dst] || used[candidate.src] {
            continue;
        }
        used[candidate.src] = true;
        done[candidate.dst] = true;
        pairs.push((candidate.src, candidate.dst));
    }
    pairs
}
