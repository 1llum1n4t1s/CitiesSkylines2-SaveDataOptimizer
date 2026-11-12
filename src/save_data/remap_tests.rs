use super::*;
use crate::capture::{CaptureReader, CapturedReference, FrameKey};
use crate::remap::{EntityRemap, ReferenceBufferElement, ReferenceLayout};
use crate::vehicle_selection::{LegacyTransportKind, dummy_traffic_candidates};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Cursor, Seek, SeekFrom};
use zip::ZipArchive;

#[test]
fn capture_covers_every_known_beartris_vehicle_reference() {
    let (Some(path), Some(trace)) = (
        std::env::var_os("COK_TEST_SAMPLE"),
        std::env::var_os("COK_TEST_REFERENCE_CAPTURE"),
    ) else {
        return;
    };
    let mut zip = ZipArchive::new(File::open(path).unwrap()).unwrap();
    let name = crate::archive::find_entry_name(&zip, ".SaveGameData").unwrap();
    let mut reader = BufReader::new(zip.by_name(&name).unwrap());
    let header = find_first_frame(&mut reader).unwrap();
    let types = parse_types(&read_frame(&mut reader, header, MAX_TABLE).unwrap()).unwrap();
    validate_systems(&read_next_frame(&mut reader, MAX_TABLE).unwrap()).unwrap();
    let rows = parse_archetypes(&read_next_frame(&mut reader, MAX_TABLE).unwrap(), &types).unwrap();
    let mut missing = HashSet::new();
    for row in &rows {
        if !row.types.iter().any(|&id| types[id].reference == Some(0)) {
            skip_frame(&mut reader).unwrap();
            continue;
        }
        let frame = read_next_frame(&mut reader, MAX_FRAME).unwrap();
        let key = FrameKey::for_payload(&frame).unwrap();
        let mut rest = frame.as_slice();
        for &id in &row.types {
            if types[id].flag == 1 {
                continue;
            }
            let offset = frame.len() - rest.len() + 4;
            let block = sized_bytes(&mut rest).unwrap();
            if types[id].reference == Some(0) {
                for (index, vehicle) in block.chunks_exact(8).enumerate() {
                    let id = i32::from_le_bytes(vehicle[..4].try_into().unwrap());
                    let item = CapturedReference {
                        frame: key,
                        offset: (offset + index * 8) as u32,
                        id,
                    };
                    assert!(item.matches_payload(&frame));
                    assert!(missing.insert(item));
                }
            }
        }
    }
    assert_eq!(missing.len(), 7104);
    let mut captured = CaptureReader::new(BufReader::new(File::open(trace).unwrap())).unwrap();
    let mut count = 0;
    while let Some(reference) = captured.next_reference().unwrap() {
        count += 1;
        missing.remove(&reference);
    }
    println!(
        "captured {count} references; vehicle matches {}/7104",
        7104 - missing.len()
    );
    assert!(
        missing.is_empty(),
        "未捕捉の CurrentVehicle: {}",
        missing.len()
    );
}

fn names(mut data: &[u8], component_table: bool) -> Vec<&str> {
    let count = uint(&mut data).unwrap();
    (0..count)
        .map(|_| {
            let mut record = sized_bytes(&mut data).unwrap();
            if component_table {
                bytes(&mut record, 1).unwrap();
            }
            let name = std::str::from_utf8(sized_bytes(&mut record).unwrap()).unwrap();
            name.split_once(',').unwrap().0
        })
        .collect()
}

#[test]
fn verified_layouts_remap_real_beartris_references() {
    let Some(path) = std::env::var_os("COK_TEST_SAMPLE") else {
        return;
    };
    let mut zip = ZipArchive::new(File::open(path).unwrap()).unwrap();
    let index = crate::archive::find_entry_name(&zip, ".SaveGameData").unwrap();
    let mut data = Vec::new();
    zip.by_name(&index).unwrap().read_to_end(&mut data).unwrap();
    let mut reader = BufReader::new(Cursor::new(&data));
    let header = find_first_frame(&mut reader).unwrap();
    let table = read_frame(&mut reader, header, MAX_TABLE).unwrap();
    let types = parse_types(&table).unwrap();
    let component_names = names(&table, true);
    let systems = read_next_frame(&mut reader, MAX_TABLE).unwrap();
    let system_names = names(&systems, false);
    let rows = parse_archetypes(&read_next_frame(&mut reader, MAX_TABLE).unwrap(), &types).unwrap();
    let household = component_names
        .iter()
        .position(|name| *name == "Game.Citizens.Household")
        .unwrap();
    let component_start = reader.stream_position().unwrap();
    let mut candidate_ids = Vec::new();
    for row in &rows {
        if row.kinds & 1 != 0 {
            let frame = read_next_frame(&mut reader, MAX_FRAME).unwrap();
            find_dummy_traffic(
                &frame,
                row,
                &rows,
                &types,
                &component_names,
                &mut candidate_ids,
            );
        } else {
            skip_frame(&mut reader).unwrap();
        }
    }
    assert_eq!(
        candidate_ids,
        [494_765, 795_274, 870_195, 870_196, 870_197, 870_198]
    );
    let mut selected: Vec<u32> = candidate_ids.iter().map(|&id| id as u32).collect();
    for row in rows.iter().filter(|row| row.kinds & 0b0100 != 0) {
        let start = row.start as u32;
        selected.extend(start..start + row.count);
    }
    selected.sort_unstable();
    let remap = EntityRemap::from_removed_ids(986_964, &selected).unwrap();
    assert_eq!(remap.remaining(), 969_119);
    assert_eq!(remap.resolve(494_765), Ok(-1));
    reader.seek(SeekFrom::Start(component_start)).unwrap();

    let mut citizens = 0;
    let mut households = 0;
    let mut cleared_transports = 0;
    let mut changed_members = 0;
    for row in &rows {
        let start = row.start as u32;
        let first = selected.partition_point(|&id| id < start);
        let last = selected.partition_point(|&id| id < start + row.count);
        if first != last {
            assert_eq!(row.kinds & 0b1000, 0);
            assert!(!row.types.contains(&household));
        }
        if row.kinds & 0b0100 != 0 {
            skip_frame(&mut reader).unwrap();
            continue;
        }
        if row.kinds & 0b1000 != 0 {
            citizens += row.count;
        }
        if row.types.contains(&household) {
            households += row.count;
        }
        let frame = read_next_frame(&mut reader, MAX_FRAME).unwrap();
        let mut payload = frame.as_slice();
        for &id in &row.types {
            if types[id].flag == 1 {
                continue;
            }
            let block = sized_bytes(&mut payload).unwrap();
            let entities = row.count as usize;
            let layout = match component_names[id] {
                "Game.Creatures.CurrentVehicle" => ReferenceLayout::CurrentVehicle { entities },
                "Game.Citizens.CurrentTransport" => ReferenceLayout::CurrentTransport { entities },
                "Game.Citizens.CurrentBuilding" => ReferenceLayout::CurrentBuilding { entities },
                "Game.Citizens.HouseholdMember" => ReferenceLayout::HouseholdMember { entities },
                "Game.Common.Owner" => ReferenceLayout::Owner { entities },
                "Game.Vehicles.PublicTransport" | "Game.Vehicles.CargoTransport"
                    if block.len() == 20 * entities =>
                {
                    ReferenceLayout::LegacyTransport { entities }
                }
                name => {
                    let Some(element) = ReferenceBufferElement::for_beartris(name) else {
                        continue;
                    };
                    assert_eq!(types[id].flag, 4);
                    ReferenceLayout::ComponentBuffer { entities, element }
                }
            };
            let output = remap
                .rewrite(block, layout)
                .unwrap_or_else(|error| panic!("{}: {error}", component_names[id]));
            for (before, after) in block.chunks_exact(4).zip(output.chunks_exact(4)) {
                if before != after {
                    if component_names[id] == "Game.Citizens.CurrentTransport" && after == [255; 4]
                    {
                        cleared_transports += 1;
                    }
                    if component_names[id] == "Game.Citizens.HouseholdMember" {
                        assert_ne!(after, [255; 4]);
                        changed_members += 1;
                    }
                }
            }
        }
        require_end(payload).unwrap();
    }
    assert_eq!((citizens, households), (73_445, 40_807));
    assert_eq!(cleared_transports, 17_837);
    assert!(changed_members > 0);
    assert_eq!(system_names.len(), 63);
    for name in system_names {
        let layout = match name {
            "Game.Simulation.CitySystem" => ReferenceLayout::CitySystem,
            "Game.Triggers.TriggerSystem" => ReferenceLayout::TriggerSystem,
            "Game.UI.InGame.ChirpLinkSystem" => ReferenceLayout::ChirpLinkSystem,
            "Game.Rendering.MarkerIconSystem" => ReferenceLayout::MarkerIconSystem,
            "Game.Simulation.ElectricityFlowSystem" => ReferenceLayout::ElectricityFlowSystem,
            "Game.Simulation.WaterPipeFlowSystem" => ReferenceLayout::WaterPipeFlowSystem,
            "Game.Areas.MapTileSystem" => ReferenceLayout::MapTileSystem,
            "Game.Routes.RoutePathSystem" => ReferenceLayout::RoutePathSystem,
            "Game.Simulation.ClimateSystem" => ReferenceLayout::ClimateSystem,
            "Game.City.CityConfigurationSystem" => ReferenceLayout::LegacyCityConfigurationSystem,
            "Game.Simulation.CityStatisticsSystem" => ReferenceLayout::CityStatisticsEvents,
            _ => {
                skip_frame(&mut reader).unwrap();
                continue;
            }
        };
        let frame = read_next_frame(&mut reader, MAX_FRAME).unwrap();
        let edited = remap.rewrite(&frame, layout).unwrap();
        if name == "Game.Simulation.CitySystem" {
            assert_eq!(&edited[4..], &remap.resolve(0x76b2b).unwrap().to_le_bytes());
            assert_ne!(edited, frame);
        }
        if name == "Game.UI.InGame.ChirpLinkSystem" {
            assert_eq!(edited.len(), frame.len());
            assert_ne!(edited, frame);
        }
        if name == "Game.Simulation.CityStatisticsSystem" {
            assert_eq!(frame.len(), 1_368_820);
            assert_eq!(
                u32::from_le_bytes(frame[12..16].try_into().unwrap()),
                114_067
            );
            assert_eq!(edited, frame);
        }
    }
    assert_eq!(reader.read(&mut [0]).unwrap(), 0);
}

fn find_dummy_traffic(
    mut frame: &[u8],
    row: &Archetype,
    rows: &[Archetype],
    types: &[Component],
    names: &[&str],
    candidates: &mut Vec<u64>,
) {
    if row.types.iter().any(|&id| {
        matches!(
            names[id],
            "Game.Objects.Static"
                | "Game.Routes.CurrentRoute"
                | "Game.Vehicles.CarTrailer"
                | "Game.Vehicles.LayoutElement"
        )
    }) {
        return;
    }
    let mut owner = None;
    let mut roles = Vec::new();
    for &id in &row.types {
        if types[id].flag == 1 {
            continue;
        }
        let payload = sized_bytes(&mut frame).unwrap();
        match names[id] {
            "Game.Common.Owner" => owner = Some(payload),
            "Game.Vehicles.PublicTransport" => {
                roles.push((payload, LegacyTransportKind::Passenger))
            }
            "Game.Vehicles.CargoTransport" => roles.push((payload, LegacyTransportKind::Cargo)),
            _ => {}
        }
    }
    let Some(owner) = owner else { return };
    for (transport, kind) in roles {
        let found = dummy_traffic_candidates(transport, owner, kind, |id| {
            let id = id as u64;
            let pos = rows.partition_point(|row| row.start <= id);
            pos.checked_sub(1)
                .and_then(|i| rows.get(i))
                .is_some_and(|owner| {
                    id < owner.start + u64::from(owner.count)
                        && owner
                            .types
                            .iter()
                            .any(|&id| names[id] == "Game.Buildings.TrafficSpawner")
                })
        })
        .unwrap();
        for index in found {
            let id = row.start + index as u64;
            if candidates.last() != Some(&id) {
                candidates.push(id);
            }
        }
    }
}
