use std::io::{self, Read};

const MAGIC: &[u8; 4] = b"SCR1";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FrameKey {
    pub fingerprint: u64,
    pub size: u32,
}

impl FrameKey {
    pub fn for_payload(payload: &[u8]) -> io::Result<Self> {
        let size = u32::try_from(payload.len()).map_err(io::Error::other)?;
        let mut fingerprint = 14_695_981_039_346_656_037_u64;
        let samples = payload.len().min(32);
        for sample in 0..samples {
            let index = sample as u64 * (payload.len() - 1) as u64 / (samples - 1).max(1) as u64;
            fingerprint =
                (fingerprint ^ u64::from(payload[index as usize])).wrapping_mul(1_099_511_628_211);
        }
        Ok(Self { fingerprint, size })
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CapturedReference {
    pub frame: FrameKey,
    pub offset: u32,
    pub id: i32,
}

impl CapturedReference {
    // 指紋は照合の候補を狭める用途。書き換えの前には記録した値と実バイトも照合する。
    pub fn matches_payload(&self, payload: &[u8]) -> bool {
        let Some(bytes) = payload
            .get(self.offset as usize..)
            .and_then(|tail| tail.get(..4))
        else {
            return false;
        };
        FrameKey::for_payload(payload).is_ok_and(|key| key == self.frame)
            && bytes == self.id.to_le_bytes()
    }
}

pub struct CaptureReader<R> {
    reader: R,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapturedStride {
    pub parent: FrameKey,
    pub parent_after: FrameKey,
    pub offset: u32,
    pub child: FrameKey,
    pub stride: u32,
}

pub fn read_strides(mut reader: impl Read) -> io::Result<Vec<CapturedStride>> {
    let mut magic = [0; 4];
    reader.read_exact(&mut magic)?;
    if &magic != b"SCS2" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ストライド記録の形式が一致しません",
        ));
    }
    let mut result = Vec::new();
    let mut bytes = [0; 40];
    while reader.read(&mut bytes[..1])? != 0 {
        reader.read_exact(&mut bytes[1..])?;
        let num = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        let key = |i| FrameKey {
            fingerprint: u64::from_le_bytes(bytes[i..i + 8].try_into().unwrap()),
            size: num(i + 8),
        };
        let (parent, offset, child, stride) = (key(0), num(12), key(16), num(28));
        if stride == 0
            || child.size == 0
            || offset
                .checked_add(child.size)
                .is_none_or(|end| end > parent.size)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ストライド記録の範囲が不正です",
            ));
        }
        let parent_after = FrameKey {
            fingerprint: u64::from_le_bytes(bytes[32..40].try_into().unwrap()),
            size: parent.size,
        };
        result.push(CapturedStride {
            parent,
            parent_after,
            offset,
            child,
            stride,
        });
    }
    Ok(result)
}

impl<R: Read> CaptureReader<R> {
    pub fn new(mut reader: R) -> io::Result<Self> {
        let mut magic = [0; 4];
        reader.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "参照記録の形式が一致しません",
            ));
        }
        Ok(Self { reader })
    }

    pub fn next_reference(&mut self) -> io::Result<Option<CapturedReference>> {
        let mut bytes = [0; 20];
        if self.reader.read(&mut bytes[..1])? == 0 {
            return Ok(None);
        }
        self.reader.read_exact(&mut bytes[1..])?;
        let frame = FrameKey {
            fingerprint: u64::from_le_bytes(bytes[..8].try_into().unwrap()),
            size: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        };
        let offset = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        if frame.size.checked_sub(4).is_none_or(|end| offset > end) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "参照記録の位置が範囲外です",
            ));
        }
        let id = i32::from_le_bytes(bytes[16..].try_into().unwrap());
        Ok(Some(CapturedReference { frame, offset, id }))
    }
}
