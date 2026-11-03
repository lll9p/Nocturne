mod large;
mod portable;
mod support;
use support::*;
fn main() {
    if let Err(e) = execute() {
        eprintln!("xtask: {e}");
        std::process::exit(1);
    }
}
fn execute() -> Result<()> {
    let Some(options) = Options::parse(std::env::args().skip(1).collect())?
    else {
        return Ok(());
    };
    let harness = Harness::new(&options.binary)?;
    let report = match options.task.as_str() {
        "portable-smoke" => portable::run(&harness)?,
        "large-library-smoke" => large::run(&harness)?,
        _ => unreachable!(),
    };
    let text = serde_json::to_string_pretty(&report)? + "\n";
    if let Some(path) = options.output {
        std::fs::write(path, &text)?;
    }
    print!("{text}");
    Ok(())
}
