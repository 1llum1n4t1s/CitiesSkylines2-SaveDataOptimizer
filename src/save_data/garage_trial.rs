use super::clean::{put_frame, write_save_copy};
use super::position::read_save_data;
use super::stride::StrideCodec;
use super::*;
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Seek};
use std::ops::Range;
use std::path::Path;

#[derive(Clone, Debug)]
struct Garage {
    id: u32,
    capacity: u16,
    reported_count: u16,
    center: [f32; 3],
}

#[derive(Clone)]
struct Car {
    id: u32,
    seed: u16,
    keeper: u32,
    lane: u32,
    hidden: bool,
}

struct Scan {
    garages: Vec<Garage>,
    cars: Vec<Car>,
    citizen_ids: Vec<Range<u64>>,
}

fn u16_at(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap())
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

type SaveLayout<'a> = (BufReader<Cursor<&'a [u8]>>, Vec<Component>, Vec<Archetype>);

fn layout(data: &[u8]) -> Result<SaveLayout<'_>, String> {
    let mut reader = BufReader::new(Cursor::new(data));
    let header = find_first_frame(&mut reader)?;
    let types = parse_types(&read_frame(&mut reader, header, MAX_TABLE)?)?;
    validate_systems(&read_next_frame(&mut reader, MAX_TABLE)?)?;
    let rows = parse_archetypes(&read_next_frame(&mut reader, MAX_TABLE)?, &types)?;
    Ok((reader, types, rows))
}

fn blocks(
    frame: &[u8],
    types: &[Component],
    row: &Archetype,
) -> Result<HashMap<usize, Range<usize>>, String> {
    let mut cursor = frame;
    let mut ranges = HashMap::new();
    for &id in &row.types {
        if types[id].flag != 1 {
            let start = frame.len() - cursor.len() + 4;
            let payload = sized_bytes(&mut cursor)?;
            ranges.insert(id, start..start + payload.len());
        }
    }
    require_end(cursor)?;
    Ok(ranges)
}

fn type_id(types: &[Component], name: &str) -> Result<usize, String> {
    types
        .iter()
        .position(|ty| ty.name == name)
        .ok_or_else(|| format!("{name} がありません"))
}

fn scan(data: &[u8], codec: &StrideCodec) -> Result<Scan, String> {
    let (mut reader, types, rows) = layout(data)?;
    let [garage, curve, parked, personal, seed, hidden] = [
        "Game.Net.GarageLane",
        "Game.Net.Curve",
        "Game.Vehicles.ParkedCar",
        "Game.Vehicles.PersonalCar",
        "Game.Common.PseudoRandomSeed",
        "Game.Objects.Unspawned",
    ]
    .map(|name| type_id(&types, name));
    let (garage, curve, parked, personal, seed, hidden) =
        (garage?, curve?, parked?, personal?, seed?, hidden?);
    let mut scan = Scan {
        garages: Vec::new(),
        cars: Vec::new(),
        citizen_ids: Vec::new(),
    };
    for row in rows {
        if row.kinds & 8 != 0 {
            scan.citizen_ids
                .push(row.start..row.start + u64::from(row.count));
        }
        let is_garage = row.types.contains(&garage) && row.types.contains(&curve);
        let is_car = row.types.contains(&personal) && row.types.contains(&seed);
        if !is_garage && !is_car {
            skip_frame(&mut reader)?;
            continue;
        }
        let frame = read_next_frame(&mut reader, MAX_FRAME)?;
        let ranges = blocks(&frame, &types, &row)?;
        let count = row.count as usize;
        if is_garage {
            let payload = &frame[ranges[&garage].clone()];
            let encoded = &frame[ranges[&curve].clone()];
            if payload.len() != count * 8 || encoded.len() != count * 48 {
                return Err("車庫のレコード長が一致しません".into());
            }
            let decoded;
            let geometry = match types[curve].flag {
                2 => encoded,
                34 => {
                    decoded = codec.transform(encoded, 12, true)?;
                    &decoded
                }
                _ => return Err("車庫レーンの形状形式が未対応です".into()),
            };
            for i in 0..count {
                let center =
                    std::array::from_fn(|axis| f32::from_bits(u32_at(geometry, i * 48 + axis * 4)));
                scan.garages.push(Garage {
                    id: row.start as u32 + i as u32,
                    capacity: u16_at(payload, i * 8 + 6),
                    reported_count: u16_at(payload, i * 8 + 4),
                    center,
                });
            }
        }
        if is_car {
            let cars = &frame[ranges[&personal].clone()];
            let seeds = &frame[ranges[&seed].clone()];
            let parking = ranges.get(&parked).map(|range| &frame[range.clone()]);
            if cars.len() != count * 8
                || seeds.len() != count * 2
                || parking.is_some_and(|p| p.len() != count * 8)
            {
                return Err(format!("自家用車 {} のレコード長が一致しません", row.start));
            }
            for i in 0..count {
                scan.cars.push(Car {
                    id: row.start as u32 + i as u32,
                    seed: u16_at(seeds, i * 2),
                    keeper: u32_at(cars, i * 8),
                    lane: parking.map_or(u32::MAX, |p| u32_at(p, i * 8)),
                    hidden: row.types.contains(&hidden),
                });
            }
        }
    }
    Ok(scan)
}

pub fn inspect_garage_trial(
    input: &Path,
    plugin: &Path,
    center: [f32; 3],
    seeds: &[u16],
) -> Result<String, String> {
    let scan = scan(&read_save_data(input)?, &StrideCodec::open(plugin)?)?;
    let distance = |g: &Garage| {
        g.center
            .iter()
            .zip(center)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f32>()
    };
    let garage = scan
        .garages
        .iter()
        .min_by(|a, b| distance(a).total_cmp(&distance(b)))
        .filter(|garage| distance(garage) < 1.0);
    let mut result = match garage {
        Some(garage) => {
            let present = scan.cars.iter().filter(|car| car.lane == garage.id).count();
            format!(
                "車庫 {} 定員={} ゲームの集計={} 参照上の自家用車={present}\n",
                garage.id, garage.capacity, garage.reported_count
            )
        }
        None => "同じ位置に試験対象の車庫がありません\n".to_owned(),
    };
    for seed in seeds {
        let matches: Vec<_> = scan.cars.iter().filter(|car| car.seed == *seed).collect();
        writeln!(
            result,
            "seed={seed} {}",
            matches
                .iter()
                .map(|car| {
                    format!(
                        "id={} keeper={} keeper_is_citizen={} lane={} in_test_garage={} hidden={}",
                        car.id,
                        car.keeper,
                        scan.citizen_ids
                            .iter()
                            .any(|ids| ids.contains(&u64::from(car.keeper))),
                        car.lane,
                        garage.is_some_and(|garage| car.lane == garage.id),
                        car.hidden
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        )
        .unwrap();
    }
    Ok(result)
}

pub fn write_garage_trial(
    input: &Path,
    output: &Path,
    plugin: &Path,
    ids: [&str; 4],
) -> Result<String, String> {
    let mut summary = String::new();
    write_save_copy(input, output, ids, |data| {
        let scan = scan(data, &StrideCodec::open(plugin)?)?;
        let garage = scan
            .garages
            .iter()
            .filter(|g| g.capacity > 0 && g.capacity <= 15)
            .filter(|g| scan.cars.iter().any(|c| c.lane == g.id && c.hidden))
            .min_by_key(|g| g.capacity.abs_diff(10))
            .ok_or("定員が10台前後の使用中の車庫がありません")?;
        let resident = scan
            .cars
            .iter()
            .find(|c| c.lane == garage.id && c.hidden)
            .unwrap();
        let current = scan.cars.iter().filter(|c| c.lane == garage.id).count();
        let garage_ids: HashSet<_> = scan.garages.iter().map(|g| g.id).collect();
        let mut frequencies = HashMap::<u16, usize>::new();
        for car in &scan.cars {
            *frequencies.entry(car.seed).or_default() += 1;
        }
        let add = (usize::from(garage.capacity) + 5)
            .saturating_sub(current)
            .max(5);
        let chosen: Vec<_> = scan
            .cars
            .iter()
            .filter(|car| {
                car.hidden
                    && car.lane != garage.id
                    && garage_ids.contains(&car.lane)
                    && car.keeper != u32::MAX
                    && frequencies[&car.seed] == 1
            })
            .take(add)
            .collect();
        if chosen.len() != add {
            return Err("持ち主付きの追跡可能な駐車車両が足りません".into());
        }
        let selected: HashSet<_> = chosen.iter().map(|car| car.id).collect();
        let (mut reader, types, rows) = layout(data)?;
        let parked = type_id(&types, "Game.Vehicles.ParkedCar")?;
        let transform = type_id(&types, "Game.Objects.Transform")?;
        let mut template = None;
        for row in &rows {
            if !(row.start..row.start + u64::from(row.count)).contains(&u64::from(resident.id)) {
                skip_frame(&mut reader)?;
                continue;
            }
            let frame = read_next_frame(&mut reader, MAX_FRAME)?;
            let ranges = blocks(&frame, &types, row)?;
            let index = resident.id as usize - row.start as usize;
            let at = ranges[&parked].start + 8 * index;
            let position = ranges[&transform].start + 28 * index;
            template = Some((
                frame[at..at + 8].to_vec(),
                frame[position..position + 28].to_vec(),
            ));
            break;
        }
        let (parking, pose) = template.ok_or("車庫内の基準車を取得できません")?;
        let (mut reader, _, _) = layout(data)?;
        let first = reader.stream_position().map_err(|e| e.to_string())? as usize;
        let mut output = data[..first].to_vec();
        for row in &rows {
            let start = reader.stream_position().map_err(|e| e.to_string())? as usize;
            let start_id = row.start as u32;
            let changed: Vec<_> = selected
                .iter()
                .filter(|id| (start_id..start_id + row.count).contains(id))
                .map(|id| (*id - start_id) as usize)
                .collect();
            if changed.is_empty() {
                skip_frame(&mut reader)?;
                let end = reader.stream_position().map_err(|e| e.to_string())? as usize;
                output.extend_from_slice(&data[start..end]);
                continue;
            }
            let mut frame = read_next_frame(&mut reader, MAX_FRAME)?;
            let ranges = blocks(&frame, &types, row)?;
            if types[transform].flag != 2 || ranges[&transform].len() != row.count as usize * 28 {
                return Err("車両の姿勢データの形式が一致しません".into());
            }
            for i in changed {
                // 屋内の駐車参照と姿勢だけを移す。車、持ち主、保存IDをそのまま残す。
                let at = ranges[&parked].start + 8 * i;
                frame[at..at + 8].copy_from_slice(&parking);
                let at = ranges[&transform].start + 28 * i;
                frame[at..at + 28].copy_from_slice(&pose);
            }
            put_frame(&mut output, &frame)?;
        }
        let end = reader.stream_position().map_err(|e| e.to_string())? as usize;
        output.extend_from_slice(&data[end..]);
        summary = format!(
            "{garage:?} before={current} after={} seeds={:?}",
            current + chosen.len(),
            chosen.iter().map(|c| c.seed).collect::<Vec<_>>()
        );
        Ok(output)
    })?;
    Ok(summary)
}
