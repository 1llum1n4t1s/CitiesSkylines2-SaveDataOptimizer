use std::cell::OnceCell;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use zip::ZipArchive;

pub const PREVIEW_LIMIT: u64 = 64 * 1024;

/// ZIP の中で、名前が `suffix` で終わる最初の項目の名前を返す。
///
/// zip 9 からは `file_names()` が名前ごとに `Result` を返す（名前の符号化が不正な項目があり得るため）。
/// 読めない名前の項目は探している項目ではないので飛ばし、従来どおり最初に一致した名前を使う。
pub fn find_entry_name<R: Read + io::Seek>(zip: &ZipArchive<R>, suffix: &str) -> Option<String> {
    zip.file_names()
        .filter_map(Result::ok)
        .find(|name| name.ends_with(suffix))
        .map(|name| name.into_owned())
}

#[derive(Debug)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    pub compressed: u64,
    pub method: String,
}

pub struct Save {
    pub path: PathBuf,
    pub file_size: u64,
    pub entries: Vec<Entry>,
    pub external_cid: Option<PathBuf>,
    save_data_index: Option<usize>,
    save_data_report: OnceCell<Result<String, String>>,
}

impl Save {
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("ファイルを開けません: {e}"))?;
        let file_size = file.metadata().map_err(|e| e.to_string())?.len();
        let mut zip =
            ZipArchive::new(file).map_err(|e| format!("ZIP 形式として開けません: {e}"))?;
        let mut entries = Vec::with_capacity(zip.len());
        for index in 0..zip.len() {
            let entry = zip
                .by_index_raw(index)
                .map_err(|e| format!("項目 {index} を読めません: {e}"))?;
            entries.push(Entry {
                // zip 9 からは名前の取り出しが失敗し得る（不正な符号化など）。読めない名前は項目ごと開けないものとして扱う。
                name: entry
                    .name()
                    .map_err(|e| format!("項目 {index} の名前を読めません: {e}"))?
                    .into_owned(),
                size: entry.size(),
                compressed: entry.compressed_size(),
                method: format!("{:?}", entry.compression()),
            });
        }
        let mut cid_name = path.as_os_str().to_os_string();
        cid_name.push(".cid");
        let cid_path = PathBuf::from(cid_name);
        let external_cid = cid_path.is_file().then_some(cid_path);
        let save_data_index = entries
            .iter()
            .position(|entry| entry.name.ends_with(".SaveGameData"));
        Ok(Self {
            path: path.to_path_buf(),
            file_size,
            entries,
            external_cid,
            save_data_index,
            save_data_report: OnceCell::new(),
        })
    }

    pub fn summary(&self) -> String {
        format!(
            "{}\r\n形式: ZIP (.cok)   サイズ: {} bytes   項目: {}   外部 .cid: {}",
            self.path.display(),
            self.file_size,
            self.entries.len(),
            if self.external_cid.is_some() {
                "あり"
            } else {
                "なし"
            }
        )
    }

    pub fn labels(&self) -> Vec<String> {
        let mut labels: Vec<_> = self
            .entries
            .iter()
            .map(|e| format!("{}  |  {} bytes  |  {}", e.name, e.size, e.method))
            .collect();
        if self.external_cid.is_some() {
            labels.push("外部 .cok.cid".into());
        }
        if self.save_data_index.is_some() {
            labels.push("車両・歩行者の集計".into());
        }
        labels
    }

    pub fn analysis_index(&self) -> Option<usize> {
        self.save_data_index
            .map(|_| self.entries.len() + usize::from(self.external_cid.is_some()))
    }

    pub fn preview(&self, index: usize) -> Result<String, String> {
        if index < self.entries.len() {
            let file = File::open(&self.path).map_err(|e| e.to_string())?;
            let mut zip = ZipArchive::new(file).map_err(|e| e.to_string())?;
            let entry = zip
                .by_index(index)
                .map_err(|e| format!("項目を展開できません: {e}"))?;
            let info = &self.entries[index];
            let header = format!(
                "名前: {}\r\n展開サイズ: {} bytes   圧縮サイズ: {} bytes   圧縮方式: {}\r\n\r\n",
                info.name, info.size, info.compressed, info.method
            );
            let data = read_prefix(entry).map_err(|e| format!("項目を読めません: {e}"))?;
            Ok(header + &render_preview(&data, info.size))
        } else if index == self.entries.len() && self.external_cid.is_some() {
            let path = self.external_cid.as_ref().ok_or("項目がありません")?;
            let file = File::open(path).map_err(|e| e.to_string())?;
            let size = file.metadata().map_err(|e| e.to_string())?.len();
            let data = read_prefix(file).map_err(|e| e.to_string())?;
            Ok(format!(
                "外部 CID: {}\r\nサイズ: {} bytes\r\n\r\n{}",
                path.display(),
                size,
                render_preview(&data, size)
            ))
        } else if Some(index) == self.analysis_index() {
            let data_index = self.save_data_index.ok_or("SaveGameData がありません")?;
            self.save_data_report
                .get_or_init(|| {
                    let mut zip =
                        ZipArchive::new(File::open(&self.path).map_err(|e| e.to_string())?)
                            .map_err(|e| e.to_string())?;
                    let entry = zip.by_index(data_index).map_err(|e| e.to_string())?;
                    crate::save_data::analyze(entry).map(|report| report.render())
                })
                .clone()
        } else {
            Err("項目がありません".into())
        }
    }
}

// 巨大なセーブ本体を全展開せず、表示に必要な先頭だけを読み取る。
fn read_prefix(reader: impl Read) -> io::Result<Vec<u8>> {
    let mut result = Vec::new();
    reader.take(PREVIEW_LIMIT).read_to_end(&mut result)?;
    Ok(result)
}

fn render_preview(data: &[u8], total: u64) -> String {
    let mut result = if let Ok(s) = std::str::from_utf8(data) {
        if s.chars()
            .all(|c| !c.is_control() || matches!(c, '\r' | '\n' | '\t'))
        {
            // EDIT コントロールでは CRLF を使う。
            s.replace("\r\n", "\n").replace('\n', "\r\n")
        } else {
            hex(data)
        }
    } else {
        hex(data)
    };
    if total > data.len() as u64 {
        result.push_str(&format!(
            "\r\n\r\n[先頭 {} bytes のみ表示 / 全 {} bytes]",
            data.len(),
            total
        ));
    }
    result
}

fn hex(data: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for (row, chunk) in data.chunks(16).enumerate() {
        let _ = write!(out, "{:08X}  ", row * 16);
        for i in 0..16 {
            if let Some(byte) = chunk.get(i) {
                let _ = write!(out, "{byte:02X} ");
            } else {
                out.push_str("   ");
            }
        }
        out.push(' ');
        for &byte in chunk {
            out.push(if (0x20..=0x7e).contains(&byte) {
                char::from(byte)
            } else {
                '.'
            });
        }
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_is_bounded_and_preserves_binary() {
        let data = vec![0; PREVIEW_LIMIT as usize + 10];
        let prefix = read_prefix(&data[..]).unwrap();
        assert_eq!(prefix.len(), PREVIEW_LIMIT as usize);
        let preview = render_preview(&prefix, data.len() as u64);
        assert!(preview.starts_with("00000000  00 00"));
        assert!(preview.contains("先頭 65536 bytes のみ表示"));
    }
    #[test]
    fn text_preview_normalizes_newlines() {
        assert_eq!(
            render_preview(b"hello\nworld\r\n", 13),
            "hello\r\nworld\r\n"
        );
    }
    #[test]
    fn real_save_can_be_inspected() {
        let Some(path) = std::env::var_os("COK_TEST_SAMPLE") else {
            return;
        };
        let save = Save::open(Path::new(&path)).unwrap();
        assert!(!save.entries.is_empty());
        assert!(save.external_cid.is_some());
        for index in 0..save.labels().len() {
            let preview = save.preview(index).unwrap();
            assert!(!preview.is_empty());
        }
    }

    #[test]
    fn real_saves_show_entity_counts_and_references() {
        for (var, counts) in [
            ("COK_TEST_SAMPLE", [17601, 15154, 17839, 73445]),
            ("COK_TEST_MICHATOWN", [107856, 95402, 57651, 673817]),
        ] {
            let Some(path) = std::env::var_os(var) else {
                continue;
            };
            let save = Save::open(Path::new(&path)).unwrap();
            let report = save.preview(save.analysis_index().unwrap()).unwrap();
            for (name, count) in ["車両", "駐車車両", "歩行者の実体", "市民データ"]
                .iter()
                .zip(counts)
            {
                assert!(
                    report
                        .lines()
                        .any(|line| line.starts_with(&format!("{name}: {count}  ")))
                );
            }
            if var == "COK_TEST_SAMPLE" {
                assert!(report.contains("CurrentVehicle の参照\r\n  車両: 7104  人物の実体: 0"));
                assert!(report.contains("CurrentTransport の参照\r\n  車両: 0  人物の実体: 17837"));
                assert!(report.contains("38735 → 684995"));
            } else {
                assert!(report.contains("CurrentVehicle の参照\r\n  車両: 18688  人物の実体: 0"));
                assert!(report.contains("CurrentTransport の参照\r\n  車両: 0  人物の実体: 57645"));
                assert!(report.contains("Owner の参照\r\n  車両: 5277  人物の実体: 1"));
                assert!(report.contains("6410643 → 3879730"));
            }
            assert!(report.contains("元ID → 先ID"));
        }
    }
}
