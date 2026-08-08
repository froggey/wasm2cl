use clap::Parser as ClapParser;
use std::{fs, path::PathBuf};

use anyhow::{Result, bail};
use wasm2cl::{emit, module};

#[derive(ClapParser)]
struct Cli {
    input: PathBuf,
    package: String,
    #[arg(long, default_value = "100")]
    functions_per_file: usize,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let bytes = fs::read(&cli.input)?;
    let module = module::parse(&bytes)?;

    println!("types:        {}", module.types.len());
    println!("functions:    {}", module.functions.len());
    println!("globals:      {}", module.globals.len());
    println!("exports:      {}", module.exports.len());
    println!("active datas: {}", module.active_data.len());
    println!("active elts:  {}", module.active_elements.len());

    let dir = std::path::Path::new(&cli.package);
    if dir.exists() {
        bail!("output directory already exists: {}", dir.display());
    }
    fs::create_dir_all(dir)?;

    emit::emit_system(&module, &cli.package, dir, cli.functions_per_file)?;
    emit::emit_main(&module, &cli.package, dir)?;
    emit::emit_functions(&module, &cli.package, dir, cli.functions_per_file)?;

    //fs::write(&cli.output, emit(&module, &cli.package)?)?;

    Ok(())
}
