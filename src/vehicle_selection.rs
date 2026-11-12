/// 旧形式の乗客・貨物車両のコンポーネントから、スポナーに再生成経路のある候補を選ぶ。
pub enum LegacyTransportKind {
    Passenger,
    Cargo,
}

/// 呼び出し元は Vehicle と Owner を持つアーキタイプに限定し、路線、牽引車、
/// 静的プロップなどの別用途の車両は除く。
pub fn dummy_traffic_candidates(
    transport: &[u8],
    owners: &[u8],
    kind: LegacyTransportKind,
    mut owner_is_traffic_spawner: impl FnMut(i32) -> bool,
) -> Result<Vec<usize>, String> {
    if !transport.len().is_multiple_of(20)
        || !owners.len().is_multiple_of(4)
        || transport.len() / 20 != owners.len() / 4
    {
        return Err("旧形式の車両と所有者のレコード長が一致しません".into());
    }
    let dummy_traffic = match kind {
        LegacyTransportKind::Passenger => 0x2000,
        LegacyTransportKind::Cargo => 0x0400,
    };
    let mut candidates = Vec::new();
    for (index, (vehicle, owner)) in transport
        .chunks_exact(20)
        .zip(owners.chunks_exact(4))
        .enumerate()
    {
        let request = i32::from_le_bytes(vehicle[..4].try_into().unwrap());
        let state = u32::from_le_bytes(vehicle[4..8].try_into().unwrap());
        let owner = i32::from_le_bytes(owner.try_into().unwrap());
        // 稼働中のリクエストを持つ車両は、フラグがあっても保持する。
        if request == -1
            && state & dummy_traffic != 0
            && owner >= 0
            && owner_is_traffic_spawner(owner)
        {
            candidates.push(index);
        }
    }
    Ok(candidates)
}
