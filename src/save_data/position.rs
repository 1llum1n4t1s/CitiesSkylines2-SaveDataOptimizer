use super::clean::{put_archetype, put_frame, write_save_copy};
use super::stride::StrideCodec;
use super::*;
use std::collections::HashSet;
use std::fs::File;
use std::io::{Cursor, Seek};
use std::path::Path;
use zip::ZipArchive;

const VEHICLE: u8 = 1;
const PARKED: u8 = 2;
const HUMAN: u8 = 4;
const CITIZEN: u8 = 8;
const TRANSFORM: &str = "Game.Objects.Transform";
const UNSPAWNED: &str = "Game.Objects.Unspawned";

#[derive(Debug, Default)]
pub struct PositionReport {
    pub vehicles: u64,
    pub parked: u64,
    pub humans: u64,
    pub pets: u64,
    pub citizens: u64,
    pub households: u64,
    pub with_coordinates: u64,
    pub without_coordinates: u64,
    pub in_region: Option<RegionCounts>,
    pub unspawned: RegionCounts,
    pub outside_connections: Vec<(u64, [f32; 3])>,
    pub extent: Option<([f32; 3], [f32; 3])>,
}

#[derive(Debug, Default)]
pub struct RegionCounts {
    pub vehicles: u64,
    pub parked: u64,
    pub humans: u64,
    pub pets: u64,
}

pub struct PositionRegion {
    pub min_x: f32,
    pub max_x: f32,
    pub min_z: f32,
    pub max_z: f32,
}

impl PositionRegion {
    fn contains(&self, [x, _, z]: [f32; 3]) -> bool {
        (self.min_x..=self.max_x).contains(&x) && (self.min_z..=self.max_z).contains(&z)
    }

    fn intersects_curve(&self, points: &[[f32; 3]; 4]) -> bool {
        // ベジェ曲線は制御点の範囲内に収まる。候補を見落とさないため外接範囲を使う。
        let bounds = |axis| {
            points
                .iter()
                .map(|point: &[f32; 3]| point[axis])
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), value| {
                    (min.min(value), max.max(value))
                })
        };
        let (min_x, max_x) = bounds(0);
        let (min_z, max_z) = bounds(2);
        max_x >= self.min_x && min_x <= self.max_x && max_z >= self.min_z && min_z <= self.max_z
    }

    fn validate(&self) -> Result<(), String> {
        if [self.min_x, self.max_x, self.min_z, self.max_z]
            .iter()
            .all(|v| v.is_finite())
            && self.min_x <= self.max_x
            && self.min_z <= self.max_z
        {
            return Ok(());
        }
        Err("移動先の範囲が不正です".into())
    }
}

#[derive(Debug, Default)]
pub struct DestinationReport {
    pub car_lanes: u64,
    pub pedestrian_lanes: u64,
    pub parking_lanes: u64,
    pub garage_lanes: u64,
    pub garage_capacity: u64,
    pub parking_facilities: u64,
    pub parked_cars: u64,
    pub parked_on_candidate_lanes: u64,
    pub residents_with_building: u64,
}

pub struct PositionGrid {
    pub origin: [f32; 3],
    pub columns: u32,
    pub spacing: f32,
}

impl PositionGrid {
    fn position(&self, ordinal: u64) -> Result<[f32; 3], String> {
        if self.columns == 0 || !self.spacing.is_finite() || self.spacing < 12.0 {
            return Err("配置には12m以上の間隔と1列以上が必要です".into());
        }
        let columns = u64::from(self.columns);
        let [x, y, z] = self.origin;
        let position = [
            x + (ordinal % columns) as f32 * self.spacing,
            y,
            z + (ordinal / columns) as f32 * self.spacing,
        ];
        if position.iter().any(|v| !v.is_finite()) {
            return Err("配置先の座標が不正です".into());
        }
        Ok(position)
    }
}

fn coordinates(transform: &[u8]) -> [f32; 3] {
    std::array::from_fn(|axis| {
        let offset = axis * 4;
        f32::from_le_bytes(transform[offset..offset + 4].try_into().unwrap())
    })
}

fn visit(
    data: &[u8],
    grid: Option<&PositionGrid>,
    region: Option<&PositionRegion>,
) -> Result<(PositionReport, Vec<u8>), String> {
    let mut reader = BufReader::new(Cursor::new(data));
    let header = find_first_frame(&mut reader)?;
    let types = parse_types(&read_frame(&mut reader, header, MAX_TABLE)?)?;
    let systems = read_next_frame(&mut reader, MAX_TABLE)?;
    validate_systems(&systems)?;
    let rows = parse_archetypes(&read_next_frame(&mut reader, MAX_TABLE)?, &types)?;
    let first_frame = reader.stream_position().map_err(|e| e.to_string())? as usize;
    let mut output = if grid.is_some() {
        data[..first_frame].to_vec()
    } else {
        Vec::new()
    };
    let mut report = PositionReport {
        in_region: region.map(|_| RegionCounts::default()),
        ..Default::default()
    };
    for row in rows {
        let start = reader.stream_position().map_err(|e| e.to_string())? as usize;
        let mut frame = read_next_frame(&mut reader, MAX_FRAME)?;
        let end = reader.stream_position().map_err(|e| e.to_string())? as usize;
        let has = |name| row.types.iter().any(|&id| types[id].name == name);
        let pet = has("Game.Creatures.Pet");
        let outside = has("Game.Objects.OutsideConnection");
        let target = row.kinds & (VEHICLE | PARKED | HUMAN | CITIZEN) != 0 || pet;
        let count = u64::from(row.count);
        if has(UNSPAWNED) {
            report.unspawned.vehicles += count * u64::from(row.kinds & VEHICLE != 0);
            report.unspawned.parked += count * u64::from(row.kinds & PARKED != 0);
            report.unspawned.humans += count * u64::from(row.kinds & HUMAN != 0);
            report.unspawned.pets += count * u64::from(pet);
        }
        for (include, total) in [
            (row.kinds & VEHICLE != 0, &mut report.vehicles),
            (row.kinds & PARKED != 0, &mut report.parked),
            (row.kinds & HUMAN != 0, &mut report.humans),
            (row.kinds & CITIZEN != 0, &mut report.citizens),
            (row.kinds & HOUSEHOLD_KIND != 0, &mut report.households),
            (pet, &mut report.pets),
        ] {
            if include {
                *total += count;
            }
        }
        let mut edited = false;
        if (target || outside) && row.count != 0 {
            let mut blocks = frame.as_slice();
            let mut positions = None;
            for &id in &row.types {
                if types[id].flag == 1 {
                    continue;
                }
                let offset = frame.len() - blocks.len() + 4;
                let block = sized_bytes(&mut blocks)?;
                if types[id].name == TRANSFORM {
                    if types[id].flag != 2 || block.len() != row.count as usize * 28 {
                        return Err(format!(
                            "位置データの形式が未対応です (保存ID {})",
                            row.start
                        ));
                    }
                    positions = Some(offset..offset + block.len());
                }
            }
            if let Some(positions) = positions {
                for (index, transform) in frame[positions].chunks_exact_mut(28).enumerate() {
                    let before = coordinates(transform);
                    if outside {
                        report
                            .outside_connections
                            .push((row.start + index as u64, before));
                    }
                    if target {
                        if let (Some(region), Some(counts)) = (region, &mut report.in_region)
                            && region.contains(before)
                        {
                            counts.vehicles += u64::from(row.kinds & VEHICLE != 0);
                            counts.parked += u64::from(row.kinds & PARKED != 0);
                            counts.humans += u64::from(row.kinds & HUMAN != 0);
                            counts.pets += u64::from(pet);
                        }
                        let (min, max) = report.extent.get_or_insert((before, before));
                        for axis in 0..3 {
                            min[axis] = min[axis].min(before[axis]);
                            max[axis] = max[axis].max(before[axis]);
                        }
                        if let Some(grid) = grid {
                            for (axis, value) in grid
                                .position(report.with_coordinates)?
                                .into_iter()
                                .enumerate()
                            {
                                transform[axis * 4..axis * 4 + 4]
                                    .copy_from_slice(&value.to_le_bytes());
                            }
                            edited = true;
                        }
                        report.with_coordinates += 1;
                    }
                }
            } else if target {
                report.without_coordinates += count;
            }
        }
        if grid.is_some() {
            if edited {
                put_frame(&mut output, &frame)?;
            } else {
                output.extend_from_slice(&data[start..end]);
            }
        }
    }
    if grid.is_some() {
        // 型表・保存ID・システム領域は維持し、位置だけを変えたときのロード挙動を調べる。
        let tail = reader.stream_position().map_err(|e| e.to_string())? as usize;
        output.extend_from_slice(&data[tail..]);
    }
    Ok((report, output))
}

pub(super) fn read_save_data(input: &Path) -> Result<Vec<u8>, String> {
    let file = File::open(input).map_err(|e| e.to_string())?;
    let mut zip = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let name = zip
        .file_names()
        .find(|name| name.ends_with(".SaveGameData"))
        .ok_or("SaveGameData がありません")?
        .to_owned();
    let mut data = Vec::new();
    zip.by_name(&name)
        .map_err(|e| e.to_string())?
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    Ok(data)
}

pub fn inspect_positions(
    input: &Path,
    region: Option<&PositionRegion>,
) -> Result<PositionReport, String> {
    if let Some(region) = region {
        region.validate()?;
    }
    visit(&read_save_data(input)?, None, region).map(|(report, _)| report)
}

pub fn inspect_destinations(
    input: &Path,
    plugin: &Path,
    region: &PositionRegion,
) -> Result<DestinationReport, String> {
    region.validate()?;
    let codec = StrideCodec::open(plugin)?;
    let data = read_save_data(input)?;
    let mut reader = BufReader::new(Cursor::new(data));
    let header = find_first_frame(&mut reader)?;
    let types = parse_types(&read_frame(&mut reader, header, MAX_TABLE)?)?;
    validate_systems(&read_next_frame(&mut reader, MAX_TABLE)?)?;
    let rows = parse_archetypes(&read_next_frame(&mut reader, MAX_TABLE)?, &types)?;
    let mut report = DestinationReport::default();
    let mut candidate_parking_ids = HashSet::new();
    let mut parked_lane_ids = Vec::new();
    for row in rows {
        let has = |name| row.types.iter().any(|&id| types[id].name == name);
        let car = has("Game.Net.CarLane");
        let pedestrian = has("Game.Net.PedestrianLane");
        let parking = has("Game.Net.ParkingLane");
        let garage = has("Game.Net.GarageLane");
        let facility = has("Game.Buildings.CarParkingFacility");
        let parked_car = row.kinds & PARKED != 0;
        if parked_car {
            report.parked_cars += u64::from(row.count);
        }
        if row.kinds & CITIZEN != 0 && has("Game.Citizens.CurrentBuilding") {
            report.residents_with_building += u64::from(row.count);
        }
        if !(car || pedestrian || parking || garage || facility || parked_car) {
            skip_frame(&mut reader)?;
            continue;
        }
        let frame = read_next_frame(&mut reader, MAX_FRAME)?;
        let mut remaining = frame.as_slice();
        let mut garage_data = None;
        let mut matching_garages = Vec::new();
        for &id in &row.types {
            let ty = &types[id];
            if ty.flag == 1 {
                continue;
            }
            let block = sized_bytes(&mut remaining)?;
            if ty.name == "Game.Net.Curve" && (car || pedestrian || parking || garage) {
                if block.len() != row.count as usize * 48 {
                    return Err("車線の形状データの長さが不正です".into());
                }
                let decoded;
                let curves = match ty.flag {
                    2 => block,
                    34 => {
                        decoded = codec.transform(block, 12, true)?;
                        &decoded
                    }
                    _ => return Err("車線の形状データの形式が未対応です".into()),
                };
                for (index, curve) in curves.chunks_exact(48).enumerate() {
                    let points = std::array::from_fn(|i| coordinates(&curve[i * 12..]));
                    if region.intersects_curve(&points) {
                        report.car_lanes += u64::from(car);
                        report.pedestrian_lanes += u64::from(pedestrian);
                        report.parking_lanes += u64::from(parking);
                        report.garage_lanes += u64::from(garage);
                        if parking || garage {
                            candidate_parking_ids.insert(row.start + index as u64);
                        }
                        if garage {
                            matching_garages.push(index);
                        }
                    }
                }
            }
            if ty.name == "Game.Net.GarageLane" {
                if ty.flag != 2 || block.len() != row.count as usize * 8 {
                    return Err("車庫データの形式が未対応です".into());
                }
                garage_data = Some(block);
            }
            if ty.name == "Game.Vehicles.ParkedCar" {
                if ty.flag != 2 || block.len() != row.count as usize * 8 {
                    return Err("駐車中の車の参照形式が未対応です".into());
                }
                parked_lane_ids.extend(
                    block.chunks_exact(8).map(|record| {
                        u64::from(u32::from_le_bytes(record[..4].try_into().unwrap()))
                    }),
                );
            }
            if ty.name == TRANSFORM && facility {
                if ty.flag != 2 || block.len() != row.count as usize * 28 {
                    return Err("駐車施設の位置データの形式が未対応です".into());
                }
                report.parking_facilities += block
                    .chunks_exact(28)
                    .filter(|record| region.contains(coordinates(record)))
                    .count() as u64;
            }
        }
        if let Some(garage_data) = garage_data {
            // 車庫の保存データは料金・快適さ・駐車台数・収容台数の順で各2バイト。
            for index in matching_garages {
                let offset = index * 8 + 6;
                report.garage_capacity += u64::from(u16::from_le_bytes(
                    garage_data[offset..offset + 2].try_into().unwrap(),
                ));
            }
        }
    }
    report.parked_on_candidate_lanes = parked_lane_ids
        .iter()
        .filter(|id| candidate_parking_ids.contains(id))
        .count() as u64;
    Ok(report)
}

pub fn write_position_trial(
    input: &Path,
    output: &Path,
    ids: [&str; 4],
    grid: &PositionGrid,
) -> Result<(), String> {
    write_save_copy(input, output, ids, |raw| {
        visit(raw, Some(grid), None).map(|(_, output)| output)
    })
}

pub fn write_unspawned_trial(input: &Path, output: &Path, ids: [&str; 4]) -> Result<(), String> {
    write_save_copy(input, output, ids, |data| {
        let mut reader = BufReader::new(Cursor::new(data));
        let header = find_first_frame(&mut reader)?;
        let types = parse_types(&read_frame(&mut reader, header, MAX_TABLE)?)?;
        validate_systems(&read_next_frame(&mut reader, MAX_TABLE)?)?;
        let table_start = reader.stream_position().map_err(|e| e.to_string())? as usize;
        let mut rows = parse_archetypes(&read_next_frame(&mut reader, MAX_TABLE)?, &types)?;
        let table_end = reader.stream_position().map_err(|e| e.to_string())? as usize;
        let tag = types
            .iter()
            .position(|ty| ty.name == UNSPAWNED && ty.flag == 1)
            .ok_or("このセーブに対応する非表示の型がありません")?;
        let mut table = (rows.len() as u32).to_le_bytes().to_vec();
        for row in &mut rows {
            let pet = row
                .types
                .iter()
                .any(|&id| types[id].name == "Game.Creatures.Pet");
            let target = row.kinds & (VEHICLE | PARKED | HUMAN) != 0 || pet;
            if target && !row.types.contains(&tag) {
                // ゲーム自身が使うデータなしの印。実体の順序と保存ID、各フレームはそのまま。
                row.types.push(tag);
            }
            put_archetype(&mut table, row.count, &row.types);
        }
        let mut output = data[..table_start].to_vec();
        put_frame(&mut output, &table)?;
        output.extend_from_slice(&data[table_end..]);
        Ok(output)
    })
}
