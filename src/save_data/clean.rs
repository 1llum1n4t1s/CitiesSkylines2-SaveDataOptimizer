use super::*;
use crate::capture::{CaptureReader, CapturedReference, CapturedStride, FrameKey, read_strides};
use crate::remap::EntityRemap;
use crate::vehicle_selection::{LegacyTransportKind, dummy_traffic_candidates};
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Cursor, Seek, Write};
use std::ops::Range;
use std::path::Path;
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

use super::stride::StrideCodec;

struct Trace {
    refs: HashMap<FrameKey, Vec<CapturedReference>>,
    strides: HashMap<FrameKey, Vec<CapturedStride>>,
    aliases: HashMap<FrameKey, Vec<FrameKey>>,
    visited: HashSet<FrameKey>,
}

impl Trace {
    fn open(references: &Path, strides: &Path) -> Result<Self, String> {
        let file = File::open(references).map_err(|e| e.to_string())?;
        let mut capture = CaptureReader::new(BufReader::new(file)).map_err(|e| e.to_string())?;
        let mut refs: HashMap<_, Vec<_>> = HashMap::new();
        while let Some(item) = capture.next_reference().map_err(|e| e.to_string())? {
            refs.entry(item.frame).or_default().push(item);
        }
        if refs.is_empty() {
            return Err("参照記録が空です".into());
        }
        for items in refs.values_mut() {
            items.sort_unstable_by_key(|item| (item.offset, item.id));
            items.dedup();
            if items
                .windows(2)
                .any(|pair| pair[0].offset == pair[1].offset)
            {
                return Err("同一バッファ位置の参照記録が矛盾しています".into());
            }
        }
        let file = File::open(strides).map_err(|e| e.to_string())?;
        let mut stride_map: HashMap<_, Vec<_>> = HashMap::new();
        let mut aliases: HashMap<_, Vec<_>> = HashMap::new();
        for item in read_strides(BufReader::new(file)).map_err(|e| e.to_string())? {
            if item.parent != item.parent_after {
                aliases
                    .entry(item.parent)
                    .or_default()
                    .push(item.parent_after);
                aliases
                    .entry(item.parent_after)
                    .or_default()
                    .push(item.parent);
            }
            stride_map.entry(item.parent).or_default().push(item);
        }
        // 地形など、参照を持たない圧縮ブロックは元のバイト列を保持する。
        // 同じサイズのゼロ領域が多いとサンプル指紋も一致するため、参照を含む経路だけ展開する。
        let mut needed: HashSet<_> = refs.keys().copied().collect();
        loop {
            let before = needed.len();
            for (&key, neighbors) in &aliases {
                if neighbors.iter().any(|neighbor| needed.contains(neighbor)) {
                    needed.insert(key);
                }
            }
            for (&parent, items) in &stride_map {
                if items.iter().any(|item| needed.contains(&item.child)) {
                    needed.insert(parent);
                }
            }
            if needed.len() == before {
                break;
            }
        }
        stride_map.retain(|_, items| {
            items.retain(|item| needed.contains(&item.child));
            !items.is_empty()
        });
        for items in stride_map.values_mut() {
            items.sort_unstable_by_key(|item| item.offset);
            items.dedup();
            if items
                .windows(2)
                .any(|pair| pair[0].offset + pair[0].child.size > pair[1].offset)
            {
                return Err("ストライド記録の範囲が重複しています".into());
            }
        }
        Ok(Self {
            refs,
            strides: stride_map,
            aliases,
            visited: HashSet::new(),
        })
    }

    fn rewrite(
        &mut self,
        input: &[u8],
        remap: &EntityRemap,
        codec: &StrideCodec,
        depth: usize,
    ) -> Result<Vec<u8>, String> {
        if depth > 8 {
            return Err("ストライド記録が循環しています".into());
        }
        let key = FrameKey::for_payload(input).map_err(|e| e.to_string())?;
        let mut aliases = HashSet::new();
        let mut pending = vec![key];
        while let Some(next) = pending.pop() {
            if aliases.insert(next)
                && let Some(neighbors) = self.aliases.get(&next)
            {
                pending.extend(neighbors);
            }
        }
        self.visited.extend(aliases.iter().copied());
        let mut output = input.to_vec();
        let mut strides: Vec<_> = aliases
            .iter()
            .filter_map(|key| self.strides.get(key))
            .flatten()
            .copied()
            .collect();
        strides.sort_unstable_by_key(|item| item.offset);
        strides.dedup_by_key(|item| (item.offset, item.child, item.stride));
        if strides
            .windows(2)
            .any(|pair| pair[0].offset + pair[0].child.size > pair[1].offset)
        {
            return Err("同一フレームのストライド記録が重複しています".into());
        }
        for refs in aliases.iter().filter_map(|key| self.refs.get(key)) {
            for reference in refs {
                let offset = reference.offset as usize;
                let bytes = &mut output[offset..offset + 4];
                if input[offset..offset + 4] != reference.id.to_le_bytes() {
                    return Err(format!(
                        "参照の元バイトが記録と一致しません ({key:?}, {offset})"
                    ));
                }
                if strides.iter().any(|item| {
                    reference.offset < item.offset + item.child.size
                        && reference.offset + 4 > item.offset
                }) {
                    return Err("直接参照とストライド範囲が重複しています".into());
                }
                bytes.copy_from_slice(&remap.resolve(reference.id)?.to_le_bytes());
            }
        }
        for item in strides {
            let range = item.offset as usize..(item.offset + item.child.size) as usize;
            let decoded = codec.transform(&input[range.clone()], item.stride, true)?;
            if FrameKey::for_payload(&decoded).map_err(|e| e.to_string())? != item.child {
                return Err(format!(
                    "ストライド展開が記録と一致しません ({key:?}, {})",
                    item.offset
                ));
            }
            let rewritten = self.rewrite(&decoded, remap, codec, depth + 1)?;
            if rewritten != decoded {
                let encoded = codec.transform(&rewritten, item.stride, false)?;
                if codec.transform(&encoded, item.stride, true)? != rewritten {
                    return Err("ストライド再エンコードの照合に失敗しました".into());
                }
                output[range].copy_from_slice(&encoded);
            }
        }
        Ok(output)
    }

    fn verify_complete(&self) -> Result<(), String> {
        let missing_refs: usize = self
            .refs
            .iter()
            .filter(|(key, _)| !self.visited.contains(key))
            .map(|(_, refs)| refs.len())
            .sum();
        let missing_strides: usize = self
            .strides
            .iter()
            .filter(|(key, _)| !self.visited.contains(key))
            .map(|(_, strides)| strides.len())
            .sum();
        if missing_refs > 0 || missing_strides > 0 {
            return Err(format!(
                "元データと照合できない記録があります (参照 {missing_refs}、ストライド {missing_strides})"
            ));
        }
        Ok(())
    }
}

pub(super) fn put_frame(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), String> {
    let data = zstd::bulk::compress(bytes, 3).map_err(|e| e.to_string())?;
    output.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    output.extend_from_slice(&(data.len() as u32).to_le_bytes());
    output.extend_from_slice(&data);
    Ok(())
}

struct RewrittenArchetype {
    count: u32,
    types: Vec<usize>,
    frame: Vec<u8>,
}

// レコードを元の順に並べたまま、参照が切れた個体だけ別のアーキタイプに分ける。
// この順序を保てば、すでに振り直した保存IDは引き続き同じ個体を指す。
fn strip_empty_links(
    row: &Archetype,
    types: &[Component],
    before: &[u8],
    after: &[u8],
    codec: &StrideCodec,
) -> Result<Vec<RewrittenArchetype>, String> {
    if !row.types.iter().any(|&id| types[id].optional_link) {
        return Ok(Vec::new());
    }
    let mut original = before;
    let mut updated = after;
    let count = row.count as usize;
    let mut blocks = Vec::new();
    let mut drop = vec![Vec::new(); count];
    for &id in &row.types {
        if types[id].flag == 1 {
            blocks.push((id, None));
            continue;
        }
        let old = sized_bytes(&mut original)?;
        let new = sized_bytes(&mut updated)?;
        if types[id].optional_link {
            if new.len() != count * 4 || old.len() != new.len() {
                return Err(format!("{} の参照サイズが不正です", types[id].name));
            }
            for (index, (old, new)) in old.chunks_exact(4).zip(new.chunks_exact(4)).enumerate() {
                if old != [255; 4] && new == [255; 4] {
                    drop[index].push(id);
                }
            }
        }
        blocks.push((id, Some(new)));
    }
    require_end(original)?;
    require_end(updated)?;
    if drop.iter().all(Vec::is_empty) {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    let mut start = 0;
    while start < count {
        let end = start
            + 1
            + drop[start + 1..]
                .iter()
                .take_while(|mask| **mask == drop[start])
                .count();
        let range = start..end;
        let mut part = RewrittenArchetype {
            count: range.len() as u32,
            types: Vec::new(),
            frame: Vec::new(),
        };
        for &(id, data) in &blocks {
            if drop[start].contains(&id) {
                continue;
            }
            part.types.push(id);
            if let Some(data) = data {
                let split;
                let data = if range.len() == count {
                    data
                } else {
                    split = slice_component(&types[id], data, count, range.clone(), codec)
                        .map_err(|error| {
                            format!(
                                "保存ID {} から {} 件の行: {error} ({})",
                                row.start,
                                count,
                                blocks
                                    .iter()
                                    .filter_map(|(id, block)| block.map(|block| format!(
                                        "{}:{}:{}",
                                        types[*id].name,
                                        types[*id].flag,
                                        block.len()
                                    )))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        })?;
                    &split
                };
                part.frame
                    .extend_from_slice(&(data.len() as u32).to_le_bytes());
                part.frame.extend_from_slice(data);
            }
        }
        result.push(part);
        start = end;
    }
    Ok(result)
}

fn slice_component(
    ty: &Component,
    mut data: &[u8],
    count: usize,
    range: Range<usize>,
    codec: &StrideCodec,
) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    match (ty.flag, ty.name.as_str()) {
        (2, "Game.Creatures.Creature") => {
            // 半径が正のときだけ、待ち行列の参照と位置が続く。
            for index in 0..count {
                let start = data;
                let radius = f32::from_le_bytes(bytes(&mut data, 4)?.try_into().unwrap());
                if radius > 0.0 {
                    bytes(&mut data, 16)?;
                }
                if range.contains(&index) {
                    output.extend_from_slice(&start[..start.len() - data.len()]);
                }
            }
        }
        (4, "Game.Objects.TransformFrame" | "Game.Common.RandomLocalizationIndex") => {
            // 両バッファとも要素は4バイト。先頭の個体ごとの長さも一緒に移す。
            for index in 0..count {
                let start = data;
                let elements = uint(&mut data)? as usize;
                bytes(&mut data, elements * 4)?;
                if range.contains(&index) {
                    output.extend_from_slice(&start[..start.len() - data.len()]);
                }
            }
        }
        (20, "Game.Rendering.CustomMeshColor") => {
            // 有効ビットを詰め直し、有効な個体の色バッファだけを移す。
            let enabled = bytes(&mut data, count.div_ceil(8))?;
            output.resize(range.len().div_ceil(8), 0);
            for index in 0..count {
                if enabled[index / 8] & (1 << (index % 8)) == 0 {
                    continue;
                }
                let elements = uint(&mut data)?;
                let colors = bytes(&mut data, elements as usize * 48)?;
                if range.contains(&index) {
                    let bit = index - range.start;
                    output[bit / 8] |= 1 << (bit % 8);
                    output.extend_from_slice(&elements.to_le_bytes());
                    output.extend_from_slice(colors);
                }
            }
        }
        (8, "Game.Simulation.UpdateFrame") => {
            // 共有値は「更新フレーム番号、同じ値の連続件数」の組で保存される。
            let mut start = 0;
            while !data.is_empty() {
                let value = uint(&mut data)?;
                let len = uint(&mut data)? as usize;
                let end = start + len;
                if len == 0 || end > count {
                    return Err("UpdateFrame の件数が不正です".into());
                }
                let overlap = end.min(range.end).saturating_sub(start.max(range.start));
                if overlap > 0 {
                    output.extend_from_slice(&value.to_le_bytes());
                    output.extend_from_slice(&(overlap as u32).to_le_bytes());
                }
                start = end;
            }
            if start != count {
                return Err("UpdateFrame がアーキタイプの件数と一致しません".into());
            }
        }
        (34, "Game.Prefabs.PrefabRef") if data.len() == count * 4 => {
            let decoded = codec.transform(data, 4, true)?;
            let selected = &decoded[range.start * 4..range.end * 4];
            output = codec.transform(selected, 4, false)?;
            if codec.transform(&output, 4, true)? != selected {
                return Err("PrefabRef の再エンコードの照合に失敗しました".into());
            }
            data = &[];
        }
        (2, name) => {
            let stride = match name {
                "Game.Vehicles.Blocker" => 6,
                "Game.Creatures.AnimalCurrentLane" => 36,
                "Game.Creatures.AnimalNavigation" => 31,
                "Game.Objects.Surface" => 5,
                "Game.Objects.Moving" => 24,
                "Game.Objects.Transform" => 28,
                "Game.Common.PseudoRandomSeed" => 2,
                "Game.Creatures.Pet"
                | "Game.Creatures.GroupMember"
                | "Game.Common.Target"
                | "Game.Creatures.Animal" => 4,
                _ => return Err(format!("アーキタイプの分割に未対応の型です ({name})")),
            };
            let block = bytes(&mut data, count * stride)?;
            output.extend_from_slice(&block[range.start * stride..range.end * stride]);
        }
        _ => return Err(format!("アーキタイプの分割に未対応の型です ({})", ty.name)),
    }
    require_end(data)?;
    Ok(output)
}

pub(super) fn put_archetype(output: &mut Vec<u8>, count: u32, types: &[usize]) {
    output.extend_from_slice(&(8 + 4 * types.len() as u32).to_le_bytes());
    output.extend_from_slice(&count.to_le_bytes());
    output.extend_from_slice(&(types.len() as u32).to_le_bytes());
    for &id in types {
        output.extend_from_slice(&(id as u32).to_le_bytes());
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CleanupPlan {
    pub vehicle_ids: Vec<u64>,
    pub pedestrians: u64,
    pub citizens: u64,
    pub households: u64,
    pub parked: u64,
    vehicles: u64,
}

fn plan_rows(
    reader: &mut (impl Read + Seek),
    types: &[Component],
    rows: &[Archetype],
) -> Result<(CleanupPlan, Vec<bool>), String> {
    let mut plan = CleanupPlan::default();
    let mut removed = Vec::new();
    for row in rows {
        let count = u64::from(row.count);
        for (mask, total) in [
            (1, &mut plan.vehicles),
            (2, &mut plan.parked),
            (4, &mut plan.pedestrians),
            (8, &mut plan.citizens),
            (HOUSEHOLD_KIND, &mut plan.households),
        ] {
            if row.kinds & mask != 0 {
                *total += count;
            }
        }
        let frame = read_next_frame(reader, MAX_FRAME)?;
        let vehicle = row.kinds & 1 != 0
            && row.kinds & 2 == 0
            && is_regenerating_vehicle_row(&frame, row, rows, types)?;
        let remove = vehicle || row.kinds & 4 != 0;
        if remove && row.kinds & (8 | HOUSEHOLD_KIND) != 0 {
            return Err("削除候補に市民または世帯のレコードが含まれます".into());
        }
        if vehicle {
            plan.vehicle_ids.extend(row.start..row.start + count);
        }
        removed.push(remove);
    }
    Ok((plan, removed))
}

fn is_regenerating_vehicle_row(
    mut frame: &[u8],
    row: &Archetype,
    rows: &[Archetype],
    types: &[Component],
) -> Result<bool, String> {
    if row.count == 0
        || row.types.iter().any(|&id| {
            matches!(
                types[id].name.as_str(),
                "Game.Objects.Static"
                    | "Game.Routes.CurrentRoute"
                    | "Game.Vehicles.CarTrailer"
                    | "Game.Vehicles.LayoutElement"
            )
        })
    {
        return Ok(false);
    }
    let mut owner = None;
    let mut roles = Vec::new();
    for &id in &row.types {
        if types[id].flag == 1 {
            continue;
        }
        let payload = sized_bytes(&mut frame)?;
        match types[id].name.as_str() {
            "Game.Common.Owner" => owner = Some(payload),
            "Game.Vehicles.PublicTransport" => {
                roles.push((payload, LegacyTransportKind::Passenger))
            }
            "Game.Vehicles.CargoTransport" => roles.push((payload, LegacyTransportKind::Cargo)),
            _ => {}
        }
    }
    let Some(owner) = owner else { return Ok(false) };
    for (transport, kind) in roles {
        // 型が同じでもレイアウトが異なるセーブや、別用途の車両が混在する行は保持する。
        if transport.len() != row.count as usize * 20 || owner.len() != row.count as usize * 4 {
            continue;
        }
        let found = dummy_traffic_candidates(transport, owner, kind, |id| {
            let id = id as u64;
            let index = rows.partition_point(|r| r.start <= id);
            index.checked_sub(1).is_some_and(|i| {
                let r = &rows[i];
                id < r.start + u64::from(r.count)
                    && r.types
                        .iter()
                        .any(|&t| types[t].name == "Game.Buildings.TrafficSpawner")
            })
        })?;
        if found.len() == row.count as usize {
            return Ok(true);
        }
    }
    Ok(false)
}

fn rewrite_save_data(
    data: &[u8],
    trace: &mut Trace,
    codec: &StrideCodec,
    expected: Option<&CleanupPlan>,
) -> Result<Vec<u8>, String> {
    let mut reader = BufReader::new(Cursor::new(data));
    let header = find_first_frame(&mut reader)?;
    let types = parse_types(&read_frame(&mut reader, header, MAX_TABLE)?)?;
    let systems = read_next_frame(&mut reader, MAX_TABLE)?;
    validate_systems(&systems)?;
    let system_count = uint(&mut systems.as_slice())?;
    let archetype_frame = reader.stream_position().map_err(|e| e.to_string())? as usize;
    let archetype_bytes = read_next_frame(&mut reader, MAX_TABLE)?;
    let rows = parse_archetypes(&archetype_bytes, &types)?;
    let first = reader.stream_position().map_err(|e| e.to_string())?;
    let (plan, removed) = plan_rows(&mut reader, &types, &rows)?;
    if expected.is_some_and(|expected| expected != &plan) {
        return Err("確認後に削除候補が変わりました。セーブを開き直してください".into());
    }
    reader
        .seek(io::SeekFrom::Start(first))
        .map_err(|e| e.to_string())?;
    let remap = EntityRemap::from_archetypes(
        rows.iter()
            .enumerate()
            .map(|(i, row)| (row.count, removed[i])),
    )?;
    let mut output = data[..archetype_frame].to_vec();
    let mut kept = 0u32;
    let mut archetypes = vec![0; 4];
    let mut frames = Vec::new();
    let entries = rows
        .iter()
        .zip(removed)
        .map(Some)
        .chain(std::iter::repeat_n(None, system_count as usize));
    for (index, entry) in entries.enumerate() {
        let start = reader.stream_position().map_err(|e| e.to_string())? as usize;
        let frame = read_next_frame(&mut reader, MAX_FRAME)?;
        let end = reader.stream_position().map_err(|e| e.to_string())? as usize;
        let rewritten = trace
            .rewrite(&frame, &remap, codec, 0)
            .map_err(|e| format!("フレーム {index}: {e}"))?;
        if let Some((row, remove)) = entry {
            if remove {
                continue;
            }
            let replacements = strip_empty_links(row, &types, &frame, &rewritten, codec)?;
            if !replacements.is_empty() {
                for part in replacements {
                    put_archetype(&mut archetypes, part.count, &part.types);
                    put_frame(&mut frames, &part.frame)?;
                    kept += 1;
                }
                continue;
            }
            put_archetype(&mut archetypes, row.count, &row.types);
            kept += 1;
        }
        if frame == rewritten {
            frames.extend_from_slice(&data[start..end]);
        } else {
            put_frame(&mut frames, &rewritten)?;
        }
    }
    if reader.stream_position().map_err(|e| e.to_string())? != data.len() as u64 {
        return Err("SaveGameData に未処理のデータがあります".into());
    }
    trace.verify_complete()?;
    archetypes[..4].copy_from_slice(&kept.to_le_bytes());
    put_frame(&mut output, &archetypes)?;
    output.extend_from_slice(&frames);
    let report = analyze(output.as_slice())?;
    if report.counts
        != [
            plan.vehicles - plan.vehicle_ids.len() as u64,
            plan.parked,
            0,
            plan.citizens,
        ]
    {
        return Err(format!(
            "出力の集計が期待値と一致しません ({:?})",
            report.counts
        ));
    }
    if report
        .references
        .iter()
        .any(|stats| stats.invalid != 0 || stats.humans.count != 0)
    {
        return Err("出力の参照集計で範囲外または人物の実体への参照を検出しました".into());
    }
    Ok(output)
}

pub fn preview_cleanup(input: &Path) -> Result<CleanupPlan, String> {
    let mut zip = ZipArchive::new(File::open(input).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let name = zip
        .file_names()
        .find(|name| name.ends_with(".SaveGameData"))
        .ok_or("SaveGameData がありません")?
        .to_owned();
    let mut reader = BufReader::new(Cursor::new(extract(&mut zip, &name)?));
    let header = find_first_frame(&mut reader)?;
    let types = parse_types(&read_frame(&mut reader, header, MAX_TABLE)?)?;
    validate_systems(&read_next_frame(&mut reader, MAX_TABLE)?)?;
    let rows = parse_archetypes(&read_next_frame(&mut reader, MAX_TABLE)?, &types)?;
    Ok(plan_rows(&mut reader, &types, &rows)?.0)
}

fn extract(zip: &mut ZipArchive<File>, name: &str) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    zip.by_name(name)
        .map_err(|e| e.to_string())?
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    Ok(data)
}

pub fn clean_save(
    input: &Path,
    output: &Path,
    references: &Path,
    strides: &Path,
    plugin: &Path,
    ids: [&str; 4],
    expected: Option<&CleanupPlan>,
) -> Result<(), String> {
    write_save_copy(input, output, ids, |raw| {
        let mut trace = Trace::open(references, strides)?;
        let codec = StrideCodec::open(plugin)?;
        rewrite_save_data(raw, &mut trace, &codec, expected)
    })
}

pub(super) fn write_save_copy(
    input: &Path,
    output: &Path,
    ids: [&str; 4],
    rewrite: impl FnOnce(&[u8]) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    if ids
        .iter()
        .any(|id| id.len() != 32 || id.bytes().any(|b| !b.is_ascii_hexdigit()))
        || ids.iter().copied().collect::<HashSet<_>>().len() != 4
    {
        return Err("新規アセットIDが不正です".into());
    }
    let mut sidecar = output.as_os_str().to_os_string();
    sidecar.push(".cid");
    let sidecar = Path::new(&sidecar);
    if output.exists() || sidecar.exists() {
        return Err("出力先がすでに存在します".into());
    }
    let file = File::open(input).map_err(|e| e.to_string())?;
    let mut zip = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let save = zip
        .file_names()
        .find(|name| name.ends_with(".SaveGameData"))
        .ok_or("SaveGameData がありません")?
        .to_owned();
    let stem = save.strip_suffix(".SaveGameData").unwrap();
    let data_id = String::from_utf8(extract(&mut zip, &format!("{stem}.SaveGameData.cid"))?)
        .map_err(|e| e.to_string())?;
    let texture_id = String::from_utf8(extract(&mut zip, &format!("{stem}.Texture.cid"))?)
        .map_err(|e| e.to_string())?;
    let meta = String::from_utf8(extract(&mut zip, &format!("{stem}.SaveGameMetadata"))?)
        .map_err(|e| e.to_string())?;
    if [&data_id, &texture_id]
        .iter()
        .any(|id| id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()))
        || data_id == texture_id
        || meta.matches(&data_id).count() != 1
        || meta.matches(&texture_id).count() != 1
    {
        return Err("セーブのメタデータとCIDが一致しません".into());
    }
    let meta = meta.replace(&data_id, ids[1]).replace(&texture_id, ids[3]);
    let raw = extract(&mut zip, &save)?;
    let texture = extract(&mut zip, &format!("{stem}.Texture"))?;
    let cleaned = rewrite(&raw)?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|e| e.to_string())?;
    let mut out = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (suffix, bytes) in [
        ("SaveGameData", cleaned.as_slice()),
        ("SaveGameData.cid", ids[1].as_bytes()),
        ("SaveGameMetadata", meta.as_bytes()),
        ("SaveGameMetadata.cid", ids[2].as_bytes()),
        ("Texture", texture.as_slice()),
        ("Texture.cid", ids[3].as_bytes()),
    ] {
        out.start_file(format!("{stem}.{suffix}"), options)
            .map_err(|e| e.to_string())?;
        out.write_all(bytes).map_err(|e| e.to_string())?;
    }
    out.finish()
        .map_err(|e| e.to_string())?
        .sync_all()
        .map_err(|e| e.to_string())?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(sidecar)
        .map_err(|e| e.to_string())?
        .write_all(ids[0].as_bytes())
        .map_err(|e| e.to_string())
}
