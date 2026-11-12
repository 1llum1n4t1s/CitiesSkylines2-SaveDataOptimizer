//! 満車・経路失敗・遠い車庫からの戻りを確かめる試験コピーの作成と照合を行うCLI。
//! ゲーム付属の PipelinePlugin.dll を、圧縮済みの車庫形状の展開に使う。
use cok_inspector::save_data::{
    citizen_categories, compare_creatures, count_entities, job_market, path_demand, inspect_people_trial, inspect_vehicle_trial,
    moving_away_report, probe_components, rekey_manifest, save_sizes, write_people_trial,
    write_vehicle_trial,
};
use std::ffi::OsString;
use std::path::Path;

fn run(args: &[OsString]) -> Result<(), String> {
    let text = |v: &OsString| v.to_string_lossy().into_owned();
    if let [mode, input] = args
        && mode == "--jobs"
    {
        print!("{}", job_market(Path::new(input))?);
        return Ok(());
    }
    if let [mode, input] = args
        && mode == "--path-demand"
    {
        print!("{}", path_demand(Path::new(input))?);
        return Ok(());
    }
    if let [mode, input] = args
        && mode == "--citizens"
    {
        print!("{}", citizen_categories(Path::new(input))?);
        return Ok(());
    }
    if let [mode, input] = args
        && mode == "--moving-away"
    {
        print!("{}", moving_away_report(Path::new(input))?);
        return Ok(());
    }
    if let [mode, input, specs @ ..] = args
        && mode == "--count"
    {
        let specs: Vec<String> = specs.iter().map(text).collect();
        let specs: Vec<&str> = specs.iter().map(String::as_str).collect();
        print!("{}", count_entities(Path::new(input), &specs)?);
        return Ok(());
    }
    if let [mode, input, anchor, names @ ..] = args
        && mode == "--probe"
    {
        let names: Vec<String> = names.iter().map(text).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        print!(
            "{}",
            probe_components(Path::new(input), &text(anchor), &names)?
        );
        return Ok(());
    }
    if let [
        mode,
        input,
        output,
        plugin,
        manifest,
        package,
        data,
        metadata,
        texture,
    ] = args
        && mode == "--write"
    {
        let manifest_path = Path::new(manifest);
        if manifest_path.exists() {
            return Err("試験記録の出力先がすでに存在します".into());
        }
        let text = write_vehicle_trial(
            Path::new(input),
            Path::new(output),
            Path::new(plugin),
            [package, data, metadata, texture].map(|id| id.to_str().unwrap_or("")),
        )?;
        std::fs::write(manifest_path, &text).map_err(|e| e.to_string())?;
        print!("{text}");
        return Ok(());
    }
    if let [mode, input, plugin, manifest] = args
        && mode == "--check"
    {
        let manifest = std::fs::read_to_string(manifest).map_err(|e| e.to_string())?;
        print!(
            "{}",
            inspect_vehicle_trial(Path::new(input), Path::new(plugin), &manifest)?
        );
        return Ok(());
    }
    if let [
        mode,
        input,
        output,
        plugin,
        manifest,
        package,
        data,
        metadata,
        texture,
    ] = args
        && mode == "--write-people"
    {
        let manifest_path = Path::new(manifest);
        if manifest_path.exists() {
            return Err("試験記録の出力先がすでに存在します".into());
        }
        let text = write_people_trial(
            Path::new(input),
            Path::new(output),
            Path::new(plugin),
            [package, data, metadata, texture].map(|id| id.to_str().unwrap_or("")),
        )?;
        std::fs::write(manifest_path, &text).map_err(|e| e.to_string())?;
        print!("{text}");
        return Ok(());
    }
    if let [mode, input, plugin, manifest] = args
        && mode == "--check-people"
    {
        let manifest = std::fs::read_to_string(manifest).map_err(|e| e.to_string())?;
        print!(
            "{}",
            inspect_people_trial(Path::new(input), Path::new(plugin), &manifest)?
        );
        return Ok(());
    }
    if let [mode, input] = args
        && mode == "--sizes"
    {
        print!("{}", save_sizes(Path::new(input))?);
        return Ok(());
    }
    if let [mode, trial, plugin, manifest] = args
        && mode == "--rekey"
    {
        let manifest = std::fs::read_to_string(manifest).map_err(|e| e.to_string())?;
        print!(
            "{}",
            rekey_manifest(Path::new(trial), Path::new(plugin), &manifest)?
        );
        return Ok(());
    }
    if let [mode, before, after, plugin] = args
        && mode == "--creatures"
    {
        print!(
            "{}",
            compare_creatures(Path::new(before), Path::new(after), Path::new(plugin))?
        );
        return Ok(());
    }
    Err(concat!(
        "vehicle-trial --probe <入力> <基準の型> [<型>...]\n",
        "vehicle-trial --write <入力> <出力cok> <PipelinePlugin.dll> <試験記録> <CID 4つ>\n",
        "vehicle-trial --check <入力> <PipelinePlugin.dll> <試験記録>\n",
        "vehicle-trial --creatures <元セーブ> <自動保存> <PipelinePlugin.dll>
",
        "vehicle-trial --write-people <入力> <出力cok> <PipelinePlugin.dll> <試験記録> <CID 4つ>
",
        "vehicle-trial --check-people <入力> <PipelinePlugin.dll> <試験記録>",
    )
    .into())
}

fn main() {
    if let Err(error) = run(&std::env::args_os().skip(1).collect::<Vec<_>>()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
