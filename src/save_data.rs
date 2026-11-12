use std::fmt::Write as _;
use std::io::{self, BufReader, Read};
mod clean;
mod garage_trial;
mod vehicle_trial;
pub use vehicle_trial::{
    citizen_categories, compare_creatures, count_entities, job_market, path_demand, moving_away_report, inspect_people_trial, inspect_vehicle_trial, probe_components,
    rekey_manifest, save_sizes, write_people_trial, write_vehicle_trial,
};
mod position;
mod stride;
pub use clean::{CleanupPlan, clean_save, preview_cleanup};
pub use garage_trial::{inspect_garage_trial, write_garage_trial};
pub use position::{
    DestinationReport, PositionGrid, PositionRegion, PositionReport, inspect_destinations,
    inspect_positions, write_position_trial, write_unspawned_trial,
};

#[cfg(test)]
mod remap_tests;

const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];
const MAX_HEADER: usize = 64 * 1024;
const MAX_TABLE: usize = 16 * 1024 * 1024;
const MAX_FRAME: usize = 256 * 1024 * 1024;
const EXAMPLE_LIMIT: usize = 8;
const HOUSEHOLD_KIND: u8 = 1 << 4;

const KINDS: [(&str, &str); 4] = [
    ("Game.Vehicles.Vehicle", "車両"),
    ("Game.Vehicles.ParkedCar", "駐車車両"),
    ("Game.Creatures.Human", "歩行者の実体"),
    ("Game.Citizens.Citizen", "市民データ"),
];

const REFERENCE_FIELDS: [(&str, usize); 3] = [
    ("Game.Creatures.CurrentVehicle", 8),
    ("Game.Citizens.CurrentTransport", 4),
    ("Game.Common.Owner", 4),
];

struct Component {
    name: String,
    flag: u8,
    kind: u8,
    reference: Option<usize>,
    optional_link: bool,
}

struct Archetype {
    start: u64,
    count: u32,
    kinds: u8,
    types: Vec<usize>,
}

#[derive(Default)]
struct TargetStats {
    count: u64,
    examples: Vec<(u64, u32)>,
}

#[derive(Default)]
struct ReferenceStats {
    vehicles: TargetStats,
    humans: TargetStats,
    other: u64,
    null: u64,
    invalid: u64,
}

pub struct Report {
    pub counts: [u64; 4],
    references: [ReferenceStats; 3],
}

impl Report {
    pub fn render(&self) -> String {
        let mut out = String::from("SaveGameData の車両・歩行者集計\r\n\r\n");
        for ((name, label), count) in KINDS.iter().zip(self.counts) {
            let _ = write!(out, "{label}: {count}  ({})\r\n", short_name(name));
        }
        out.push_str("\r\n種類ごとに集計しています。駐車車両は車両と重複します。\r\n");
        out.push_str("Human は乗車中・未出現の人物の実体も含みます。\r\n");
        for ((name, _), stats) in REFERENCE_FIELDS.iter().zip(&self.references) {
            let _ = write!(
                out,
                "\r\n{} の参照\r\n  車両: {}  人物の実体: {}  その他: {}  空: {}  範囲外: {}\r\n",
                short_name(name),
                stats.vehicles.count,
                stats.humans.count,
                stats.other,
                stats.null,
                stats.invalid
            );
            for (label, target) in [("車両", &stats.vehicles), ("人物の実体", &stats.humans)]
            {
                if !target.examples.is_empty() {
                    let _ = write!(out, "  {label}への参照例（元ID → 先ID）\r\n");
                    for (source, id) in &target.examples {
                        let _ = write!(out, "    {source} → {id}\r\n");
                    }
                }
            }
        }
        out.push_str("\r\n保存IDはこのセーブ内の番号です。集計対象は上記3種類の参照です。");
        out
    }
}

fn short_name(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

pub fn analyze(reader: impl Read) -> Result<Report, String> {
    let mut reader = BufReader::new(reader);
    let header = find_first_frame(&mut reader)?;
    let types = parse_types(&read_frame(&mut reader, header, MAX_TABLE)?)?;
    validate_systems(&read_next_frame(&mut reader, MAX_TABLE)?)?;
    let archetypes = parse_archetypes(&read_next_frame(&mut reader, MAX_TABLE)?, &types)?;
    let mut report = Report {
        counts: [0; 4],
        references: Default::default(),
    };

    for (index, row) in archetypes.iter().enumerate() {
        for (kind, count) in report.counts.iter_mut().enumerate() {
            if row.kinds & (1 << kind) != 0 {
                *count += u64::from(row.count);
            }
        }
        // 参照を持たないアーキタイプもフレームの長さと形式を確認して読み飛ばす。
        let result = if row.types.iter().any(|&id| types[id].reference.is_some()) {
            read_next_frame(&mut reader, MAX_FRAME).and_then(|frame| {
                read_references(&frame, row, &types, &archetypes, &mut report.references)
            })
        } else {
            skip_frame(&mut reader)
        };
        result.map_err(|e| format!("アーキタイプ {index}: {e}"))?;
    }
    Ok(report)
}

// セーブによってヘッダーの長さが異なる。Zstdの直前にある2つのサイズから最初の表を読む。
fn find_first_frame(reader: &mut impl Read) -> Result<FrameHeader, String> {
    let mut window = read_array::<12>(reader)?;
    for _ in 0..MAX_HEADER {
        if window[8..] == ZSTD_MAGIC {
            let header = FrameHeader::from_bytes(window[..8].try_into().unwrap());
            if header.raw > 0 && header.raw <= MAX_TABLE && (4..=MAX_TABLE).contains(&header.packed)
            {
                return Ok(header);
            }
        }
        window.rotate_left(1);
        window[11] = read_array::<1>(reader)?[0];
    }
    Err("対応する Zstandard 型テーブルが見つかりません".into())
}

struct FrameHeader {
    raw: usize,
    packed: usize,
}

impl FrameHeader {
    fn from_bytes(bytes: [u8; 8]) -> Self {
        Self {
            raw: u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize,
            packed: u32::from_le_bytes(bytes[4..].try_into().unwrap()) as usize,
        }
    }

    fn read(reader: &mut impl Read) -> Result<Self, String> {
        Ok(Self::from_bytes(read_array(reader)?))
    }

    fn validate(&self, limit: usize) -> Result<(), String> {
        if self.raw == 0 && self.packed == 0 {
            return Ok(());
        }
        if self.raw == 0 || self.raw > limit || !(4..=limit).contains(&self.packed) {
            return Err(format!(
                "未対応のバッファサイズ ({}, {})",
                self.raw, self.packed
            ));
        }
        Ok(())
    }
}

fn read_next_frame(reader: &mut impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let header = FrameHeader::read(reader)?;
    if header.packed == 0 && header.raw == 0 {
        return Ok(Vec::new());
    }
    if read_array::<4>(reader)? != ZSTD_MAGIC {
        return Err("未対応の内部圧縮形式です".into());
    }
    read_frame(reader, header, limit)
}

fn read_frame(
    reader: &mut impl Read,
    header: FrameHeader,
    limit: usize,
) -> Result<Vec<u8>, String> {
    header.validate(limit)?;
    let mut bytes = vec![0; header.packed];
    bytes[..4].copy_from_slice(&ZSTD_MAGIC);
    reader
        .read_exact(&mut bytes[4..])
        .map_err(|e| e.to_string())?;
    let data = zstd::bulk::decompress(&bytes, header.raw).map_err(|e| e.to_string())?;
    if data.len() != header.raw {
        return Err("展開サイズがヘッダーと一致しません".into());
    }
    Ok(data)
}

fn skip_frame(reader: &mut impl Read) -> Result<(), String> {
    let header = FrameHeader::read(reader)?;
    header.validate(MAX_FRAME)?;
    if header.packed == 0 {
        return Ok(());
    }
    if read_array::<4>(reader)? != ZSTD_MAGIC {
        return Err("未対応の内部圧縮形式です".into());
    }
    let remaining = (header.packed - 4) as u64;
    let skipped =
        io::copy(&mut reader.take(remaining), &mut io::sink()).map_err(|e| e.to_string())?;
    if skipped != remaining {
        return Err("圧縮バッファが途中で終わっています".into());
    }
    Ok(())
}

fn read_array<const N: usize>(reader: &mut impl Read) -> Result<[u8; N], String> {
    let mut bytes = [0; N];
    reader.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn bytes<'a>(data: &mut &'a [u8], size: usize) -> Result<&'a [u8], String> {
    let (head, rest) = data
        .split_at_checked(size)
        .ok_or("レコードが途中で終わっています")?;
    *data = rest;
    Ok(head)
}

fn uint(data: &mut &[u8]) -> Result<u32, String> {
    Ok(u32::from_le_bytes(read_array(data)?))
}

fn sized_bytes<'a>(data: &mut &'a [u8]) -> Result<&'a [u8], String> {
    let size = uint(data)? as usize;
    bytes(data, size)
}

fn require_end(data: &[u8]) -> Result<(), String> {
    if data.is_empty() {
        Ok(())
    } else {
        Err("レコード末尾に未解釈のデータがあります".into())
    }
}

fn parse_types(mut data: &[u8]) -> Result<Vec<Component>, String> {
    let count = uint(&mut data)?;
    let mut components = Vec::new();
    let mut known_kinds = 0;
    for _ in 0..count {
        let mut record = sized_bytes(&mut data)?;
        let flag = bytes(&mut record, 1)?[0];
        let name = std::str::from_utf8(sized_bytes(&mut record)?).map_err(|e| e.to_string())?;
        let name = name
            .split_once(',')
            .ok_or("型のアセンブリ名がありません")?
            .0;
        require_end(record)?;
        let kind = KINDS
            .iter()
            .position(|(n, _)| *n == name)
            .map_or(0, |i| 1 << i)
            | if name == "Game.Citizens.Household" {
                HOUSEHOLD_KIND
            } else {
                0
            };
        known_kinds |= kind;
        let reference = REFERENCE_FIELDS.iter().position(|(n, _)| *n == name);
        if reference.is_some() && flag != 2 {
            return Err(format!("未対応の参照型です ({name}, フラグ {flag})"));
        }
        let optional_link = matches!(
            name,
            "Game.Citizens.CurrentTransport" | "Game.Creatures.GroupMember"
        );
        components.push(Component {
            name: name.to_owned(),
            flag,
            kind,
            reference,
            optional_link,
        });
    }
    require_end(data)?;
    if known_kinds == 0 {
        return Err("対応する SaveGameData 型テーブルではありません".into());
    }
    Ok(components)
}

fn validate_systems(mut data: &[u8]) -> Result<(), String> {
    let count = uint(&mut data)?;
    for _ in 0..count {
        let mut record = sized_bytes(&mut data)?;
        std::str::from_utf8(sized_bytes(&mut record)?).map_err(|e| e.to_string())?;
        require_end(record)?;
    }
    require_end(data)
}

fn parse_archetypes(mut data: &[u8], types: &[Component]) -> Result<Vec<Archetype>, String> {
    let count = uint(&mut data)?;
    let mut rows = Vec::new();
    let mut start = 0;
    for _ in 0..count {
        let mut record = sized_bytes(&mut data)?;
        let count = uint(&mut record)?;
        let type_count = uint(&mut record)?;
        let mut ids = Vec::new();
        let mut kinds = 0;
        for _ in 0..type_count {
            let id = uint(&mut record)? as usize;
            kinds |= types.get(id).ok_or("コンポーネント番号が範囲外です")?.kind;
            ids.push(id);
        }
        require_end(record)?;
        rows.push(Archetype {
            start,
            count,
            kinds,
            types: ids,
        });
        start += u64::from(count);
    }
    require_end(data)?;
    Ok(rows)
}

fn read_references(
    mut data: &[u8],
    row: &Archetype,
    types: &[Component],
    archetypes: &[Archetype],
    references: &mut [ReferenceStats; 3],
) -> Result<(), String> {
    for &id in &row.types {
        let component = &types[id];
        // フラグ1は型一覧に載るが、アーキタイプにペイロードを持たない。
        if component.flag == 1 {
            continue;
        }
        let block = sized_bytes(&mut data)?;
        if let Some(index) = component.reference {
            let (name, stride) = REFERENCE_FIELDS[index];
            if block.len() % stride != 0 || block.len() / stride != row.count as usize {
                return Err(format!("{name} の参照レコードが想定サイズと一致しません"));
            }
            for (entity, value) in block.chunks_exact(stride).enumerate() {
                let target = u32::from_le_bytes(value[..4].try_into().unwrap());
                references[index].record(row.start + entity as u64, target, archetypes);
            }
        }
    }
    require_end(data)
}

impl ReferenceStats {
    fn record(&mut self, source: u64, target: u32, rows: &[Archetype]) {
        if target == u32::MAX {
            self.null += 1;
            return;
        }
        let id = u64::from(target);
        // 保存IDはアーキタイプ順に並ぶ。開始IDの重複は0件の行で起こりうる。
        let pos = rows.partition_point(|row| row.start <= id);
        let Some(row) = pos
            .checked_sub(1)
            .and_then(|i| rows.get(i))
            .filter(|row| id < row.start + u64::from(row.count))
        else {
            self.invalid += 1;
            return;
        };
        let stats = if row.kinds & 1 != 0 {
            &mut self.vehicles
        } else if row.kinds & (1 << 2) != 0 {
            &mut self.humans
        } else {
            self.other += 1;
            return;
        };
        stats.count += 1;
        if stats.examples.len() < EXAMPLE_LIMIT {
            stats.examples.push((source, target));
        }
    }
}
