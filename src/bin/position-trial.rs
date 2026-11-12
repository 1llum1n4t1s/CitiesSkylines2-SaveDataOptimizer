use cok_inspector::save_data::{
    PositionGrid, PositionRegion, inspect_destinations, inspect_garage_trial, inspect_positions,
    write_garage_trial, write_position_trial, write_unspawned_trial,
};
use std::ffi::OsString;
use std::path::Path;

fn run(args: &[OsString]) -> Result<(), String> {
    if let [mode, input, output, package, data, metadata, texture] = args
        && mode == "--unspawned"
    {
        return write_unspawned_trial(
            Path::new(input),
            Path::new(output),
            [package, data, metadata, texture].map(|id| id.to_str().unwrap_or("")),
        );
    }
    let scalar = |value: &OsString| {
        value
            .to_string_lossy()
            .parse::<f32>()
            .map_err(|e| e.to_string())
    };
    if let [
        mode,
        input,
        output,
        plugin,
        package,
        data,
        metadata,
        texture,
    ] = args
        && mode == "--garage"
    {
        println!(
            "{}",
            write_garage_trial(
                Path::new(input),
                Path::new(output),
                Path::new(plugin),
                [package, data, metadata, texture].map(|id| id.to_str().unwrap_or(""))
            )?
        );
        return Ok(());
    }
    if let [mode, input, plugin, x, y, z, seeds @ ..] = args
        && mode == "--garage-check"
    {
        let seeds = seeds
            .iter()
            .map(|seed| {
                seed.to_string_lossy()
                    .parse::<u16>()
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        print!(
            "{}",
            inspect_garage_trial(
                Path::new(input),
                Path::new(plugin),
                [scalar(x)?, scalar(y)?, scalar(z)?],
                &seeds
            )?
        );
        return Ok(());
    }
    if let [input] = args {
        println!("{:#?}", inspect_positions(Path::new(input), None)?);
        return Ok(());
    }
    if let [input, min_x, max_x, min_z, max_z] = args {
        let region = PositionRegion {
            min_x: scalar(min_x)?,
            max_x: scalar(max_x)?,
            min_z: scalar(min_z)?,
            max_z: scalar(max_z)?,
        };
        println!("{:#?}", inspect_positions(Path::new(input), Some(&region))?);
        return Ok(());
    }
    if let [input, plugin, min_x, max_x, min_z, max_z] = args {
        let region = PositionRegion {
            min_x: scalar(min_x)?,
            max_x: scalar(max_x)?,
            min_z: scalar(min_z)?,
            max_z: scalar(max_z)?,
        };
        println!(
            "{:#?}",
            inspect_destinations(Path::new(input), Path::new(plugin), &region)?
        );
        return Ok(());
    }
    let [
        input,
        output,
        x,
        y,
        z,
        columns,
        spacing,
        package,
        data,
        metadata,
        texture,
    ] = args
    else {
        return Err(concat!(
            "position-trial <入力> [<最小X> <最大X> <最小Z> <最大Z>]\n",
            "position-trial <入力> <PipelinePlugin.dll> <最小X> <最大X> <最小Z> <最大Z>\n",
            "position-trial <入力> <出力> <x> <y> <z> <列数> <間隔> <CID 4つ>\n",
            "position-trial --unspawned <入力> <出力> <CID 4つ>\n",
            "position-trial --garage <入力> <出力> <PipelinePlugin.dll> <CID 4つ>\n",
            "position-trial --garage-check <入力> <PipelinePlugin.dll> <x> <y> <z> [<車両の乱数値>...]",
        )
        .into());
    };
    let grid = PositionGrid {
        origin: [scalar(x)?, scalar(y)?, scalar(z)?],
        columns: columns
            .to_string_lossy()
            .parse::<u32>()
            .map_err(|e| e.to_string())?,
        spacing: scalar(spacing)?,
    };
    write_position_trial(
        Path::new(input),
        Path::new(output),
        [package, data, metadata, texture].map(|id| id.to_str().unwrap_or("")),
        &grid,
    )
}

fn main() {
    if let Err(error) = run(&std::env::args_os().skip(1).collect::<Vec<_>>()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
