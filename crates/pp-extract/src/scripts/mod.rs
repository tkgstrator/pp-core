//! Dialogue VM script extraction for the verified Japanese cartridges.

mod pp4;
mod pp6;
mod semantic;

use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fmt::Write;
use std::ops::Range;
use std::path::Path;

use crate::Game;

pub fn extract(rom: &[u8], out: &Path, game: Game) -> Result<()> {
    let actual = digest(rom);
    let pp4_v0 = game == Game::Pp4 && actual == crate::rom::PP4_V0_SHA256;
    ensure!(
        game.accepts_sha256(&actual),
        "Expected verified Japanese {} ROM",
        game.code()
    );
    match game {
        Game::Pp4 => pp4::extract(rom, &out.join("native"), out, &actual, pp4_v0),
        Game::Pp6 => pp6::extract(rom, &out.join("native"), out, &actual),
    }
}

const BASE: usize = 0x0800_0000;

fn bounds(data: &[u8], desc: usize) -> Option<(usize, usize)> {
    if desc + 4 > data.len() {
        return None;
    }
    let start = word(data, desc).checked_sub(BASE)?;
    if !(start < desc && desc - start <= 0x4000) {
        return None;
    }
    let mut end = desc;
    while end > desc.saturating_sub(3) && data[end - 1] == 0 {
        end -= 1;
    }
    (end > start && data[end - 1] == 0xff).then_some((start, end))
}

fn discover_descriptors(data: &[u8], scan: Range<usize>) -> BTreeMap<usize, (usize, usize)> {
    let mut candidates: BTreeMap<_, _> = scan
        .step_by(4)
        .filter_map(|desc| bounds(data, desc).map(|span| (desc, span)))
        .filter(|&(_, (start, _))| start.is_multiple_of(4))
        .collect();
    let mut first_desc = BTreeMap::new();
    for (&desc, &(start, _)) in &candidates {
        first_desc.entry(start).or_insert(desc);
    }
    candidates.retain(|desc, (start, _)| first_desc[start] == *desc);
    let candidate_addrs = candidates.keys().copied().collect::<Vec<_>>();
    candidates.retain(|desc, (start, _)| {
        candidate_addrs.get(candidate_addrs.partition_point(|&x| x < *start)) == Some(desc)
    });
    candidates
}

fn event_candidates(data: &[u8], ranges: &[(usize, usize)]) -> BTreeMap<usize, (usize, usize)> {
    ranges
        .iter()
        .flat_map(|&(start, end)| discover_descriptors(data, start..end))
        .filter(|&(_, (start, end))| {
            let body = &data[start..end];
            matches!(body[0], 0xf8 | 0xf9 | 0xfc | 0xfd | 0xfe) || body.contains(&0xf8)
        })
        .collect()
}

fn referenced_candidates(data: &[u8]) -> BTreeMap<usize, (usize, usize)> {
    let mut candidates = event_candidates(data, &[(0, data.len())]);
    let mut referenced = HashSet::new();
    for offset in (0..data.len().saturating_sub(3)).step_by(4) {
        if let Some(descriptor) = word(data, offset).checked_sub(BASE)
            && let Some(&(start, _)) = candidates.get(&descriptor)
            && !(start..descriptor + 4).contains(&offset)
        {
            referenced.insert(descriptor);
        }
    }
    candidates.retain(|descriptor, _| referenced.contains(descriptor));
    candidates
}

pub(crate) fn export_scanned_events(
    data: &[u8],
    out: &Path,
    listing_format: Option<NativeListing>,
) -> Result<usize> {
    let events = referenced_candidates(data);
    let count = events.len();
    if count != 0 {
        write_native_events(data, out, &[(0, data.len())], listing_format, events, true)?;
    }
    Ok(count)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PortraitSentinels {
    Standard,
    Unchecked,
    Pp7,
}

#[derive(Clone, Copy)]
pub(crate) struct NativeListing {
    pub dispatch: usize,
    pub effect_dispatch: Option<usize>,
    pub event_entry: usize,
    pub indexed_entrypoints: &'static [(usize, usize)],
    pub unchecked_opcodes: &'static [u8],
    pub operand_overrides: &'static [(u8, i8)],
    pub portrait_sentinels: PortraitSentinels,
}

// Both PP5 Rev 0 variants use the same ordered set of Thumb handlers. Hash
// their offsets relative to the first handler so table and code can move.
pub(crate) fn discover_pp5_listing(data: &[u8]) -> Option<NativeListing> {
    const TABLE_LENGTH: usize = (112 + 64) * 4;
    const HANDLERS: &str = "0131424596907f9bc9e8dce384b48e2fc40802231d3b363854d69371b4780370";
    const ENTRY: &str = "79b27b007f822ae9303b8ee74c66c672348f89084ab3e568d885dabaa64d5ed7";
    let mut found = None;
    for (index, table) in data.windows(TABLE_LENGTH).step_by(4).enumerate() {
        let first = word(table, 0);
        let Some(entry) = (first & !1).checked_sub(BASE + 0x7b0) else {
            continue;
        };
        if first & 1 == 0
            || word(table, 4).checked_sub(first) != Some(0x18)
            || word(table, 8).checked_sub(first) != Some(0x38)
        {
            continue;
        }
        let Some(entry_code) = data.get(entry..).and_then(|bytes| bytes.get(..32)) else {
            continue;
        };
        if digest(entry_code) != ENTRY {
            continue;
        }
        let pointers: Vec<_> = table.chunks_exact(4).map(|bytes| word(bytes, 0)).collect();
        if pointers.iter().any(|&pointer| {
            pointer & 1 == 0 || !(BASE..BASE + data.len()).contains(&(pointer & !1))
        }) {
            continue;
        }
        let mut digest = Sha256::new();
        for pointer in pointers {
            digest.update((pointer as u32).wrapping_sub(first as u32).to_le_bytes());
        }
        if format!("{:x}", digest.finalize()) != HANDLERS {
            continue;
        }
        if found.is_some() {
            return None;
        }
        let dispatch = index * 4;
        found = Some(NativeListing {
            dispatch,
            effect_dispatch: Some(dispatch + 112 * 4),
            event_entry: entry,
            indexed_entrypoints: &[],
            unchecked_opcodes: &[],
            operand_overrides: &[],
            portrait_sentinels: PortraitSentinels::Standard,
        });
    }
    found
}

pub(crate) fn export_native_events(
    data: &[u8],
    out: &Path,
    ranges: &[(usize, usize)],
    listing_format: Option<NativeListing>,
) -> Result<()> {
    write_native_events(
        data,
        out,
        ranges,
        listing_format,
        event_candidates(data, ranges),
        false,
    )
}

fn write_native_events(
    data: &[u8],
    out: &Path,
    ranges: &[(usize, usize)],
    listing_format: Option<NativeListing>,
    events: BTreeMap<usize, (usize, usize)>,
    require_reference: bool,
) -> Result<()> {
    let native = out.join("native");
    let mut index = String::from("descriptor\tbody\tend_exclusive\tsha256\tfile\n");
    std::fs::create_dir_all(native.join("events"))?;
    let mut records: HashSet<_> = events.keys().copied().collect();
    let mut byte_count = 0;
    let glyphs = listing_format.map(|_| glyph_table(false));
    let mut listings = 0;
    let mut event_references = Vec::new();
    let mut sound_requests = Vec::new();
    let mut conditions = Vec::new();
    let mut switches = Vec::new();
    let mut listing_index = String::from("descriptor\tfile\n");
    let mut queue: VecDeque<_> = events.into_iter().collect();
    let mut occupied: BTreeMap<_, _> = queue
        .iter()
        .map(|&(descriptor, (start, _))| (start, descriptor + 4))
        .collect();
    let scanned_count = queue.len();
    let entries = listing_format.map_or_else(Vec::new, |format| {
        native_entrypoints(
            data,
            format.event_entry,
            ranges[0].0,
            format.indexed_entrypoints,
        )
    });
    for entry in &entries {
        let target = entry.descriptor;
        let (start, end) = entry.span;
        if !records.contains(&target)
            && occupied
                .range(..target + 4)
                .next_back()
                .is_none_or(|(_, &stop)| stop <= start)
        {
            records.insert(target);
            occupied.insert(start, target + 4);
            queue.push_back((target, (start, end)));
        }
    }
    let entrypoint_candidates = records.len() - scanned_count;
    let mut linked = String::from("descriptor\tsource_descriptor\tsource_offset\n");
    while let Some((descriptor, (start, end))) = queue.pop_front() {
        let body = &data[start..end];
        let path = format!("events/{descriptor:06x}.bin");
        std::fs::write(native.join(&path), body)?;
        writeln!(
            index,
            "{descriptor:06x}\t{start:06x}\t{end:06x}\t{}\t{path}",
            digest(body)
        )?;
        if let (Some(chars), Some(format)) = (&glyphs, listing_format)
            && let Ok(Some(listing)) =
                pp4::native_listing(data, descriptor, start, end, chars, format)
        {
            let path = format!("events/{descriptor:06x}.tsv");
            write_file(&native.join(&path), &listing.text)?;
            sound_requests.extend(
                listing
                    .sound_requests
                    .into_iter()
                    .map(|(offset, sound_id)| (sound_id, descriptor, offset)),
            );
            conditions.extend(
                listing
                    .conditions
                    .into_iter()
                    .map(|(offset, on_true, on_false)| (descriptor, offset, on_true, on_false)),
            );
            switches.extend(
                listing
                    .switches
                    .into_iter()
                    .map(|entry| (descriptor, entry)),
            );
            for (offset, target, kind) in listing.references {
                if target.is_multiple_of(4)
                    && target < data.len().saturating_sub(3)
                    && !records.contains(&target)
                    && let Some((start, end)) = bounds(data, target)
                    && occupied
                        .range(..target + 4)
                        .next_back()
                        .is_none_or(|(_, &stop)| stop <= start)
                {
                    records.insert(target);
                    occupied.insert(start, target + 4);
                    queue.push_back((target, (start, end)));
                    writeln!(linked, "{target:06x}\t{descriptor:06x}\t{offset:06x}")?;
                }
                event_references.push((descriptor, (offset, target, kind)));
            }
            writeln!(listing_index, "{descriptor:06x}\t{path}")?;
            listings += 1;
        }
        byte_count += body.len();
    }
    ensure!(!records.is_empty(), "No native script records found");
    let mut references = String::from("pointer_offset\tdescriptor\n");
    let mut reference_count = 0;
    for offset in (0..data.len().saturating_sub(3)).step_by(4) {
        if let Some(descriptor) = word(data, offset).checked_sub(BASE)
            && records.contains(&descriptor)
        {
            writeln!(references, "{offset:06x}\t{descriptor:06x}")?;
            reference_count += 1;
        }
    }
    write_file(&native.join("references.tsv"), &references)?;
    let mut manifest = serde_json::json!({
        "rom_sha256": digest(data),
        "vm_decoding": "native bodies only",
        "descriptor_scan": ranges.iter().map(|&(start, end)| [format!("{start:#x}"), format!("{end:#x}")]).collect::<Vec<_>>(),
        "candidate_records": records.len(),
        "aligned_pointer_references": reference_count,
        "source_bytes": byte_count,
    });
    if require_reference {
        manifest["candidate_filter"] =
            serde_json::json!("aligned pointer outside the event body and descriptor");
    }
    if let Some(format) = listing_format {
        write_file(&native.join("listings.tsv"), &listing_index)?;
        write_file(&native.join("linked.tsv"), &linked)?;
        manifest["linked_candidates"] =
            serde_json::json!(records.len() - scanned_count - entrypoint_candidates);
        manifest["entrypoint_candidates"] = serde_json::json!(entrypoint_candidates);
        let mut edges =
            String::from("descriptor\tsource_offset\tkind\ttarget_descriptor\ttarget_status\n");
        let mut adjacency = BTreeMap::<usize, Vec<usize>>::new();
        let mut matched = 0;
        let mut edge_count = 0;
        for (descriptor, (offset, target, kind)) in event_references {
            if target >= data.len().saturating_sub(3) {
                continue;
            }
            let status = if records.contains(&target) {
                matched += 1;
                adjacency.entry(descriptor).or_default().push(target);
                "exported_candidate"
            } else if bounds(data, target).is_some() {
                "descriptor_outside_candidate_set"
            } else {
                "unresolved_rom_target"
            };
            writeln!(
                edges,
                "{descriptor:06x}\t{offset:06x}\t{kind}\t{target:06x}\t{status}"
            )?;
            edge_count += 1;
        }
        write_file(&native.join("edges.tsv"), &edges)?;
        manifest["provisional_event_edges"] = serde_json::json!(edge_count);
        manifest["edges_to_exported_candidates"] = serde_json::json!(matched);
        write_entrypoints(&native, &entries, |descriptor| {
            records.contains(&descriptor)
        })?;
        let (paths, reached) = entrypoint_reachability(&entries, &records, &adjacency)?;
        write_file(&native.join("reachable.tsv"), &paths)?;
        manifest["direct_native_entrypoints"] = serde_json::json!(
            entries
                .iter()
                .map(|entry| entry.call)
                .collect::<HashSet<_>>()
                .len()
        );
        manifest["direct_native_entrypoint_targets"] = serde_json::json!(entries.len());
        manifest["statically_reachable_candidates"] = serde_json::json!(reached);
        manifest["vm_event_entry"] = serde_json::json!(format!("{:#x}", format.event_entry));
        manifest["vm_decoding"] =
            serde_json::json!("Native bodies with PP4-compatible provisional listings");
        manifest["vm_dispatch"] = serde_json::json!(format!("{:#x}", format.dispatch));
        manifest["unchecked_extended_opcodes"] = serde_json::json!(format.unchecked_opcodes);
        if format.portrait_sentinels == PortraitSentinels::Unchecked {
            manifest["unchecked_portrait_sentinels"] = serde_json::json!(true);
        }
        sound_requests.sort_unstable();
        let mut sound_index = String::from("encoded_sound_id\tdescriptor\tsource_offset\n");
        for &(id, descriptor, offset) in &sound_requests {
            writeln!(sound_index, "{id:04x}\t{descriptor:06x}\t{offset:06x}")?;
        }
        write_file(&native.join("sound-requests.tsv"), &sound_index)?;
        manifest["encoded_sound_requests"] = serde_json::json!(sound_requests.len());
        conditions.sort_unstable();
        let mut condition_index =
            String::from("descriptor\tsource_offset\ttrue_offset\tfalse_offset\n");
        for &(descriptor, offset, on_true, on_false) in &conditions {
            let on_false = on_false.map_or_else(String::new, |at| format!("{at:06x}"));
            writeln!(
                condition_index,
                "{descriptor:06x}\t{offset:06x}\t{on_true:06x}\t{on_false}"
            )?;
        }
        write_file(&native.join("conditions.tsv"), &condition_index)?;
        manifest["condition_branches"] = serde_json::json!(conditions.len());
        switches.sort_unstable_by_key(|(descriptor, entry)| (*descriptor, entry.offset));
        let mut switch_index =
            String::from("descriptor\tsource_offset\tslot\tcase\ttarget_offset\n");
        for (descriptor, entry) in &switches {
            let offset = entry.offset;
            let slot = entry.slot;
            for &(case, target) in &entry.cases {
                writeln!(
                    switch_index,
                    "{descriptor:06x}\t{offset:06x}\t{slot:02x}\t{case}\t{target:06x}"
                )?;
            }
            let target = entry.end.map_or_else(String::new, |at| format!("{at:06x}"));
            writeln!(
                switch_index,
                "{descriptor:06x}\t{offset:06x}\t{slot:02x}\tend\t{target}"
            )?;
        }
        write_file(&native.join("switches.tsv"), &switch_index)?;
        manifest["provisional_switches"] = serde_json::json!(switches.len());
        manifest["decoded_listings"] = serde_json::json!(listings);
        manifest["glyph_mapping"] = serde_json::json!("inherited PP4 assignments");
    }
    write_file(&native.join("index.tsv"), &index)?;
    write_file(
        &native.join("manifest.json"),
        &serde_json::to_string_pretty(&manifest)?,
    )
}

struct NativeEntry {
    call: usize,
    mode: u8,
    table_index: Option<usize>,
    subtable_index: Option<usize>,
    literal: usize,
    descriptor: usize,
    span: (usize, usize),
}

fn write_entrypoints(
    out: &Path,
    entries: &[NativeEntry],
    contains: impl Fn(usize) -> bool,
) -> Result<()> {
    let mut entrypoints = String::from(
        "call_offset\tliteral_offset\tdescriptor\ttarget_status\tmode\ttable_index\tsubtable_index\n",
    );
    for entry in entries {
        let status = if contains(entry.descriptor) {
            "exported_candidate"
        } else {
            "descriptor_outside_candidate_set"
        };
        writeln!(
            entrypoints,
            "{:06x}\t{:06x}\t{:06x}\t{status}\t{}\t{}\t{}",
            entry.call,
            entry.literal,
            entry.descriptor,
            entry.mode,
            entry
                .table_index
                .map_or_else(String::new, |index| index.to_string()),
            entry
                .subtable_index
                .map_or_else(String::new, |index| index.to_string())
        )?;
    }
    write_file(&out.join("entrypoints.tsv"), &entrypoints)
}

fn native_entrypoints(
    data: &[u8],
    event_entry: usize,
    code_end: usize,
    indexed_entrypoints: &[(usize, usize)],
) -> Vec<NativeEntry> {
    let mut entries = Vec::new();
    let halfword = |at: usize| u16::from_le_bytes([data[at], data[at + 1]]);
    for call in (4..code_end.saturating_sub(3)).step_by(2) {
        let high = halfword(call);
        let low = halfword(call + 2);
        if high & 0xf800 != 0xf000 || low & 0xf800 != 0xf800 {
            continue;
        }
        let displacement = (i32::from(high & 0x7ff) << 12) | (i32::from(low & 0x7ff) << 1);
        let signed_displacement = (displacement << 9) >> 9;
        let before = halfword(call - 2);
        let earlier = halfword(call - 4);
        let indexed_mode_site = if call >= 14
            && before & 0xff00 == 0x2000
            && [halfword(call - 8), halfword(call - 6), earlier] == [0x0080, 0x1840, 0x6801]
        {
            if call >= 16
                && halfword(call - 16) >> 8 == 0x49
                && halfword(call - 14) >> 8 == 0x48
                && matches!(
                    (halfword(call - 12), halfword(call - 10)),
                    (0x7cc0, 0x380a) | (0x7b40 | 0x7c00, 0x381e)
                )
            {
                Some(call - 16)
            } else if call >= 16
                && halfword(call - 16) >> 8 == 0x49
                && [
                    halfword(call - 14),
                    halfword(call - 12),
                    halfword(call - 10),
                ] == [0x4642, 0x7bd0, 0x380a]
            {
                Some(call - 16)
            } else if call >= 16
                && halfword(call - 16) >> 8 == 0x49
                && [
                    halfword(call - 14),
                    halfword(call - 12),
                    halfword(call - 10),
                ] == [0x1c10, 0x30f8, 0x6800]
            {
                Some(call - 16)
            } else if halfword(call - 14) >> 8 == 0x49
                && [halfword(call - 12), halfword(call - 10)] == [0x6820, 0x8800]
            {
                Some(call - 14)
            } else if halfword(call - 14) >> 8 == 0x49
                && matches!(halfword(call - 12), 0x7b60 | 0x7be0 | 0x7c18)
                && halfword(call - 10) == 0x380a
            {
                Some(call - 14)
            } else {
                None
            }
        } else {
            None
        };
        let (load, site, mode, dereference) = match (call + 4)
            .checked_add_signed(signed_displacement as isize)
        {
            Some(target) if target == event_entry && before & 0xff00 == 0x4800 => {
                (before, call - 2, 0, None)
            }
            Some(target)
                if target == event_entry
                    && earlier & 0xff00 == 0x4800
                    && before & 0xf83f == 0x6800 =>
            {
                (
                    earlier,
                    call - 4,
                    0,
                    Some(4 * usize::from((before >> 6) & 31)),
                )
            }
            Some(target)
                if target == event_entry
                    || (target == event_entry + 0x10 && indexed_mode_site.is_some()) =>
            {
                let Some(&(_, count)) = indexed_entrypoints.iter().find(|&&(site, _)| site == call)
                else {
                    continue;
                };
                let explicit_mode = target != event_entry;
                let mode = if explicit_mode { before as u8 } else { 0 };
                let nested = if !explicit_mode && call >= 12 {
                    match [
                        halfword(call - 12),
                        halfword(call - 10),
                        halfword(call - 8),
                        halfword(call - 6),
                        earlier,
                        before,
                    ] {
                        [0x6811, 0x0080, 0x1840, 0x6800, 0x2800, 0xd009]
                            if call >= 32 && halfword(call - 32) >> 8 == 0x4b =>
                        {
                            Some((32, 20))
                        }
                        [0x6809, 0x00b0, 0x1840, 0x6800, 0x2800, 0xd005]
                            if call >= 32 && halfword(call - 32) >> 8 == 0x4a =>
                        {
                            Some((32, 19))
                        }
                        [0x6821, 0x0080, 0x1840, 0x6800, 0x2800, 0xd009 | 0xd00a]
                            if call >= 20 && halfword(call - 20) >> 8 == 0x49 =>
                        {
                            Some((20, 12))
                        }
                        [0x6809, 0x00a8, 0x1840, 0x6800, 0x2800, 0xd006 | 0xd008]
                            if call >= 34 && halfword(call - 34) >> 8 == 0x4b =>
                        {
                            Some((34, 19))
                        }
                        [0x6809, 0x00a8, 0x1840, 0x6800, 0x2800, 0xd005]
                            if call >= 32 && halfword(call - 32) >> 8 == 0x4a =>
                        {
                            Some((32, 19))
                        }
                        [0x6809, 0x0098, 0x1840, 0x6800, 0x2800, 0xd006]
                            if call >= 28 && halfword(call - 28) >> 8 == 0x4a =>
                        {
                            Some((28, 20))
                        }
                        [0x6801, 0x0090, 0x1840, 0x6800, 0x2800, 0xd00d]
                            if call >= 22 && halfword(call - 22) >> 8 == 0x49 =>
                        {
                            Some((22, 16))
                        }
                        [0x6811, 0x0080, 0x1840, 0x6800, 0x2800, 0xd00a]
                            if call >= 32 && halfword(call - 32) >> 8 == 0x4c =>
                        {
                            Some((32, 19))
                        }
                        [0x6821, 0x0090, 0x1840, 0x6800, 0x2800, 0xd009]
                            if call >= 20 && halfword(call - 20) >> 8 == 0x48 =>
                        {
                            Some((20, 12))
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                let site = if let Some((distance, _)) = nested {
                    call - distance
                } else if explicit_mode {
                    let Some(site) = indexed_mode_site else {
                        continue;
                    };
                    site
                } else if call >= 14
                    && halfword(call - 14) >> 8 == 0x49
                    && matches!(halfword(call - 12), 0x1c10 | 0x1c18)
                    && matches!(halfword(call - 10), 0x30f8 | 0x30fc)
                    && halfword(call - 8) == 0x6800
                    && [halfword(call - 6), earlier, before] == [0x0080, 0x1840, 0x6800]
                {
                    call - 14
                } else if call >= 14
                    && halfword(call - 14) >> 8 == 0x49
                    && halfword(call - 12) >> 8 == 0x48
                    && matches!(halfword(call - 10), 0x7cc0 | 0x7b40 | 0x7bc0 | 0x7c00)
                    && halfword(call - 8) >> 8 == 0x38
                    && [halfword(call - 6), earlier, before] == [0x0080, 0x1840, 0x6800]
                {
                    call - 14
                } else if call >= 16
                    && halfword(call - 16) >> 8 == 0x49
                    && halfword(call - 14) >> 8 == 0x48
                    && [halfword(call - 12), halfword(call - 10), halfword(call - 8)]
                        == [0x6800, 0x8800, 0x3801]
                    && [halfword(call - 6), earlier, before] == [0x0080, 0x1840, 0x6800]
                {
                    call - 16
                } else if call >= 10
                    && halfword(call - 10) >> 8 == 0x48
                    && matches!(halfword(call - 8), 0x1e69 | 0x1e61 | 0x6811 | 0x8809)
                    && [halfword(call - 6), earlier, before] == [0x0089, 0x1809, 0x6808]
                {
                    call - 10
                } else if call >= 12
                    && halfword(call - 12) >> 8 == 0x49
                    && matches!(
                        halfword(call - 10),
                        0x1c20 | 0x7820 | 0x7828 | 0x7b50 | 0x7b68 | 0x7c58 | 0x7c60
                    )
                    && [halfword(call - 8), halfword(call - 6), earlier, before]
                        == [0x3801, 0x0080, 0x1840, 0x6800]
                {
                    call - 12
                } else if call >= 12
                    && halfword(call - 12) >> 8 == 0x49
                    && halfword(call - 10) >> 8 == 0x4c
                    && matches!(
                        (halfword(call - 8), halfword(call - 6)),
                        (0x7860, 0x0080) | (0x7862, 0x0090)
                    )
                    && [earlier, before] == [0x1840, 0x6800]
                {
                    call - 12
                } else if call >= 12
                    && halfword(call - 12) >> 8 == 0x49
                    && [
                        halfword(call - 10),
                        halfword(call - 8),
                        halfword(call - 6),
                        earlier,
                        before,
                    ] == [0x6820, 0x8800, 0x0080, 0x1840, 0x6800]
                {
                    call - 12
                } else if call >= 14
                    && halfword(call - 14) >> 8 == 0x49
                    && halfword(call - 12) >> 8 == 0x4c
                    && [
                        halfword(call - 10),
                        halfword(call - 8),
                        halfword(call - 6),
                        earlier,
                        before,
                    ] == [0x6820, 0x8800, 0x0080, 0x1840, 0x6800]
                {
                    call - 14
                } else if call >= 10
                    && halfword(call - 10) >> 8 == 0x49
                    && [halfword(call - 8), halfword(call - 6), earlier, before]
                        == [0x7812, 0x0090, 0x1840, 0x6800]
                {
                    call - 10
                } else if call >= 16
                    && halfword(call - 16) >> 8 == 0x49
                    && [
                        halfword(call - 14),
                        halfword(call - 12),
                        halfword(call - 10),
                    ] == [0x23a8, 0x005b, 0x18d0]
                    && [halfword(call - 8), halfword(call - 6), earlier, before]
                        == [0x7800, 0x0080, 0x1840, 0x6800]
                {
                    call - 16
                } else if call >= 8
                    && matches!(
                        (halfword(call - 8) >> 8, halfword(call - 6), earlier, before),
                        (0x49, 0x0080, 0x1840, 0x6800) | (0x48, 0x0099, 0x1809, 0x6808)
                    )
                {
                    call - 8
                } else {
                    continue;
                };
                let literal = ((site + 4) & !3) + 4 * usize::from(halfword(site) & 0xff);
                if literal + 4 > data.len() {
                    continue;
                }
                let Some(table) = word(data, literal).checked_sub(BASE) else {
                    continue;
                };
                if table + count * 4 > data.len() {
                    continue;
                }
                for (index, slot) in (table..table + count * 4).step_by(4).enumerate() {
                    let (start, length) = if let Some((_, lengths_distance)) = nested {
                        let Some(counts) = table.checked_sub(lengths_distance) else {
                            continue;
                        };
                        let Some(start) = word(data, slot).checked_sub(BASE) else {
                            continue;
                        };
                        (start, usize::from(data[counts + index]))
                    } else {
                        (slot, 1)
                    };
                    if start + length * 4 > data.len() {
                        continue;
                    }
                    for (subindex, at) in (start..start + length * 4).step_by(4).enumerate() {
                        if let Some(descriptor) = word(data, at).checked_sub(BASE)
                            && descriptor.is_multiple_of(4)
                            && let Some(span) = bounds(data, descriptor)
                        {
                            entries.push(NativeEntry {
                                call,
                                mode,
                                table_index: Some(index),
                                subtable_index: nested.is_some().then_some(subindex),
                                literal,
                                descriptor,
                                span,
                            });
                        }
                    }
                }
                continue;
            }
            // The default wrapper calls this implementation at +0x10 with
            // r0 = mode and r1 = descriptor. Accept either setup order.
            Some(target) if target == event_entry + 0x10 => {
                match (earlier & 0xff00, before & 0xff00) {
                    (0x4900, 0x2000) => (earlier, call - 4, before as u8, None),
                    (0x2000, 0x4900) => (before, call - 2, earlier as u8, None),
                    _ => continue,
                }
            }
            _ => continue,
        };
        let literal = ((site + 4) & !3) + 4 * usize::from(load & 0xff);
        if literal + 4 > data.len() {
            continue;
        }
        let Some(mut descriptor) = word(data, literal).checked_sub(BASE) else {
            continue;
        };
        if let Some(offset) = dereference {
            let pointer = descriptor + offset;
            if pointer + 4 > data.len() {
                continue;
            }
            let Some(target) = word(data, pointer).checked_sub(BASE) else {
                continue;
            };
            descriptor = target;
        }
        if descriptor.is_multiple_of(4)
            && let Some(span) = bounds(data, descriptor)
        {
            entries.push(NativeEntry {
                call,
                mode,
                table_index: None,
                subtable_index: None,
                literal,
                descriptor,
                span,
            });
        }
    }
    entries
}

fn entrypoint_reachability(
    entries: &[NativeEntry],
    candidates: &HashSet<usize>,
    adjacency: &BTreeMap<usize, Vec<usize>>,
) -> Result<(String, usize)> {
    let roots: std::collections::BTreeSet<_> = entries
        .iter()
        .map(|entry| entry.descriptor)
        .filter(|descriptor| candidates.contains(descriptor))
        .collect();
    let mut listing = String::from("entry_descriptor\ttarget_descriptor\tmin_edges\n");
    let mut reached = HashSet::new();
    for root in roots {
        let mut visited = HashSet::from([root]);
        let mut queue = VecDeque::from([(root, 0)]);
        while let Some((descriptor, distance)) = queue.pop_front() {
            writeln!(listing, "{root:06x}\t{descriptor:06x}\t{distance}")?;
            reached.insert(descriptor);
            for &target in adjacency.get(&descriptor).into_iter().flatten() {
                if visited.insert(target) {
                    queue.push_back((target, distance + 1));
                }
            }
        }
    }
    Ok((listing, reached.len()))
}

fn word(data: &[u8], at: usize) -> usize {
    u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize
}

fn digest(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn hex(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 2);
    for b in data {
        use std::fmt::Write;
        write!(out, "{b:02x}").unwrap();
    }
    out
}

fn quote(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            ch if ch <= '\x1f' => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

pub(crate) fn glyph_table(pp6: bool) -> Vec<Option<char>> {
    let mut chars = vec![None; 0x1100];
    for line in include_str!("glyphs.txt").lines() {
        let (index, ch) = line.split_once('\t').unwrap();
        chars[index.parse::<usize>().unwrap()] = ch.chars().next();
    }
    if pp6 {
        // Python PP6's canonical font names replace convenience ASCII aliases.
        for ch in "＋？！＠／（）％～￥".chars() {
            if let Some(index) = chars.iter().position(|item| {
                *item
                    == Some(match ch {
                        '＋' => '+',
                        '？' => '?',
                        '！' => '!',
                        '＠' => '@',
                        '／' => '/',
                        '（' => '(',
                        '）' => ')',
                        '％' => '%',
                        '～' => '~',
                        '￥' => '¥',
                        _ => unreachable!(),
                    })
            }) {
                chars[index] = Some(ch);
            }
        }
    } else {
        for (start, alphabet) in [
            (0x1051, "0123456789"),
            (0x105b, "ABCDEFGHIJKLMNOPQRSTUVWXYZ"),
            (0x107e, "abcdefghijklmnopqrstuvwxyz"),
        ] {
            for (i, ch) in alphabet.chars().enumerate() {
                chars[start + i] = Some(ch);
            }
        }
        for (index, ch) in [
            (0x1005, 'ｶ'),
            (0x107d, '/'),
            (0x0d02, '屁'),
            (0x0d44, '璧'),
            (0x0d54, '祟'),
        ] {
            chars[index] = Some(ch);
        }
    }
    chars
}

fn decode_pp4(raw: &[u8], table: &[Option<char>]) -> String {
    let mut result = String::new();
    let mut i = 0;
    while i < raw.len() {
        let c = raw[i];
        i += 1;
        if c == 0 {
            result.push('　');
            continue;
        }
        let index = if (0xe8..=0xf7).contains(&c) && i < raw.len() {
            let index = (c as usize - 0xe7) * 256 + raw[i] as usize;
            i += 1;
            index
        } else {
            c as usize - 1
        };
        if let Some(ch) = table.get(index).copied().flatten() {
            result.push(ch);
        } else {
            result.push_str(&format!("〈glyph:{index:03X}〉"));
        }
    }
    result
}

fn write_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    #[test]
    fn whole_rom_scan_recovers_pp5_events_without_fixed_ranges() {
        for id in ["pp5_v0", "pp5_v0_p"] {
            let Some(path) = crate::test_assets::rom(id) else {
                continue;
            };
            let (profile, rom) = crate::rom::verify_supported_rom(&path).unwrap();
            let expected_listing = profile.script_listing.unwrap();
            let discovered = discover_pp5_listing(&rom).unwrap();
            assert_eq!(discovered.dispatch, expected_listing.dispatch);
            assert_eq!(discovered.effect_dispatch, expected_listing.effect_dispatch);
            assert_eq!(discovered.event_entry, expected_listing.event_entry);
            let known = event_candidates(&rom, profile.script_scan);
            let found = referenced_candidates(&rom);
            let overlap = found
                .iter()
                .filter(|(address, range)| known.get(address) == Some(*range))
                .count();
            assert!(overlap >= 2300, "{id}: {overlap}");
            assert!(
                found.len() - overlap < 10,
                "{id}: {} outside known scan",
                found.len() - overlap
            );
            let out = tempfile::tempdir().unwrap();
            assert_eq!(
                export_scanned_events(&rom, out.path(), None).unwrap(),
                found.len()
            );
            let (&desc, &(start, end)) = found.first_key_value().unwrap();
            let binary = out.path().join(format!("native/events/{desc:06x}.bin"));
            assert_eq!(std::fs::read(binary).unwrap(), &rom[start..end]);
            let text_out = tempfile::tempdir().unwrap();
            export_scanned_events(&rom, text_out.path(), Some(discovered)).unwrap();
            let tsv = text_out
                .path()
                .join(format!("native/events/{desc:06x}.tsv"));
            if let Ok(Some(expected)) = pp4::native_listing(
                &rom,
                desc,
                start,
                end,
                &glyph_table(false),
                expected_listing,
            ) {
                assert_eq!(std::fs::read_to_string(tsv).unwrap(), expected.text);
            }
            let manifest: serde_json::Value = serde_json::from_slice(
                &std::fs::read(text_out.path().join("native/manifest.json")).unwrap(),
            )
            .unwrap();
            assert!(manifest["decoded_listings"].as_u64().unwrap() >= 2300);
        }
    }
}
