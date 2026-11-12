use cok_inspector::save_data::{clean_save, preview_cleanup};
use std::ffi::OsString;
use std::path::Path;

fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let result = run(&args);
    match result {
        Ok(()) => {}
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn run(args: &[OsString]) -> Result<(), String> {
    if let [input] = args {
        println!("{:#?}", preview_cleanup(Path::new(input))?);
        return Ok(());
    }
    let [
        input,
        output,
        refs,
        strides,
        plugin,
        package_id,
        data_id,
        metadata_id,
        texture_id,
    ] = args
    else {
        return Err("使い方: clean-beartris <元cok> <出力cok> <参照記録> <stride記録> <PipelinePlugin.dll> <package-id> <data-id> <metadata-id> <texture-id>".into());
    };
    let ids = [package_id, data_id, metadata_id, texture_id].map(|id| id.to_str().unwrap_or(""));
    clean_save(
        Path::new(input),
        Path::new(output),
        Path::new(refs),
        Path::new(strides),
        Path::new(plugin),
        ids,
        None,
    )?;
    println!("編集済みセーブを保存しました");
    Ok(())
}
