//! 満車・経路失敗・遠い車庫からの戻りを、少数の自家用車で確かめる試験用の読み書き。
//!
//! 位置を大量に動かす試験とは違い、ゲームの通常処理に入る条件だけを最小限に作る。
//! 保存IDはロード後の保存で変わるため、追跡は元セーブで一意な車両の乱数値と、
//! 市民の誕生日・乱数値の組（世帯側の記録）で行う。照合に必要な値は試験記録に書き出す。

use super::clean::{put_frame, write_save_copy};
use super::position::read_save_data;
use super::stride::StrideCodec;
use super::*;
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Seek};
use std::ops::Range;
use std::path::Path;

const NULL: u32 = u32::MAX;
// PersonalCarFlags と PathFlags のうち試験で使うビット。
const TRANSPORTING: u32 = 1;
const DUMMY_TRAFFIC: u32 = 8;
const HOME_TARGET: u32 = 0x10;
const PATH_FAILED: u16 = 2;

/// 1つのアーキタイプ行。フレームを展開したまま持ち、各コンポーネントの範囲を引けるようにする。
struct Row {
    start: u32,
    count: u32,
    types: Vec<usize>,
    frame: Vec<u8>,
    blocks: HashMap<usize, Range<usize>>,
    /// 試験で書き換えたら true。書き出し時に再圧縮する。
    dirty: bool,
}

/// セーブ全体を保存ID単位で読むための索引。
struct World {
    types: Vec<Component>,
    names: HashMap<String, usize>,
    rows: Vec<Row>,
    /// 先頭フレームまでのヘッダー・型表・システム領域と、末尾の残り。
    head: Range<usize>,
    tail: Range<usize>,
}

impl World {
    fn load(data: &[u8]) -> Result<Self, String> {
        let mut reader = BufReader::new(Cursor::new(data));
        let header = find_first_frame(&mut reader)?;
        let types = parse_types(&read_frame(&mut reader, header, MAX_TABLE)?)?;
        validate_systems(&read_next_frame(&mut reader, MAX_TABLE)?)?;
        let archetypes = parse_archetypes(&read_next_frame(&mut reader, MAX_TABLE)?, &types)?;
        let first = reader.stream_position().map_err(|e| e.to_string())? as usize;
        let mut rows = Vec::with_capacity(archetypes.len());
        for row in archetypes {
            let frame = read_next_frame(&mut reader, MAX_FRAME)?;
            let mut cursor = frame.as_slice();
            let mut blocks = HashMap::new();
            for &id in &row.types {
                if types[id].flag != 1 {
                    let start = frame.len() - cursor.len() + 4;
                    let payload = sized_bytes(&mut cursor)?;
                    blocks.insert(id, start..start + payload.len());
                }
            }
            require_end(cursor)?;
            rows.push(Row {
                start: u32::try_from(row.start).map_err(|e| e.to_string())?,
                count: row.count,
                types: row.types,
                frame,
                blocks,
                dirty: false,
            });
        }
        let end = reader.stream_position().map_err(|e| e.to_string())? as usize;
        let names = types
            .iter()
            .enumerate()
            .map(|(i, t)| (t.name.clone(), i))
            .collect();
        Ok(Self {
            types,
            names,
            rows,
            head: 0..first,
            tail: end..data.len(),
        })
    }

    fn id(&self, name: &str) -> Result<usize, String> {
        self.names
            .get(name)
            .copied()
            .ok_or_else(|| format!("{name} がありません"))
    }

    fn locate(&self, entity: u32) -> Option<(usize, usize)> {
        if entity == NULL {
            return None;
        }
        let i = self
            .rows
            .partition_point(|r| r.start <= entity)
            .checked_sub(1)?;
        let row = &self.rows[i];
        (entity < row.start + row.count).then(|| (i, (entity - row.start) as usize))
    }

    fn has(&self, entity: u32, ty: usize) -> bool {
        self.locate(entity)
            .is_some_and(|(r, _)| self.rows[r].types.contains(&ty))
    }

    /// 固定長で保存されるコンポーネント（形式2）の1件分の範囲。
    fn fixed_range(&self, entity: u32, ty: usize) -> Option<(usize, Range<usize>)> {
        let (r, i) = self.locate(entity)?;
        let row = &self.rows[r];
        if self.types[ty].flag != 2 {
            return None;
        }
        let block = row.blocks.get(&ty)?;
        let size = block.len() / row.count as usize;
        (size * row.count as usize == block.len())
            .then(|| (r, block.start + size * i..block.start + size * (i + 1)))
    }

    fn fixed(&self, entity: u32, ty: usize) -> Option<&[u8]> {
        let (r, range) = self.fixed_range(entity, ty)?;
        Some(&self.rows[r].frame[range])
    }

    fn u32_of(&self, entity: u32, ty: usize, offset: usize) -> Option<u32> {
        self.fixed(entity, ty)
            .map(|b| u32::from_le_bytes(b[offset..offset + 4].try_into().unwrap()))
    }

    /// 有効・無効だけを保存するコンポーネント（形式17・18）のビット。
    fn enabled(&self, entity: u32, ty: usize) -> Option<bool> {
        let (r, i) = self.locate(entity)?;
        let row = &self.rows[r];
        let block = row.blocks.get(&ty)?;
        Some(row.frame[block.start + i / 8] & (1 << (i % 8)) != 0)
    }

    fn position(&self, entity: u32, transform: usize) -> Option<[f32; 3]> {
        let b = self.fixed(entity, transform)?;
        Some(std::array::from_fn(|a| {
            f32::from_le_bytes(b[a * 4..a * 4 + 4].try_into().unwrap())
        }))
    }

    /// 経路の要素（PathElement）の行き先を、現在位置以降について返す。
    /// 要素は可変長で保存されるため、行の先頭から順に読み進める。
    fn path_targets(&self, entity: u32, ty: usize) -> Option<Vec<u32>> {
        let (r, index) = self.locate(entity)?;
        let row = &self.rows[r];
        let mut data = &row.frame[row.blocks.get(&ty)?.clone()];
        let mut result = None;
        for i in 0..row.count as usize {
            let count = uint(&mut data).ok()?;
            let mut targets = Vec::with_capacity(count as usize);
            for _ in 0..count {
                let target = u32::from_le_bytes(bytes(&mut data, 4).ok()?.try_into().unwrap());
                let mode = bytes(&mut data, 1).ok()?[0];
                // 0と1の端点は省略され、2のときだけ浮動小数で保存される。
                let extra = 4 * (usize::from(mode & 0xf == 2) + usize::from(mode >> 4 == 2));
                bytes(&mut data, extra + 1).ok()?;
                targets.push(target);
            }
            if i == index {
                result = Some(targets);
            }
        }
        result
    }

    fn write(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        let mut reader = BufReader::new(Cursor::new(data));
        reader
            .seek(std::io::SeekFrom::Start(self.head.end as u64))
            .map_err(|e| e.to_string())?;
        let mut output = data[self.head.clone()].to_vec();
        for row in &self.rows {
            let start = reader.stream_position().map_err(|e| e.to_string())? as usize;
            skip_frame(&mut reader)?;
            let end = reader.stream_position().map_err(|e| e.to_string())? as usize;
            if row.dirty {
                put_frame(&mut output, &row.frame)?;
            } else {
                output.extend_from_slice(&data[start..end]);
            }
        }
        output.extend_from_slice(&data[self.tail.clone()]);
        Ok(output)
    }

    fn patch(&mut self, entity: u32, ty: usize, offset: usize, value: &[u8]) -> Result<(), String> {
        let (r, range) = self.fixed_range(entity, ty).ok_or_else(|| {
            format!(
                "保存ID {entity} の {} を書き換えられません",
                self.types[ty].name
            )
        })?;
        let at = range.start + offset;
        if at + value.len() > range.end {
            return Err("書き換え範囲がレコードを超えます".into());
        }
        let row = &mut self.rows[r];
        row.frame[at..at + value.len()].copy_from_slice(value);
        row.dirty = true;
        Ok(())
    }
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f32>()
        .sqrt()
}

/// 試験で参照する型番号。
struct Ids {
    personal: usize,
    seed: usize,
    owner: usize,
    path_owner: usize,
    path: usize,
    parked: usize,
    transform: usize,
    unspawned: usize,
    garage: usize,
    curve: usize,
    citizen: usize,
    member: usize,
    building: usize,
    keeper: usize,
    renter: usize,
    /// このセーブに自転車が無ければ型表に載らない。
    bicycle: Option<usize>,
}

impl Ids {
    fn new(w: &World) -> Result<Self, String> {
        Ok(Self {
            personal: w.id("Game.Vehicles.PersonalCar")?,
            seed: w.id("Game.Common.PseudoRandomSeed")?,
            owner: w.id("Game.Common.Owner")?,
            path_owner: w.id("Game.Pathfind.PathOwner")?,
            path: w.id("Game.Pathfind.PathElement")?,
            parked: w.id("Game.Vehicles.ParkedCar")?,
            transform: w.id("Game.Objects.Transform")?,
            unspawned: w.id("Game.Objects.Unspawned")?,
            garage: w.id("Game.Net.GarageLane")?,
            curve: w.id("Game.Net.Curve")?,
            citizen: w.id("Game.Citizens.Citizen")?,
            member: w.id("Game.Citizens.HouseholdMember")?,
            building: w.id("Game.Citizens.CurrentBuilding")?,
            keeper: w.id("Game.Citizens.CarKeeper")?,
            renter: w.id("Game.Buildings.PropertyRenter")?,
            bicycle: w.id("Game.Vehicles.Bicycle").ok(),
        })
    }
}

#[derive(Clone, Debug)]
struct Car {
    id: u32,
    seed: u16,
    keeper: u32,
    state: u32,
    household: u32,
    /// 駐車中なら駐車先のレーン。走行中は NULL。
    lane: u32,
    path_state: Option<u16>,
    hidden: bool,
    /// 自転車も PersonalCar を持つ。ゲームは失敗時に自転車を削除するため、車と分けて扱う。
    bicycle: bool,
    position: [f32; 3],
}

#[derive(Clone, Debug)]
struct Garage {
    id: u32,
    count: u16,
    capacity: u16,
    center: [f32; 3],
}

/// 世帯・市民・車庫の索引。照合と試験コピーの作成で共有する。
struct Index {
    ids: Ids,
    cars: Vec<Car>,
    garages: HashMap<u32, Garage>,
    /// 世帯 → 構成員の市民
    members: HashMap<u32, Vec<u32>>,
}

impl Index {
    fn build(w: &World, codec: &StrideCodec) -> Result<Self, String> {
        let ids = Ids::new(w)?;
        let mut cars = Vec::new();
        let mut garages = HashMap::new();
        let mut members: HashMap<u32, Vec<u32>> = HashMap::new();
        for row in &w.rows {
            let has = |ty| row.types.contains(&ty);
            if has(ids.personal) && has(ids.seed) {
                for i in 0..row.count {
                    let id = row.start + i;
                    let state = w.u32_of(id, ids.personal, 4).unwrap();
                    cars.push(Car {
                        id,
                        seed: u16::from_le_bytes(
                            w.fixed(id, ids.seed).unwrap().try_into().unwrap(),
                        ),
                        keeper: w.u32_of(id, ids.personal, 0).unwrap(),
                        state,
                        household: w.u32_of(id, ids.owner, 0).unwrap_or(NULL),
                        lane: w.u32_of(id, ids.parked, 0).unwrap_or(NULL),
                        path_state: w
                            .fixed(id, ids.path_owner)
                            .map(|b| u16::from_le_bytes(b[4..6].try_into().unwrap())),
                        hidden: has(ids.unspawned),
                        bicycle: ids.bicycle.is_some_and(has),
                        position: w.position(id, ids.transform).unwrap_or([f32::NAN; 3]),
                    });
                }
            }
            if has(ids.garage) && has(ids.curve) {
                let encoded = &row.frame[row.blocks[&ids.curve].clone()];
                let decoded;
                let geometry = match w.types[ids.curve].flag {
                    2 => encoded,
                    34 => {
                        decoded = codec.transform(encoded, 12, true)?;
                        &decoded
                    }
                    _ => return Err("車庫レーンの形状形式が未対応です".into()),
                };
                for i in 0..row.count {
                    let id = row.start + i;
                    let g = w.fixed(id, ids.garage).unwrap();
                    let at = i as usize * 48;
                    garages.insert(
                        id,
                        Garage {
                            id,
                            count: u16::from_le_bytes(g[4..6].try_into().unwrap()),
                            capacity: u16::from_le_bytes(g[6..8].try_into().unwrap()),
                            center: std::array::from_fn(|a| {
                                f32::from_le_bytes(
                                    geometry[at + a * 4..at + a * 4 + 4].try_into().unwrap(),
                                )
                            }),
                        },
                    );
                }
            }
            if has(ids.citizen) && has(ids.member) {
                for i in 0..row.count {
                    let id = row.start + i;
                    members
                        .entry(w.u32_of(id, ids.member, 0).unwrap())
                        .or_default()
                        .push(id);
                }
            }
        }
        Ok(Self {
            ids,
            cars,
            garages,
            members,
        })
    }

    /// 市民の識別値（誕生日と乱数値）。保存し直しても変わらない。
    fn person(&self, w: &World, citizen: u32) -> Option<u32> {
        // Citizen の末尾2バイト(8..10)は乱数値で、保存し直しても変わらない。
        // 直前の2バイト(6..8)は実機の自動保存で値が変わったため識別に使わない。
        // 乱数値だけでは7万人規模で重なるので、世帯の住まいの座標（整数に丸めた値）を混ぜる。
        let seed = w
            .fixed(citizen, self.ids.citizen)
            .map(|b| u32::from(u16::from_le_bytes(b[8..10].try_into().unwrap())))?;
        let home = w
            .u32_of(citizen, self.ids.member, 0)
            .and_then(|h| self.home(w, h))
            .map_or(0, |p| {
                let (x, z) = (p[0].round() as i32 as u32, p[2].round() as i32 as u32);
                x.wrapping_mul(0x9E37_79B1) ^ z.wrapping_mul(0x85EB_CA77)
            });
        Some((seed << 16) | (home >> 16))
    }

    fn home(&self, w: &World, household: u32) -> Option<[f32; 3]> {
        let property = w.u32_of(household, self.ids.renter, 0)?;
        w.position(property, self.ids.transform)
    }

    fn household_people(&self, w: &World, household: u32) -> Vec<u32> {
        let mut people: Vec<u32> = self
            .members
            .get(&household)
            .into_iter()
            .flatten()
            .filter_map(|&c| self.person(w, c))
            .collect();
        people.sort_unstable();
        people
    }

    fn garage_of(&self, car: &Car) -> Option<&Garage> {
        self.garages.get(&car.lane)
    }
}

/// 試験記録の1行。作成時に書き出し、照合時に読み戻す。
#[derive(Debug)]
struct Tracked {
    group: String,
    seed: u16,
    keeper: Option<u32>,
    people: Vec<u32>,
    home: Option<[f32; 3]>,
    garage: Option<[f32; 3]>,
}

fn fmt_point(p: Option<[f32; 3]>) -> String {
    p.map_or("-".into(), |p| format!("{},{},{}", p[0], p[1], p[2]))
}

fn parse_point(s: &str) -> Result<Option<[f32; 3]>, String> {
    if s == "-" {
        return Ok(None);
    }
    let v: Vec<f32> = s
        .split(',')
        .map(|x| x.parse::<f32>().map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    v.try_into()
        .map(Some)
        .map_err(|_| "座標の形式が不正です".into())
}

impl Tracked {
    fn line(&self) -> String {
        format!(
            "{} {} {} {} {} {}",
            self.group,
            self.seed,
            self.keeper.map_or("-".into(), |k| k.to_string()),
            if self.people.is_empty() {
                "-".into()
            } else {
                self.people
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            },
            fmt_point(self.home),
            fmt_point(self.garage)
        )
    }

    fn parse(line: &str) -> Result<Self, String> {
        let f: Vec<&str> = line.split_whitespace().collect();
        let [group, seed, keeper, people, home, garage] = f[..] else {
            return Err(format!("試験記録の行が不正です: {line}"));
        };
        Ok(Self {
            group: group.into(),
            seed: seed.parse().map_err(|_| "乱数値が不正です")?,
            keeper: (keeper != "-")
                .then(|| keeper.parse().map_err(|_| "持ち主の識別値が不正です"))
                .transpose()?,
            people: if people == "-" {
                Vec::new()
            } else {
                people
                    .split(',')
                    .map(|p| p.parse().map_err(|_| "世帯の識別値が不正です".to_string()))
                    .collect::<Result<_, _>>()?
            },
            home: parse_point(home)?,
            garage: parse_point(garage)?,
        })
    }
}

/// 試験コピーを作り、追跡用の記録を返す。
///
/// 群:
/// - `full`: 走行中で、経路の先に車庫レーンがある車。その車庫を駐車車両で満車にする。
/// - `fail-carry`: 人を乗せて走行中の車。経路の状態に Failed を立てる。
/// - `fail-empty`: 人を乗せずに走行中の車。経路の状態に Failed を立てる。
/// - `far-kept` / `far-free`: 持ち主（運転役の市民）付き・なしの駐車車両を、
///   家から1km以上離れた空きのある車庫へ移す。
pub fn write_vehicle_trial(
    input: &Path,
    output: &Path,
    plugin: &Path,
    ids: [&str; 4],
) -> Result<String, String> {
    let codec = StrideCodec::open(plugin)?;
    let mut manifest = String::new();
    write_save_copy(input, output, ids, |data| {
        let mut w = World::load(data)?;
        let x = Index::build(&w, &codec)?;
        let k = &x.ids;
        let mut seeds = HashMap::<u16, usize>::new();
        for car in &x.cars {
            *seeds.entry(car.seed).or_default() += 1;
        }
        let unique = |c: &Car| seeds[&c.seed] == 1 && !c.bicycle;
        let mut used = HashSet::new();
        let mut tracked = Vec::new();
        // 世帯・持ち主の識別値は編集で変わらないため、書き換え後の World から読んでもよい。
        let record = |group: &str,
                      car: &Car,
                      garage: Option<[f32; 3]>,
                      used: &mut HashSet<u32>,
                      tracked: &mut Vec<Tracked>,
                      w: &World| {
            used.insert(car.id);
            tracked.push(Tracked {
                group: group.into(),
                seed: car.seed,
                keeper: (car.keeper != NULL)
                    .then(|| x.person(w, car.keeper))
                    .flatten(),
                people: x.household_people(w, car.household),
                home: x.home(w, car.household),
                garage,
            });
        };
        let moving = |c: &&Car| {
            c.lane == NULL
                && c.path_state.is_some()
                && c.state & DUMMY_TRAFFIC == 0
                && unique(c)
                && c.keeper != NULL
        };

        // 1. 満車の車庫に向かう車。経路の残りに含まれる車庫レーンごとに数える。
        let mut inbound: HashMap<u32, Vec<&Car>> = HashMap::new();
        for car in x.cars.iter().filter(moving) {
            let Some(targets) = w.path_targets(car.id, k.path) else {
                continue;
            };
            let index = w.fixed(car.id, k.path_owner).map_or(0, |b| {
                i32::from_le_bytes(b[0..4].try_into().unwrap()).max(0) as usize
            });
            if let Some(g) = targets
                .iter()
                .skip(index)
                .find(|t| x.garages.contains_key(t))
            {
                inbound.entry(*g).or_default().push(car);
            }
        }
        let residents = |g: u32| x.cars.iter().filter(move |c| c.lane == g).count();
        let (full_id, arrivals) = inbound
            .iter()
            .filter(|(g, _)| x.cars.iter().any(|c| c.lane == **g && c.hidden))
            .filter(|(g, _)| x.garages[g].capacity > 0 && x.garages[g].capacity <= 40)
            .max_by_key(|(g, cars)| (cars.len(), std::cmp::Reverse(x.garages[g].capacity)))
            .map(|(g, cars)| (*g, cars.clone()))
            .ok_or("車庫へ向かう走行中の車がありません")?;
        let full = x.garages[&full_id].clone();
        let arrivals: Vec<Car> = arrivals.into_iter().take(5).cloned().collect();
        for car in &arrivals {
            record("full", car, Some(full.center), &mut used, &mut tracked, &w);
        }
        // 既存の駐車車両の駐車参照と姿勢を写し、定員ちょうどまで埋める。
        let template = x
            .cars
            .iter()
            .find(|c| c.lane == full_id && c.hidden)
            .unwrap()
            .id;
        let parking = w.fixed(template, k.parked).unwrap().to_vec();
        let pose = w.fixed(template, k.transform).unwrap().to_vec();
        let need = usize::from(full.capacity).saturating_sub(residents(full_id));
        let fillers: Vec<Car> = x
            .cars
            .iter()
            .filter(|c| {
                c.hidden
                    && c.lane != full_id
                    && x.garages.contains_key(&c.lane)
                    && unique(c)
                    && !used.contains(&c.id)
            })
            .take(need)
            .cloned()
            .collect();
        if fillers.len() != need {
            return Err("満車にする駐車車両が足りません".into());
        }
        for car in &fillers {
            w.patch(car.id, k.parked, 0, &parking)?;
            w.patch(car.id, k.transform, 0, &pose)?;
            record(
                "filler",
                car,
                Some(full.center),
                &mut used,
                &mut tracked,
                &w,
            );
        }
        // ゲームが満車判定に使う集計値も実数に合わせる。
        let total = residents(full_id) + fillers.len();
        w.patch(full_id, k.garage, 4, &(total as u16).to_le_bytes())?;

        // 2. 経路失敗。乗車中と空車を別々に数台ずつ。
        for (group, carrying) in [("fail-carry", true), ("fail-empty", false)] {
            let chosen: Vec<Car> = x
                .cars
                .iter()
                .filter(moving)
                .filter(|c| (c.state & TRANSPORTING != 0) == carrying && !used.contains(&c.id))
                .filter(|c| c.path_state.is_some_and(|s| s & PATH_FAILED == 0))
                // 家のある世帯の車に限る（家へ戻す修正処理を観察するため）。
                .filter(|c| x.home(&w, c.household).is_some() && !c.hidden)
                .take(4)
                .cloned()
                .collect();
            for car in &chosen {
                let state = car.path_state.unwrap() | PATH_FAILED;
                w.patch(car.id, k.path_owner, 4, &state.to_le_bytes())?;
                record(group, car, None, &mut used, &mut tracked, &w);
            }
        }

        // 3. 家から遠い車庫に置いた駐車車両。空きが十分ある車庫を選び、定員内に収める。
        let far = x
            .garages
            .values()
            .filter(|g| g.id != full_id && usize::from(g.capacity) >= residents(g.id) + 4)
            .filter(|g| x.cars.iter().any(|c| c.lane == g.id && c.hidden))
            .max_by_key(|g| usize::from(g.capacity) - residents(g.id))
            .cloned()
            .ok_or("空きのある車庫がありません")?;
        let template = x
            .cars
            .iter()
            .find(|c| c.lane == far.id && c.hidden)
            .unwrap()
            .id;
        // 定員内に収まる台数だけ移す（定員超過は前回の試験で別に確認済み）。
        let free = usize::from(far.capacity) - residents(far.id);
        let parking = w.fixed(template, k.parked).unwrap().to_vec();
        let pose = w.fixed(template, k.transform).unwrap().to_vec();
        for (group, kept) in [("far-kept", true), ("far-free", false)] {
            let chosen: Vec<Car> = x
                .cars
                .iter()
                .filter(|c| {
                    c.lane != NULL && c.lane != far.id && unique(c) && !used.contains(&c.id)
                })
                // 屋内の車庫から車庫へ移す。路上の表示中の車は非表示の印を足す必要があるため使わない。
                .filter(|c| c.hidden && x.garages.contains_key(&c.lane))
                .filter(|c| (c.keeper != NULL) == kept && c.state & DUMMY_TRAFFIC == 0)
                .filter(|c| {
                    x.home(&w, c.household)
                        .is_some_and(|h| distance(h, far.center) > 1000.0)
                })
                // 持ち主付きは、持ち主が家の外にいる車を選ぶ（帰宅時に車庫から出る場面を作る）。
                .filter(|c| {
                    !kept || {
                        let home = w.u32_of(c.household, k.renter, 0);
                        let at = w.u32_of(c.keeper, k.building, 0);
                        home.is_some() && at != home
                    }
                })
                .take((if kept { free.div_ceil(2) } else { free / 2 }).min(5))
                .cloned()
                .collect();
            for car in &chosen {
                w.patch(car.id, k.parked, 0, &parking)?;
                w.patch(car.id, k.transform, 0, &pose)?;
                if !car.hidden {
                    return Err("表示中の車を車庫へ移すには非表示の印が必要です".into());
                }
                record(group, car, Some(far.center), &mut used, &mut tracked, &w);
            }
        }
        let total = residents(far.id)
            + tracked
                .iter()
                .filter(|t| t.group.starts_with("far"))
                .count();
        w.patch(far.id, k.garage, 4, &(total as u16).to_le_bytes())?;

        writeln!(manifest, "# full={:?} far={:?}", full, far).unwrap();
        for t in &tracked {
            writeln!(manifest, "{}", t.line()).unwrap();
        }
        w.write(data)
    })?;
    Ok(manifest)
}

/// 試験記録の車を自動保存などから探し、持ち主・世帯・位置・表示状態を照合する。
pub fn inspect_vehicle_trial(
    input: &Path,
    plugin: &Path,
    manifest: &str,
) -> Result<String, String> {
    let codec = StrideCodec::open(plugin)?;
    let data = read_save_data(input)?;
    let w = World::load(&data)?;
    let x = Index::build(&w, &codec)?;
    let k = &x.ids;
    let mut out = String::new();
    // 照合では保存IDではなく市民の識別値で引く。
    let mut people_index: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&household, members) in &x.members {
        for &m in members {
            if let Some(p) = x.person(&w, m) {
                people_index.entry(p).or_default().push(household);
            }
        }
    }
    let near = |a: Option<[f32; 3]>, b: [f32; 3]| a.is_some_and(|a| distance(a, b) < 1.0);
    for line in manifest
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let t = Tracked::parse(line)?;
        let found: Vec<&Car> = x.cars.iter().filter(|c| c.seed == t.seed).collect();
        let mut parts = Vec::new();
        for car in &found {
            let people = x.household_people(&w, car.household);
            let keeper = (car.keeper != NULL)
                .then(|| x.person(&w, car.keeper))
                .flatten();
            let keeper_on = (car.keeper != NULL)
                .then(|| w.enabled(car.keeper, k.keeper))
                .flatten();
            let garage = x.garage_of(car);
            let home = x.home(&w, car.household);
            parts.push(format!(
                "{} bicycle={} path_to_tracked_garage={} dist_to_tracked_garage={} same_household={} keeper={} keeper_flag={} same_keeper={} lane={} in_tracked_garage={} garage_load={} home_dist={} hidden={} state={:#x} home_target={}",
                if car.lane != NULL { "parked" } else if car.path_state.is_some() { "moving" } else { "other" },
                car.bicycle,
                t.garage.is_some_and(|g| w.path_targets(car.id, k.path).is_some_and(|ts| ts.iter().any(|id| x.garages.get(id).is_some_and(|gg| distance(gg.center, g) < 1.0)))),
                t.garage.map_or("-".into(), |g| format!("{:.0}", distance(g, car.position))),
                !t.people.is_empty() && people == t.people,
                keeper.map_or("-".into(), |p| p.to_string()),
                keeper_on.map_or("-".into(), |b| b.to_string()),
                t.keeper.is_some() && keeper == t.keeper,
                if garage.is_some() { "garage" } else if car.lane == NULL { "none" } else { "street" },
                t.garage.is_some_and(|g| near(garage.map(|g| g.center), g)),
                garage.map_or("-".into(), |g| {
                    format!("{}/{}/{}", x.cars.iter().filter(|c| c.lane == g.id).count(), g.count, g.capacity)
                }),
                home.map_or("-".into(), |h| format!("{:.0}", distance(h, car.position))),
                car.hidden,
                car.state,
                car.state & HOME_TARGET != 0,
            ));
        }
        // 車が見つからない場合でも、元の持ち主側の記録が残っているかを示す。
        let keeper_alive = t.keeper.map(|p| people_index.contains_key(&p));
        writeln!(
            out,
            "{} seed={} found={} keeper_citizen_exists={:?} | {}",
            t.group,
            t.seed,
            found.len(),
            keeper_alive,
            parts.join(" ; ")
        )
        .unwrap();
    }
    Ok(out)
}

/// 人・ペットの「施設に収容した状態」を、街に実体を持たない市民・世帯ペットの居場所だけで作る。
///
/// 実体（人物・ペットのクリーチャー）は経路やレーンを持つため、座標だけ動かすと東端試験と同じ破綻になる。
/// そこで家にいて実体を持たない市民と世帯ペットの CurrentBuilding だけを、家から1km以上離れた建物 B に変える。
/// 次の外出でゲームが B から実体を作るか、家へ戻すかを観察する。
/// 群:
/// - `citizen`: 世帯ペットのいない世帯の市民1人。
/// - `with-pet`: 世帯ペットのいる世帯の市民1人と、その世帯の家にいるペット全部。
pub fn write_people_trial(
    input: &Path,
    output: &Path,
    plugin: &Path,
    ids: [&str; 4],
) -> Result<String, String> {
    let codec = StrideCodec::open(plugin)?;
    let mut manifest = String::new();
    write_save_copy(input, output, ids, |data| {
        let mut w = World::load(data)?;
        let x = Index::build(&w, &codec)?;
        let k = &x.ids;
        let transport = w.id("Game.Citizens.CurrentTransport")?;
        let pet_record = w.id("Game.Citizens.HouseholdPet")?;
        let building = w.id("Game.Buildings.Building")?;
        // B は前回の試験で使った「家から遠い車庫」と同じ駐車場の建物にする。車庫レーンから所有者をたどる。
        let lane = x
            .garages
            .values()
            .min_by(|a, b| {
                let target = [1805.464, 605.3842, -4384.997];
                distance(a.center, target).total_cmp(&distance(b.center, target))
            })
            .ok_or("車庫がありません")?
            .id;
        let mut b = lane;
        while let Some(owner) = w.u32_of(b, k.owner, 0).filter(|&o| o != NULL) {
            b = owner;
        }
        if !w.has(b, building) {
            return Err("車庫の所有者が建物ではありません".into());
        }
        let b_pos = w
            .position(b, k.transform)
            .ok_or("建物 B の座標がありません")?;
        // 世帯 → 家にいて実体を持たない世帯ペット
        let mut pets: HashMap<u32, Vec<u32>> = HashMap::new();
        for row in &w.rows {
            if row.types.contains(&pet_record)
                && row.types.contains(&k.building)
                && !row.types.contains(&transport)
            {
                for i in 0..row.count {
                    let pet = row.start + i;
                    let household = w.u32_of(pet, pet_record, 0).unwrap();
                    let home = w.u32_of(household, k.renter, 0);
                    if home.is_some() && w.u32_of(pet, k.building, 0) == home {
                        pets.entry(household).or_default().push(pet);
                    }
                }
            }
        }
        let mut chosen = Vec::new();
        let mut households = HashSet::new();
        for row in &w.rows {
            if !(row.types.contains(&k.citizen)
                && row.types.contains(&k.member)
                && row.types.contains(&k.building))
                || row.types.contains(&transport)
            {
                continue;
            }
            for i in 0..row.count {
                let citizen = row.start + i;
                let household = w.u32_of(citizen, k.member, 0).unwrap();
                let home = w.u32_of(household, k.renter, 0);
                let at_home = home.is_some() && w.u32_of(citizen, k.building, 0) == home;
                let far = x
                    .home(&w, household)
                    .is_some_and(|h| distance(h, b_pos) > 1000.0);
                if !at_home || !far || households.contains(&household) {
                    continue;
                }
                let has_pet = pets.contains_key(&household);
                let wanted = if has_pet { "with-pet" } else { "citizen" };
                if chosen.iter().filter(|(g, _, _)| *g == wanted).count() >= 5 {
                    continue;
                }
                households.insert(household);
                chosen.push((wanted, citizen, household));
            }
        }
        writeln!(manifest, "# B={},{},{}", b_pos[0], b_pos[1], b_pos[2]).unwrap();
        for (group, citizen, household) in &chosen {
            w.patch(*citizen, k.building, 0, &b.to_le_bytes())?;
            let mut moved_pets = 0;
            if *group == "with-pet" {
                for pet in &pets[household] {
                    w.patch(*pet, k.building, 0, &b.to_le_bytes())?;
                    moved_pets += 1;
                }
            }
            writeln!(
                manifest,
                "{group} {} {} {moved_pets} {}",
                x.person(&w, *citizen).unwrap(),
                household_key(&x, &w, *household),
                fmt_point(x.home(&w, *household))
            )
            .unwrap();
        }
        w.write(data)
    })?;
    Ok(manifest)
}

/// 世帯の識別値（構成員の識別値をまとめたもの）。
fn household_key(x: &Index, w: &World, household: u32) -> u64 {
    x.household_people(w, household)
        .iter()
        .fold(0xcbf29ce484222325u64, |h, p| {
            (h ^ u64::from(*p)).wrapping_mul(0x100000001b3)
        })
}

/// 人・ペットの試験記録を照合する。市民と世帯ペットの居場所と、外出中なら実体の位置を示す。
pub fn inspect_people_trial(input: &Path, plugin: &Path, manifest: &str) -> Result<String, String> {
    let codec = StrideCodec::open(plugin)?;
    let data = read_save_data(input)?;
    let w = World::load(&data)?;
    let x = Index::build(&w, &codec)?;
    let k = &x.ids;
    let transport = w.id("Game.Citizens.CurrentTransport")?;
    let pet_record = w.id("Game.Citizens.HouseholdPet")?;
    let b_pos = manifest
        .lines()
        .find_map(|l| l.strip_prefix("# B="))
        .map(parse_point)
        .transpose()?
        .flatten()
        .ok_or("試験記録に B がありません")?;
    let mut citizens = HashMap::new();
    let mut households = HashMap::new();
    for (&household, members) in &x.members {
        households.insert(household_key(&x, &w, household), household);
        for &m in members {
            if let Some(p) = x.person(&w, m) {
                citizens.insert(p, m);
            }
        }
    }
    let mut pets: HashMap<u32, Vec<u32>> = HashMap::new();
    for row in w.rows.iter().filter(|r| r.types.contains(&pet_record)) {
        for i in 0..row.count {
            pets.entry(w.u32_of(row.start + i, pet_record, 0).unwrap())
                .or_default()
                .push(row.start + i);
        }
    }
    // 居場所（建物）と、外出中の実体の位置を、B・家からの距離で表す。
    let vehicle = w.id("Game.Creatures.CurrentVehicle")?;
    let target = w.id("Game.Common.Target")?;
    let place = |entity: u32, home: Option<[f32; 3]>| -> String {
        let at = w
            .u32_of(entity, k.building, 0)
            .and_then(|b| w.position(b, k.transform))
            .map_or("-".into(), |p| format!("{:.0}", distance(p, b_pos)));
        let creature = w.u32_of(entity, transport, 0).filter(|&c| c != NULL);
        let out = creature.map_or("-".into(), |c| {
            let hidden = w
                .locate(c)
                .is_some_and(|(r, _)| w.rows[r].types.contains(&k.unspawned));
            // 実体の乱数値・乗車の有無・家までの距離・行き先が家かどうかも示す。
            let seed = w.fixed(c, k.seed).map_or(0, |b| u16::from_le_bytes(b.try_into().unwrap()));
            let riding = w.fixed(c, vehicle).is_some();
            let to_home = w
                .u32_of(c, target, 0)
                .and_then(|t| w.position(t, k.transform))
                .zip(home)
                .map(|(t, h)| distance(t, h) < 1.0);
            w.position(c, k.transform).map_or("?".into(), |p| {
                format!(
                    "{:.0}(hidden={hidden} seed={seed} in_vehicle={riding} dist_to_home={} target_is_home={to_home:?})",
                    distance(p, b_pos),
                    home.map_or("-".into(), |h| format!("{:.0}", distance(p, h)))
                )
            })
        });
        format!("building_dist_to_B={at} creature_dist_to_B={out}")
    };
    let mut out = String::new();
    for line in manifest
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let f: Vec<&str> = line.split_whitespace().collect();
        let [group, person, key, pet_count, home] = f[..] else {
            return Err(format!("試験記録の行が不正です: {line}"));
        };
        let person: u32 = person.parse().map_err(|_| "識別値が不正です")?;
        let key: u64 = key.parse().map_err(|_| "世帯の識別値が不正です")?;
        let home = parse_point(home)?;
        let citizen = citizens.get(&person);
        let household = households.get(&key);
        let home_note = |entity: u32| {
            let at = w
                .u32_of(entity, k.building, 0)
                .and_then(|b| w.position(b, k.transform));
            match (at, home) {
                (Some(a), Some(h)) => format!("{:.0}", distance(a, h)),
                _ => "-".into(),
            }
        };
        write!(
            out,
            "{group} citizen_found={} household_found={} ",
            citizen.is_some(),
            household.is_some()
        )
        .unwrap();
        if let Some(&c) = citizen {
            write!(
                out,
                "citizen[{} building_dist_to_home={}] ",
                place(c, home),
                home_note(c)
            )
            .unwrap();
        }
        if pet_count != "0"
            && let Some(h) = household
        {
            for &p in pets.get(h).into_iter().flatten() {
                write!(
                    out,
                    "pet[{} building_dist_to_home={}] ",
                    place(p, home),
                    home_note(p)
                )
                .unwrap();
            }
            write!(out, "pets_before={pet_count}").unwrap();
        }
        writeln!(out).unwrap();
    }
    Ok(out)
}

/// 試験記録の持ち主・世帯の識別値を、試験コピー自体から計算し直す。
/// 識別値の定義を変えたときに、同じ車（乱数値）のまま記録を作り直すために使う。
pub fn rekey_manifest(trial: &Path, plugin: &Path, manifest: &str) -> Result<String, String> {
    let codec = StrideCodec::open(plugin)?;
    let data = read_save_data(trial)?;
    let w = World::load(&data)?;
    let x = Index::build(&w, &codec)?;
    let mut out = String::new();
    for line in manifest.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            writeln!(out, "{line}").unwrap();
            continue;
        }
        let mut t = Tracked::parse(line)?;
        let car = x
            .cars
            .iter()
            .find(|c| c.seed == t.seed)
            .ok_or_else(|| format!("乱数値 {} の車が試験コピーにありません", t.seed))?;
        t.keeper = (car.keeper != NULL)
            .then(|| x.person(&w, car.keeper))
            .flatten();
        t.people = x.household_people(&w, car.household);
        writeln!(out, "{}", t.line()).unwrap();
    }
    Ok(out)
}

/// 人・ペットの実体を、持ち主側の記録（市民・世帯ペット）の CurrentTransport からたどって分類する。
/// キーは (持ち主の識別値, 実体の乱数値)。保存し直しても同じ実体なら同じキーになる。
fn creatures(w: &World, x: &Index) -> Result<HashMap<(u64, u16, bool), &'static str>, String> {
    let transport = w.id("Game.Citizens.CurrentTransport")?;
    let pet_record = w.id("Game.Citizens.HouseholdPet")?;
    let vehicle = w.id("Game.Creatures.CurrentVehicle")?;
    let trip_source = w.id("Game.Objects.TripSource")?;
    let resident = w.id("Game.Creatures.Resident")?;
    let pet = w.id("Game.Creatures.Pet")?;
    let mut out = HashMap::new();
    for row in &w.rows {
        let is_citizen = row.types.contains(&x.ids.citizen);
        let is_pet = row.types.contains(&pet_record);
        if !(is_citizen || is_pet) || !row.types.contains(&transport) {
            continue;
        }
        for i in 0..row.count {
            let owner = row.start + i;
            let Some(creature) = w.u32_of(owner, transport, 0) else {
                continue;
            };
            let Some(seed) = w.fixed(creature, x.ids.seed) else {
                continue;
            };
            let seed = u16::from_le_bytes(seed.try_into().unwrap());
            let ident = if is_citizen {
                u64::from(x.person(w, owner).unwrap_or(0))
            } else {
                // 世帯ペットは HouseholdPet の先頭4バイトが世帯。世帯の構成員の識別値をまとめて使う。
                let household = w.u32_of(owner, pet_record, 0).unwrap_or(NULL);
                x.household_people(w, household)
                    .iter()
                    .fold(0xcbf29ce484222325u64, |h, p| {
                        (h ^ u64::from(*p)).wrapping_mul(0x100000001b3)
                    })
            };
            let flags = if is_citizen {
                w.u32_of(creature, resident, 0)
            } else {
                w.u32_of(creature, pet, 0)
            }
            .unwrap_or(0);
            let arrived = if is_citizen {
                flags & 8 != 0
            } else {
                flags & 4 != 0
            };
            let category = if w.fixed(creature, vehicle).is_some() {
                "in-vehicle"
            } else if w.fixed(creature, trip_source).is_some() {
                "waiting-in-building"
            } else if arrived {
                "arrived"
            } else if w
                .locate(creature)
                .is_some_and(|(r, _)| w.rows[r].types.contains(&x.ids.unspawned))
            {
                "hidden"
            } else {
                "walking"
            };
            out.insert((ident, seed, is_citizen), category);
        }
    }
    Ok(out)
}

/// 元セーブと自動保存の人・ペットの実体を比べ、同じ実体が残ったか・どの状態に移ったかを数える。
pub fn compare_creatures(before: &Path, after: &Path, plugin: &Path) -> Result<String, String> {
    let codec = StrideCodec::open(plugin)?;
    let mut maps = Vec::new();
    let mut owners = Vec::new();
    for path in [before, after] {
        let data = read_save_data(path)?;
        let w = World::load(&data)?;
        let x = Index::build(&w, &codec)?;
        let map = creatures(&w, &x)?;
        owners.push(map.keys().map(|k| (k.0, k.2)).collect::<HashSet<_>>());
        maps.push(map);
    }
    let mut table: HashMap<(bool, &str, &str), usize> = HashMap::new();
    for (key, from) in &maps[0] {
        let to = match maps[1].get(key) {
            Some(to) => *to,
            None if owners[1].contains(&(key.0, key.2)) => "replaced-by-new-entity",
            None => "no-entity-now",
        };
        *table.entry((key.2, from, to)).or_default() += 1;
    }
    let mut rows: Vec<_> = table.into_iter().collect();
    rows.sort();
    let mut out = format!("before={} after={}\n", maps[0].len(), maps[1].len());
    for ((human, from, to), n) in rows {
        writeln!(
            out,
            "{} {from} -> {to}: {n}",
            if human { "human" } else { "pet" }
        )
        .unwrap();
    }
    Ok(out)
}

/// セーブが何で重いかを調べる：ZIP内の各部品、実体データ（行ごとの圧縮サイズ）、
/// 実体データの後ろに続くシステム領域の大きさを集計する。
/// 行の圧縮サイズは、行内の各コンポーネントの展開サイズの比率で按分する（近似）。
pub fn save_sizes(input: &Path) -> Result<String, String> {
    let mut out = String::new();
    let file = std::fs::File::open(input).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    writeln!(out, "== ZIP内の部品").unwrap();
    for i in 0..zip.len() {
        let f = zip.by_index(i).map_err(|e| e.to_string())?;
        // zip 9 からは名前の取り出しが失敗し得る。
        let name = f.name().map_err(|e| e.to_string())?;
        writeln!(out, "{:>12} {}", f.size(), name).unwrap();
    }
    drop(zip);
    let data = read_save_data(input)?;
    let mut reader = BufReader::new(Cursor::new(data.as_slice()));
    let header = find_first_frame(&mut reader)?;
    let types = parse_types(&read_frame(&mut reader, header, MAX_TABLE)?)?;
    validate_systems(&read_next_frame(&mut reader, MAX_TABLE)?)?;
    let rows = parse_archetypes(&read_next_frame(&mut reader, MAX_TABLE)?, &types)?;
    let first = reader.stream_position().map_err(|e| e.to_string())? as usize;
    // 型ごと: (実体数, 展開バイト, 按分した圧縮バイト)
    let mut by_type: HashMap<usize, (u64, u64, f64)> = HashMap::new();
    let (mut packed_total, mut raw_total, mut entities) = (0u64, 0u64, 0u64);
    for row in &rows {
        let start = reader.stream_position().map_err(|e| e.to_string())?;
        let frame = read_next_frame(&mut reader, MAX_FRAME)?;
        let packed = reader.stream_position().map_err(|e| e.to_string())? - start;
        packed_total += packed;
        raw_total += frame.len() as u64;
        entities += u64::from(row.count);
        let mut cursor = frame.as_slice();
        let mut sizes = Vec::new();
        for &id in &row.types {
            let len = if types[id].flag == 1 { 0 } else { sized_bytes(&mut cursor)?.len() };
            sizes.push((id, len as u64));
        }
        let sum = sizes.iter().map(|s| s.1).sum::<u64>().max(1);
        for (id, len) in sizes {
            let e = by_type.entry(id).or_default();
            e.0 += u64::from(row.count);
            e.1 += len;
            e.2 += packed as f64 * len as f64 / sum as f64;
        }
    }
    let after = reader.stream_position().map_err(|e| e.to_string())? as usize;
    writeln!(
        out,
        "\n== SaveGameData {} バイト\n先頭(型表など) {} / 実体データ 圧縮{} 展開{} ({}行, {}実体) / 実体の後ろの領域 {}",
        data.len(),
        first,
        packed_total,
        raw_total,
        rows.len(),
        entities,
        data.len() - after
    )
    .unwrap();
    // 実体の後ろの領域も同じ「サイズ2つ＋Zstd」の並びなら、フレームごとに大きさを示す。
    let mut frames = Vec::new();
    while let Ok(h) = FrameHeader::read(&mut reader) {
        let pos = reader.stream_position().map_err(|e| e.to_string())? as usize;
        if h.packed == 0 || pos + h.packed > data.len() || data[pos..pos + 4] != ZSTD_MAGIC {
            break;
        }
        frames.push((frames.len(), h.raw, h.packed));
        reader
            .seek(std::io::SeekFrom::Start((pos + h.packed) as u64))
            .map_err(|e| e.to_string())?;
    }
    let rest = data.len() - reader.stream_position().map_err(|e| e.to_string())? as usize;
    frames.sort_by_key(|f| std::cmp::Reverse(f.2));
    writeln!(out, "後ろの領域のフレーム {} 個（解釈できない残り {} バイト）。圧縮サイズ上位:", frames.len(), rest).unwrap();
    for (i, raw, packed) in frames.iter().take(15) {
        writeln!(out, "  #{i:<5} 圧縮{packed:>11} 展開{raw:>11}").unwrap();
    }
    let mut list: Vec<_> = by_type.into_iter().collect();
    list.sort_by(|a, b| b.1.2.total_cmp(&a.1.2));
    writeln!(out, "\n== 実体データの型別（圧縮サイズは按分の近似）上位40").unwrap();
    for (id, (count, raw, packed)) in list.iter().take(40) {
        writeln!(out, "{:>11.0} 圧縮 {raw:>11} 展開 {count:>8}実体  {}", packed, types[*id].name).unwrap();
    }
    Ok(out)
}

/// 「引っ越し中」の世帯が溜まっていないかを調べる。
/// ゲームは引っ越し中の世帯を、生きている全員が街の出入口に着いた（MovingAwayReachOC）時点で消す。
/// 1人でも着かない世帯は残り続けるため、構成員の状態を集計する。
pub fn moving_away_report(input: &Path) -> Result<String, String> {
    let data = read_save_data(input)?;
    let w = World::load(&data)?;
    let moving = w.id("Game.Agents.MovingAway")?;
    let household = w.id("Game.Citizens.Household")?;
    let citizen = w.id("Game.Citizens.Citizen")?;
    let member = w.id("Game.Citizens.HouseholdMember")?;
    let transport = w.id("Game.Citizens.CurrentTransport")?;
    let building = w.id("Game.Citizens.CurrentBuilding")?;
    let renter = w.id("Game.Buildings.PropertyRenter")?;
    let health = w.id("Game.Citizens.HealthProblem").ok();
    // 世帯 → (到着済み, 移動中, 建物内, その他) の人数
    let mut households: HashMap<u32, [u32; 4]> = HashMap::new();
    let mut reasons: HashMap<u32, u64> = HashMap::new();
    let mut with_home = 0u64;
    for row in &w.rows {
        if row.types.contains(&household) && row.types.contains(&moving) {
            for i in 0..row.count {
                let h = row.start + i;
                households.insert(h, [0; 4]);
                *reasons.entry(w.u32_of(h, moving, 4).unwrap_or(u32::MAX)).or_default() += 1;
                with_home += u64::from(w.has(h, renter));
            }
        }
    }
    let mut dead_or_sick = 0u64;
    for row in &w.rows {
        if !(row.types.contains(&citizen) && row.types.contains(&member)) {
            continue;
        }
        for i in 0..row.count {
            let c = row.start + i;
            let Some(slot) = households.get_mut(&w.u32_of(c, member, 0).unwrap()) else {
                continue;
            };
            let state = w.fixed(c, citizen).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
            let index = if state & 4 != 0 {
                0
            } else if row.types.contains(&transport) {
                1
            } else if row.types.contains(&building) {
                2
            } else {
                3
            };
            slot[index] += 1;
            dead_or_sick += u64::from(health.is_some_and(|hp| row.types.contains(&hp)));
        }
    }
    let mut out = format!(
        "引っ越し中の世帯 {}（家の記録が残る {with_home}）、構成員のうち健康上の問題あり {dead_or_sick}\n",
        households.len()
    );
    let sum = |k: usize| households.values().map(|s| u64::from(s[k])).sum::<u64>();
    writeln!(
        out,
        "構成員: 出入口に到着済み {} / 移動中 {} / 建物内 {} / どれでもない {}",
        sum(0),
        sum(1),
        sum(2),
        sum(3)
    )
    .unwrap();
    let empty = households.values().filter(|s| s.iter().sum::<u32>() == 0).count();
    let partly = households
        .values()
        .filter(|s| s[0] > 0 && s[1] + s[2] + s[3] > 0)
        .count();
    let none_moving = households
        .values()
        .filter(|s| s[0] == 0 && s[1] == 0 && s[2] + s[3] > 0)
        .count();
    writeln!(
        out,
        "構成員0人の世帯 {empty} / 一部だけ到着 {partly} / 誰も動いていない {none_moving}"
    )
    .unwrap();
    let mut r: Vec<_> = reasons.into_iter().collect();
    r.sort();
    writeln!(out, "理由(MoveAwayReason)別: {r:?}").unwrap();
    Ok(out)
}

/// 市民を「人口に数えられる住民」とそれ以外（観光客・通勤者・家なし・引っ越し中）に分けて数える。
/// シミュレーションは人口表示に出ない市民も処理するため、負荷の母数を知るために使う。
pub fn citizen_categories(input: &Path) -> Result<String, String> {
    let data = read_save_data(input)?;
    let w = World::load(&data)?;
    let citizen = w.id("Game.Citizens.Citizen")?;
    let member = w.id("Game.Citizens.HouseholdMember")?;
    let moving = w.id("Game.Agents.MovingAway")?;
    let renter = w.id("Game.Buildings.PropertyRenter")?;
    let transport = w.id("Game.Citizens.CurrentTransport")?;
    let worker = w.id("Game.Citizens.Worker").ok();
    let student = w.id("Game.Citizens.Student").ok();
    let health = w.id("Game.Citizens.HealthProblem").ok();
    // (人数, 移動中, 失業中の求職対象)
    let mut counts: HashMap<&str, (u64, u64, u64)> = HashMap::new();
    for row in &w.rows {
        if !(row.types.contains(&citizen) && row.types.contains(&member)) {
            continue;
        }
        let traveling = row.types.contains(&transport);
        let has = |t: Option<usize>| t.is_some_and(|t| row.types.contains(&t));
        // CitizenFindJobSystem の失業者側の対象：仕事・学校・健康問題なし
        let jobless_row = !has(worker) && !has(student) && !has(health);
        for i in 0..row.count {
            let c = row.start + i;
            let state = w.fixed(c, citizen).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
            // 年齢 = 2*AgeBit1 + AgeBit2（0:子ども 1:10代 2:大人 3:高齢者）。子どもと高齢者は仕事を探さない。
            let age = 2 * (state & 1) + ((state >> 1) & 1);
            let seeker = jobless_row && (age == 1 || age == 2);
            let household = w.u32_of(c, member, 0).unwrap_or(NULL);
            // 優先順: 引っ越し中 → 観光客 → 通勤者 → 家なし → 家のある住民
            let kind = if w.has(household, moving) {
                "引っ越し中の世帯"
            } else if state & 0x200 != 0 {
                "観光客"
            } else if state & 0x400 != 0 {
                "通勤者（街の外に住む）"
            } else if state & 0x4000 != 0 || !w.has(household, renter) {
                "家なし"
            } else {
                "家のある住民"
            };
            let e = counts.entry(kind).or_default();
            e.0 += 1;
            e.1 += u64::from(traveling);
            e.2 += u64::from(seeker);
        }
    }
    let mut list: Vec<_> = counts.into_iter().collect();
    list.sort_by_key(|e| std::cmp::Reverse(e.1.0));
    let mut out = String::new();
    for (kind, (n, t, s)) in list {
        writeln!(out, "{n:>9} 人（うち移動中 {t:>7}、失業中の求職対象 {s:>7}） {kind}").unwrap();
    }
    Ok(out)
}

/// 求人と失業者を学歴レベル別に比べる。
/// 空き求人は会社ごとの FreeWorkplaces（レベル0〜4の5バイト）、失業者は仕事・学校・健康問題のない10代・大人。
/// ゲームは学歴以下のレベルの空きに就ける（FreeWorkplaces.GetBestFor）。
pub fn job_market(input: &Path) -> Result<String, String> {
    let data = read_save_data(input)?;
    let w = World::load(&data)?;
    let free = w.id("Game.Companies.FreeWorkplaces")?;
    let citizen = w.id("Game.Citizens.Citizen")?;
    let member = w.id("Game.Citizens.HouseholdMember")?;
    let worker = w.id("Game.Citizens.Worker").ok();
    let student = w.id("Game.Citizens.Student").ok();
    let health = w.id("Game.Citizens.HealthProblem").ok();
    let mut vacant = [0u64; 5];
    let mut employers = 0u64;
    for row in w.rows.iter().filter(|r| r.types.contains(&free)) {
        for i in 0..row.count {
            if let Some(b) = w.fixed(row.start + i, free) {
                employers += 1;
                for l in 0..5 {
                    vacant[l] += u64::from(b[l]);
                }
            }
        }
    }
    // [居住者, 通勤者] × 学歴
    let mut jobless = [[0u64; 5]; 2];
    let mut workers = [0u64; 5];
    for row in &w.rows {
        if !(row.types.contains(&citizen) && row.types.contains(&member)) {
            continue;
        }
        let has = |t: Option<usize>| t.is_some_and(|t| row.types.contains(&t));
        for i in 0..row.count {
            let state = w.fixed(row.start + i, citizen).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
            let age = 2 * (state & 1) + ((state >> 1) & 1);
            let edu = if state & 0x40 != 0 { 4 } else { (if state & 0x10 != 0 { 2 } else { 0 }) + usize::from(state & 0x20 != 0) };
            if has(worker) {
                workers[edu] += 1;
            } else if !has(student) && !has(health) && (age == 1 || age == 2) {
                jobless[usize::from(state & 0x400 != 0)][edu] += 1;
            }
        }
    }
    // 空き求人と失業者の場所を 2km 四方の区画で比べる（事業所・世帯 → PropertyRenter → 建物の座標）。
    let renter = w.id("Game.Buildings.PropertyRenter")?;
    let transform = w.id("Game.Objects.Transform")?;
    let cell = |e: u32| -> Option<(i32, i32)> {
        // 会社・世帯は借りている建物の座標、公共施設などは自分自身の座標を使う
        let p = w
            .u32_of(e, renter, 0)
            .and_then(|b| w.position(b, transform))
            .or_else(|| w.position(e, transform))?;
        Some(((p[0] / 2000.0).floor() as i32, (p[2] / 2000.0).floor() as i32))
    };
    let mut grid: HashMap<(i32, i32), (u64, u64)> = HashMap::new();
    for row in w.rows.iter().filter(|r| r.types.contains(&free)) {
        for i in 0..row.count {
            let e = row.start + i;
            if let (Some(b), Some(c)) = (w.fixed(e, free), cell(e)) {
                grid.entry(c).or_default().0 += b.iter().map(|&x| u64::from(x)).sum::<u64>();
            }
        }
    }
    for row in &w.rows {
        if !(row.types.contains(&citizen) && row.types.contains(&member)) {
            continue;
        }
        let has = |t: Option<usize>| t.is_some_and(|t| row.types.contains(&t));
        if has(worker) || has(student) || has(health) {
            continue;
        }
        for i in 0..row.count {
            let c = row.start + i;
            let state = w.fixed(c, citizen).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
            let age = 2 * (state & 1) + ((state >> 1) & 1);
            if (age == 1 || age == 2)
                && state & 0x400 == 0
                && let Some(cl) = w.u32_of(c, member, 0).and_then(cell)
            {
                grid.entry(cl).or_default().1 += 1;
            }
        }
    }
    let mut cells: Vec<_> = grid.into_iter().filter(|(_, v)| v.0 + v.1 > 0).collect();
    cells.sort_by_key(|(k, _)| (k.1, k.0));
    let mut out = String::from("== 2km区画ごとの 空き求人 / 失業者（居住）\n");
    for ((x, z), (v, u)) in &cells {
        writeln!(out, "区画({x:>3},{z:>3})  空き {v:>6}  失業 {u:>6}").unwrap();
    }
    writeln!(out, "空き求人のある事業所（FreeWorkplaces を持つ） {employers}").unwrap();
    out.push_str("学歴/職場レベル  空き求人  失業(居住)  失業(通勤者)  就業者\n");
    for l in 0..5 {
        writeln!(
            out,
            "{l} {:>16} {:>11} {:>13} {:>8}",
            vacant[l], jobless[0][l], jobless[1][l], workers[l]
        )
        .unwrap();
    }
    writeln!(
        out,
        "合計 {:>13} {:>11} {:>13} {:>8}",
        vacant.iter().sum::<u64>(),
        jobless[0].iter().sum::<u64>(),
        jobless[1].iter().sum::<u64>(),
        workers.iter().sum::<u64>()
    )
    .unwrap();
    Ok(out)
}

/// 経路探しの依頼元を、保存時点の状態で種類別に数える。
/// 移動する物は PathOwner（int + u16 の状態）、探し物をする主体（世帯・会社・求職者・配車依頼など）は
/// PathInformation（24バイト、末尾2バイトが状態）を持つ。状態の Pending(1) は経路待ち、
/// Obsolete(4) は経路の作り直し待ち、Failed(2) は失敗。保存ファイル上は Pending のまま残る。
pub fn path_demand(input: &Path) -> Result<String, String> {
    let data = read_save_data(input)?;
    let w = World::load(&data)?;
    let owner = w.id("Game.Pathfind.PathOwner")?;
    let info = w.id("Game.Pathfind.PathInformation")?;
    // 行の種類を表す型。上から順に最初に当たったものを使う。
    const LABELS: &[&str] = &[
        "Game.Vehicles.PersonalCar",
        "Game.Vehicles.Taxi",
        "Game.Vehicles.PublicTransport",
        "Game.Vehicles.CargoTransport",
        "Game.Vehicles.DeliveryTruck",
        "Game.Vehicles.GarbageTruck",
        "Game.Vehicles.Ambulance",
        "Game.Vehicles.Hearse",
        "Game.Vehicles.PoliceCar",
        "Game.Vehicles.FireEngine",
        "Game.Vehicles.MaintenanceVehicle",
        "Game.Vehicles.PostVan",
        "Game.Vehicles.WorkVehicle",
        "Game.Vehicles.PrisonerTransport",
        "Game.Vehicles.EvacuatingTransport",
        "Game.Creatures.Resident",
        "Game.Creatures.Pet",
        "Game.Creatures.Wildlife",
        "Game.Creatures.Domesticated",
        "Game.Citizens.Household",
        "Game.Citizens.Citizen",
        "Game.Companies.CompanyData",
        "Game.Buildings.Building",
        "Game.Vehicles.Vehicle",
    ];
    let label_ids: Vec<(usize, &str)> = LABELS
        .iter()
        .filter_map(|n| w.names.get(*n).map(|&id| (id, *n)))
        .collect();
    // 種類 → [件数, 経路待ち, 作り直し待ち, 失敗]
    let mut table: HashMap<(String, &str), [u64; 4]> = HashMap::new();
    for row in &w.rows {
        for (ty, kind, offset) in [(owner, "移動", 4usize), (info, "探索", 22usize)] {
            if !row.types.contains(&ty) || row.count == 0 {
                continue;
            }
            let label = row
                .types
                .iter()
                .find(|&&t| w.types[t].name.ends_with("Request"))
                .map(|&t| w.types[t].name.clone())
                .or_else(|| {
                    label_ids
                        .iter()
                        .find(|(id, _)| row.types.contains(id))
                        .map(|(_, n)| n.to_string())
                })
                .unwrap_or_else(|| {
                    row.types
                        .iter()
                        .map(|&t| w.types[t].name.as_str())
                        .find(|n| !n.starts_with("Game.Prefabs") && !n.starts_with("Game.Pathfind"))
                        .unwrap_or("?")
                        .to_string()
                });
            let e = table.entry((label, kind)).or_default();
            for i in 0..row.count {
                let state = w
                    .fixed(row.start + i, ty)
                    .map_or(0, |b| u16::from_le_bytes([b[offset], b[offset + 1]]));
                e[0] += 1;
                e[1] += u64::from(state & 1 != 0);
                e[2] += u64::from(state & 4 != 0);
                e[3] += u64::from(state & 2 != 0);
            }
        }
    }
    let mut rows: Vec<_> = table.into_iter().collect();
    rows.sort_by_key(|(_, v)| std::cmp::Reverse(v[1] + v[2]));
    let mut out = String::from("  経路待ち  作り直し待ち    失敗      件数  種類\n");
    for ((label, kind), v) in rows.iter().take(40) {
        writeln!(out, "{:>10} {:>12} {:>7} {:>9}  [{kind}] {label}", v[1], v[2], v[3], v[0]).unwrap();
    }
    let total: [u64; 2] = rows.iter().fold([0, 0], |a, (_, v)| [a[0] + v[1], a[1] + v[2]]);
    writeln!(out, "合計: 経路待ち {} / 作り直し待ち {}", total[0], total[1]).unwrap();
    Ok(out)
}

/// 調査用：型の組み合わせごとの実体数を数える。
/// 各指定は `A+B-C` の形で、A と B を持ち C を持たない実体を数える。
pub fn count_entities(input: &Path, specs: &[&str]) -> Result<String, String> {
    let data = read_save_data(input)?;
    let w = World::load(&data)?;
    let mut out = String::new();
    for spec in specs {
        let mut need = Vec::new();
        let mut deny = Vec::new();
        // 先頭は必ず「持つ」側。以降は + / - で区切る。
        let mut rest = *spec;
        let mut include = true;
        loop {
            let end = rest.find(['+', '-']).unwrap_or(rest.len());
            let id = w.names.get(&rest[..end]).copied();
            if include {
                need.push(id);
            } else {
                deny.push(id);
            }
            if end == rest.len() {
                break;
            }
            include = &rest[end..end + 1] == "+";
            rest = &rest[end + 1..];
        }
        let total: u64 = w
            .rows
            .iter()
            .filter(|r| need.iter().all(|n| n.is_some_and(|n| r.types.contains(&n))))
            .filter(|r| deny.iter().all(|d| d.is_none_or(|d| !r.types.contains(&d))))
            .map(|r| u64::from(r.count))
            .sum();
        // 先頭の型が有効・無効を持つ型（形式17・18）なら、有効な件数も示す。
        let enabled = need[0]
            .filter(|&id| matches!(w.types[id].flag, 17 | 18))
            .map(|id| {
                w.rows
                    .iter()
                    .filter(|r| need.iter().all(|n| n.is_some_and(|n| r.types.contains(&n))))
                    .filter(|r| deny.iter().all(|d| d.is_none_or(|d| !r.types.contains(&d))))
                    .map(|r| (0..r.count).filter(|&i| w.enabled(r.start + i, id) == Some(true)).count() as u64)
                    .sum::<u64>()
            });
        match enabled {
            Some(e) => writeln!(out, "{total:>10} {spec}（有効 {e}）").unwrap(),
            None => writeln!(out, "{total:>10} {spec}").unwrap(),
        }
    }
    Ok(out)
}

/// 調査用：指定したコンポーネントを持つ行について、各コンポーネントの形式とレコード長を表示する。
pub fn probe_components(input: &Path, anchor: &str, names: &[&str]) -> Result<String, String> {
    let data = read_save_data(input)?;
    let w = World::load(&data)?;
    let mut out = String::new();
    for name in names {
        match w.names.get(*name) {
            Some(&id) => writeln!(out, "type {name} id={id} flag={}", w.types[id].flag).unwrap(),
            None => writeln!(out, "type {name} なし").unwrap(),
        }
    }
    let anchor = w.names.get(anchor).copied();
    for row in w
        .rows
        .iter()
        .filter(|r| r.count != 0 && anchor.is_some_and(|a| r.types.contains(&a)))
        .take(6)
    {
        writeln!(out, "row start={} count={}", row.start, row.count).unwrap();
        for &id in &row.types {
            let Some(block) = row.blocks.get(&id) else {
                writeln!(out, "  {} (tag)", w.types[id].name).unwrap();
                continue;
            };
            let block = &row.frame[block.clone()];
            writeln!(
                out,
                "  {} flag={} len={} per={:.2} head={:02x?}",
                w.types[id].name,
                w.types[id].flag,
                block.len(),
                block.len() as f64 / f64::from(row.count),
                &block[..block.len().min(24)]
            )
            .unwrap();
        }
    }
    Ok(out)
}
