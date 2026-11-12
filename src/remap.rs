use std::ops::Range;

const NULL_ENTITY: i32 = -1;

/// アーキタイプ順の旧IDを、削除後の連続したIDに対応付ける。
pub struct EntityRemap {
    runs: Vec<Run>,
    remaining: i32,
}

struct Run {
    old: Range<i32>,
    new_start: Option<i32>,
}

impl EntityRemap {
    /// 同じアーキタイプ内でも、選別された個体だけを削除できるようにする。
    pub fn from_removed_ids(total: u32, removed: &[u32]) -> Result<Self, String> {
        let mut cursor = 0;
        let mut segments = Vec::new();
        for &id in removed {
            if id < cursor || id >= total {
                return Err("削除対象IDが昇順でないか、保存IDの範囲外です".into());
            }
            segments.push((id - cursor, false));
            segments.push((1, true));
            cursor = id + 1;
        }
        segments.push((total - cursor, false));
        Self::from_archetypes(segments)
    }

    /// 各入力は元のアーキタイプの件数と、そのアーキタイプを削除するかどうか。
    pub fn from_archetypes(
        archetypes: impl IntoIterator<Item = (u32, bool)>,
    ) -> Result<Self, String> {
        let mut runs = Vec::new();
        let mut start = 0i32;
        let mut remaining = 0i32;
        for (count, removed) in archetypes {
            if count == 0 {
                continue;
            }
            let count = i32::try_from(count).map_err(|_| "保存IDの範囲を超えています")?;
            let end = start
                .checked_add(count)
                .ok_or("保存IDの範囲を超えています")?;
            runs.push(Run {
                old: start..end,
                new_start: (!removed).then_some(remaining),
            });
            if !removed {
                remaining += count;
            }
            start = end;
        }
        Ok(Self { runs, remaining })
    }

    pub fn remaining(&self) -> i32 {
        self.remaining
    }

    pub fn resolve(&self, old_id: i32) -> Result<i32, String> {
        if old_id == NULL_ENTITY {
            return Ok(NULL_ENTITY);
        }
        let index = self.runs.partition_point(|run| run.old.end <= old_id);
        let run = self
            .runs
            .get(index)
            .filter(|run| run.old.contains(&old_id))
            .ok_or_else(|| format!("参照先の保存IDが範囲外です ({old_id})"))?;
        Ok(run
            .new_start
            .map_or(NULL_ENTITY, |start| start + (old_id - run.old.start)))
    }

    /// 型定義で確認した参照フィールドだけを変更する。
    /// 参照が途中で不正と分かっても、呼び出し元のデータには書き込まない。
    pub fn rewrite(&self, payload: &[u8], layout: ReferenceLayout) -> Result<Vec<u8>, String> {
        if let ReferenceLayout::ComponentBuffer { entities, element } = layout {
            return self.rewrite_component_buffer(payload, entities, element);
        }
        if matches!(
            layout,
            ReferenceLayout::CityStatisticsEvents | ReferenceLayout::LegacyCityStatisticsEntities
        ) {
            return self.rewrite_statistics(payload, layout);
        }
        if matches!(layout, ReferenceLayout::ChirpLinkSystem) {
            return self.rewrite_chirps(payload);
        }
        if matches!(
            layout,
            ReferenceLayout::ClimateSystem | ReferenceLayout::LegacyCityConfigurationSystem
        ) {
            return self.rewrite_configuration(payload, layout);
        }
        let (start, stride, count) = layout.records(payload)?;
        let mut output = payload.to_vec();
        for index in 0..count {
            self.rewrite_id(&mut output[start + index * stride..][..4], false)?;
        }
        Ok(output)
    }

    fn rewrite_component_buffer(
        &self,
        payload: &[u8],
        entities: usize,
        element: ReferenceBufferElement,
    ) -> Result<Vec<u8>, String> {
        let mut output = payload.to_vec();
        let mut rest = output.as_mut_slice();
        for _ in 0..entities {
            let count = read_count(take(&mut rest, 4)?)?;
            for _ in 0..count {
                if matches!(element, ReferenceBufferElement::StorageTransferRequest) {
                    take(&mut rest, 6)?; // 資源区分と数量
                }
                self.rewrite_id(take(&mut rest, 4)?, false)?;
                match element {
                    ReferenceBufferElement::Entity
                    | ReferenceBufferElement::StorageTransferRequest => {}
                    ReferenceBufferElement::NavigationLane => {
                        take(&mut rest, 12)?; // 曲線座標2つとフラグ
                    }
                    ReferenceBufferElement::BlockedLane => {
                        take(&mut rest, 8)?; // 曲線座標2つ
                    }
                    ReferenceBufferElement::Queue => {
                        take(&mut rest, 2)?; // 有効期限
                        let radius = f32::from_le_bytes(take(&mut rest, 4)?.try_into().unwrap());
                        if radius > 0.0 {
                            take(&mut rest, 12)?; // 球の中心座標
                        }
                    }
                    ReferenceBufferElement::Path => {
                        let packed = take(&mut rest, 1)?[0];
                        for part in [packed & 15, packed >> 4] {
                            match part {
                                0 | 1 => {}
                                2 => {
                                    take(&mut rest, 4)?;
                                } // 0・1以外の曲線座標
                                _ => return Err("経路の座標表現が不正です".into()),
                            }
                        }
                        take(&mut rest, 1)?; // PathElementFlags
                    }
                    ReferenceBufferElement::Policy => {
                        take(&mut rest, 5)?;
                    }
                    ReferenceBufferElement::EntityWithValue => {
                        take(&mut rest, 4)?;
                    }
                    ReferenceBufferElement::Employee => {
                        take(&mut rest, 1)?;
                    }
                    ReferenceBufferElement::LegacyServiceBudget => {
                        take(&mut rest, 8)?;
                    }
                    ReferenceBufferElement::LegacyTrip => {
                        take(&mut rest, 6)?;
                    }
                    ReferenceBufferElement::EntityPair
                    | ReferenceBufferElement::PathInformation => {
                        self.rewrite_id(take(&mut rest, 4)?, false)?;
                        if matches!(element, ReferenceBufferElement::PathInformation) {
                            take(&mut rest, 16)?; // 距離・所要時間・費用・経路フラグ
                        }
                    }
                }
            }
        }
        require_consumed(rest)?;
        Ok(output)
    }

    fn rewrite_statistics(
        &self,
        payload: &[u8],
        layout: ReferenceLayout,
    ) -> Result<Vec<u8>, String> {
        ReferenceLayout::check_system_length(payload)?;
        let mut output = payload.to_vec();
        let mut rest = &mut output[4..];
        if matches!(layout, ReferenceLayout::CityStatisticsEvents) {
            // 繁華街では件数、直近のサンプルフレーム、イベント配列が並ぶ。
            take(&mut rest, 8)?;
            let count = read_count(take(&mut rest, 4)?)?;
            // StatisticsEvent は統計の種類・分類値・変化量の3つ。保存IDは持たない。
            take_records(&mut rest, count, 12)?;
        } else {
            // statisticsRefactor より古い分岐だけにある統計用エンティティ。
            for _ in 0..3 {
                self.rewrite_id(take(&mut rest, 4)?, false)?;
            }
            let count = read_count(take(&mut rest, 4)?)?;
            take_records(&mut rest, count, 8)?;
            let count = read_count(take(&mut rest, 4)?)?;
            for field in take_records(&mut rest, count, 4)?.chunks_exact_mut(4) {
                self.rewrite_id(field, false)?;
            }
            take(&mut rest, 8)?;
        }
        require_consumed(rest)?;
        Ok(output)
    }

    fn rewrite_id(&self, field: &mut [u8], dictionary_key: bool) -> Result<(), String> {
        let old = i32::from_le_bytes(field.try_into().unwrap());
        let new = self.resolve(old)?;
        // ChirpLinkSystem は空のキーを読み飛ばさない。該当時は項目そのものの削除が必要。
        if dictionary_key && old != NULL_ENTITY && new == NULL_ENTITY {
            return Err("削除対象がチャープ辞書のキーになっています".into());
        }
        field.copy_from_slice(&new.to_le_bytes());
        Ok(())
    }

    fn rewrite_chirps(&self, payload: &[u8]) -> Result<Vec<u8>, String> {
        ReferenceLayout::check_system_length(payload)?;
        let mut output = payload.to_vec();
        let mut rest = &mut output[4..];
        let count = read_count(take(&mut rest, 4)?)?;
        for _ in 0..count {
            self.rewrite_id(take(&mut rest, 4)?, true)?;
            self.rewrite_cached_name(&mut rest)?;
            let links = read_count(take(&mut rest, 4)?)?;
            for _ in 0..links {
                self.rewrite_cached_name(&mut rest)?;
            }
        }
        if !rest.is_empty() {
            return Err("チャープの末尾に未解釈のデータがあります".into());
        }
        Ok(output)
    }

    fn rewrite_cached_name(&self, rest: &mut &mut [u8]) -> Result<(), String> {
        self.rewrite_id(take(rest, 4)?, false)?;
        take(rest, 4)?; // NameType
        skip_string(rest)?; // NameID
        let args = read_count(take(rest, 4)?)?;
        for _ in 0..args {
            skip_string(rest)?;
        }
        Ok(())
    }

    fn rewrite_configuration(
        &self,
        payload: &[u8],
        layout: ReferenceLayout,
    ) -> Result<Vec<u8>, String> {
        ReferenceLayout::check_system_length(payload)?;
        let mut output = payload.to_vec();
        let mut rest = &mut output[4..];
        if matches!(layout, ReferenceLayout::ClimateSystem) {
            self.rewrite_id(take(&mut rest, 4)?, false)?;
            for _ in 0..2 {
                let count = read_count(take(&mut rest, 4)?)?;
                for _ in 0..count {
                    self.rewrite_id(take(&mut rest, 4)?, false)?;
                }
            }
            take(&mut rest, 4)?; // 気温の基準高度
        } else {
            // 繁華街の形式。必要コンテンツ一覧と保存Mod一覧が追加される前のレイアウト。
            skip_string(&mut rest)?;
            self.rewrite_id(take(&mut rest, 4)?, false)?; // テーマ
            take(&mut rest, 26)?; // 左側通行・災害とカメラの座標・向き・ズーム
            self.rewrite_id(take(&mut rest, 4)?, false)?; // カメラの追跡対象
            take(&mut rest, 2)?; // 資金・解放オプション
        }
        if !rest.is_empty() {
            return Err("システムブロックの末尾に未解釈のデータがあります".into());
        }
        Ok(output)
    }
}

fn take<'a>(rest: &mut &'a mut [u8], size: usize) -> Result<&'a mut [u8], String> {
    if size > rest.len() {
        return Err("参照ブロックが途中で終わっています".into());
    }
    let (head, tail) = std::mem::take(rest).split_at_mut(size);
    *rest = tail;
    Ok(head)
}

fn take_records<'a>(
    rest: &mut &'a mut [u8],
    count: usize,
    stride: usize,
) -> Result<&'a mut [u8], String> {
    let size = count
        .checked_mul(stride)
        .ok_or("レコードの長さが範囲外です")?;
    take(rest, size)
}

fn require_consumed(rest: &[u8]) -> Result<(), String> {
    if rest.is_empty() {
        Ok(())
    } else {
        Err("参照ブロックの末尾に未解釈のデータがあります".into())
    }
}

fn skip_string(rest: &mut &mut [u8]) -> Result<(), String> {
    // ゲームはバイト長ではなく、BinaryReader.Read(char) の呼び出し回数を書く。
    let chars = read_count(take(rest, 4)?)?;
    for _ in 0..chars {
        let lead = take(rest, 1)?[0];
        let continuations = match lead {
            0..=0x7f => 0,
            0xc0..=0xdf => 1,
            0xe0..=0xef => 2,
            0xf0..=0xf7 => 3,
            _ => return Err("文字列の先頭バイトが不正です".into()),
        };
        if take(rest, continuations)?
            .iter()
            .any(|byte| byte & 0xc0 != 0x80)
        {
            return Err("文字列の継続バイトが不正です".into());
        }
    }
    Ok(())
}

/// 「繁華街」のバッファと、インストール済みゲームのシリアライザーの両方で確認した配置。
pub enum ReferenceLayout {
    CurrentVehicle {
        entities: usize,
    },
    CurrentTransport {
        entities: usize,
    },
    CurrentBuilding {
        entities: usize,
    },
    HouseholdMember {
        entities: usize,
    },
    Owner {
        entities: usize,
    },
    LegacyTransport {
        entities: usize,
    },
    CitySystem,
    TriggerSystem,
    ChirpLinkSystem,
    MarkerIconSystem,
    ElectricityFlowSystem,
    WaterPipeFlowSystem,
    MapTileSystem,
    RoutePathSystem,
    ClimateSystem,
    LegacyCityConfigurationSystem,
    CityStatisticsEvents,
    LegacyCityStatisticsEntities,
    ComponentBuffer {
        entities: usize,
        element: ReferenceBufferElement,
    },
}

/// 要素数に続く参照と付随データの配置を、ゲームのシリアライザーと照合したもの。
pub enum ReferenceBufferElement {
    Entity,
    EntityPair,
    EntityWithValue,
    NavigationLane,
    BlockedLane,
    Queue,
    Path,
    PathInformation,
    Policy,
    Employee,
    LegacyServiceBudget,
    LegacyTrip,
    StorageTransferRequest,
}

impl ReferenceBufferElement {
    /// 繁華街で使用される旧形式。型IDはファイル内の型名から引く。
    pub fn for_beartris(name: &str) -> Option<Self> {
        Some(match name {
            "Game.Vehicles.CarNavigationLane"
            | "Game.Vehicles.WatercraftNavigationLane"
            | "Game.Vehicles.TrainNavigationLane"
            | "Game.Vehicles.AircraftNavigationLane" => Self::NavigationLane,
            "Game.Vehicles.LayoutElement"
            | "Game.Triggers.ChirpEntity"
            | "Game.Triggers.ChirpLink"
            | "Game.Triggers.LifePathEntry"
            | "Game.Routes.DispatchedRequest"
            | "Game.Net.AggregateElement"
            | "Game.Events.TargetElement"
            | "Game.Buildings.Patient"
            | "Game.Buildings.Occupant"
            | "Game.Simulation.ServiceDispatch"
            | "Game.Areas.ServiceDistrict"
            | "Game.Areas.WoodResource"
            | "Game.Citizens.CoordinatedMeetingAttendee" => Self::Entity,
            "Game.Citizens.PetTrip" => Self::EntityPair,
            "Game.Net.ConnectedNode" | "Game.Citizens.LookingForPartner" => Self::EntityWithValue,
            "Game.Pathfind.PathElement" => Self::Path,
            "Game.Pathfind.PathInformations" => Self::PathInformation,
            "Game.Objects.BlockedLane" => Self::BlockedLane,
            "Game.Creatures.Queue" => Self::Queue,
            "Game.Policies.Policy" => Self::Policy,
            "Game.Companies.Employee" => Self::Employee,
            "Game.Companies.StorageTransferRequest" => Self::StorageTransferRequest,
            "Game.Simulation.ServiceBudgetData" => Self::LegacyServiceBudget,
            "Game.Citizens.TripNeeded" => Self::LegacyTrip,
            _ => return None,
        })
    }
}

impl ReferenceLayout {
    fn records(self, payload: &[u8]) -> Result<(usize, usize, usize), String> {
        let (start, stride, count) = match self {
            Self::CurrentVehicle { entities } => (0, 8, entities),
            Self::LegacyTransport { entities } => (0, 20, entities),
            Self::CurrentTransport { entities }
            | Self::CurrentBuilding { entities }
            | Self::HouseholdMember { entities }
            | Self::Owner { entities } => (0, 4, entities),
            Self::CitySystem => {
                Self::check_system_length(payload)?;
                (4, 4, 1)
            }
            Self::TriggerSystem | Self::MapTileSystem | Self::RoutePathSystem => {
                Self::check_system_length(payload)?;
                let count = read_count(payload.get(4..).unwrap_or_default())?;
                let stride = if matches!(self, Self::TriggerSystem) {
                    8
                } else {
                    4
                };
                (8, stride, count)
            }
            Self::MarkerIconSystem => (4, 4, 2),
            Self::ElectricityFlowSystem => return Self::utility_records(payload, 16),
            Self::WaterPipeFlowSystem => return Self::utility_records(payload, 20),
            Self::ChirpLinkSystem
            | Self::ClimateSystem
            | Self::LegacyCityConfigurationSystem
            | Self::CityStatisticsEvents
            | Self::LegacyCityStatisticsEntities
            | Self::ComponentBuffer { .. } => {
                unreachable!("可変長レコードは専用の処理で書き換える")
            }
        };
        let size = count
            .checked_mul(stride)
            .and_then(|bytes| bytes.checked_add(start));
        if size != Some(payload.len()) {
            return Err("参照レコードの件数とバイト数が一致しません".into());
        }
        if matches!(self, Self::MarkerIconSystem) {
            Self::check_system_length(payload)?;
        }
        Ok((start, stride, count))
    }

    fn utility_records(
        payload: &[u8],
        expected_size: usize,
    ) -> Result<(usize, usize, usize), String> {
        Self::check_system_length(payload)?;
        if payload.len() != expected_size {
            return Err("公共設備システムのバージョンに対応していません".into());
        }
        Ok((4, 4, 2))
    }

    fn check_system_length(payload: &[u8]) -> Result<(), String> {
        let stored = read_count(payload)?;
        if Some(stored) != payload.len().checked_sub(4) {
            return Err("システムブロックの長さが一致しません".into());
        }
        Ok(())
    }
}

fn read_count(data: &[u8]) -> Result<usize, String> {
    let bytes = data.get(..4).ok_or("レコードの長さ欄がありません")?;
    usize::try_from(i32::from_le_bytes(bytes.try_into().unwrap()))
        .map_err(|_| "負のレコード長または件数です".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaps_only_entity_fields_in_component_and_system_blocks() {
        let remap =
            EntityRemap::from_archetypes([(2, false), (2, true), (0, true), (2, false)]).unwrap();
        assert_eq!(remap.remaining(), 4);
        assert_eq!(remap.resolve(1), Ok(1));
        assert_eq!(remap.resolve(2), Ok(-1));
        assert_eq!(remap.resolve(4), Ok(2));
        assert_eq!(remap.resolve(-1), Ok(-1));
        assert!(remap.resolve(6).is_err());
        let bytes = |values: &[i32]| {
            values
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>()
        };
        let source = bytes(&[3, 3, 5, 5]);
        assert_eq!(
            remap
                .rewrite(&source, ReferenceLayout::CurrentVehicle { entities: 2 })
                .unwrap(),
            bytes(&[-1, 3, 3, 5])
        );
        assert_eq!(
            remap
                .rewrite(&bytes(&[20, 2, 2, 2, 5, 5]), ReferenceLayout::TriggerSystem)
                .unwrap(),
            bytes(&[20, 2, -1, 2, 3, 5])
        );
        assert!(
            remap
                .rewrite(&source, ReferenceLayout::CurrentVehicle { entities: 1 })
                .is_err()
        );
        assert!(
            remap
                .rewrite(&bytes(&[4, 6]), ReferenceLayout::CitySystem)
                .is_err()
        );
        // 旧統計システムでは固定の3参照と配列だけがID。後続の統計値は同じ数値でも維持する。
        assert_eq!(
            remap
                .rewrite(
                    &bytes(&[44, 1, 2, 5, 1, 2, 5, 2, 3, 4, 4, 5]),
                    ReferenceLayout::LegacyCityStatisticsEntities,
                )
                .unwrap(),
            bytes(&[44, 1, -1, 3, 1, 2, 5, 2, -1, 2, 4, 5])
        );
    }
}
