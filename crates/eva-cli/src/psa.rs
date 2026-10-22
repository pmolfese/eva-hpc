use std::{ffi::OsString, sync::atomic::AtomicU64};
pub fn run(args: Vec<OsString>) -> Result<(), String> {
    if args.is_empty()
        || args
            .iter()
            .take_while(|a| *a != "--")
            .any(|a| a == "-help" || a == "-h")
    {
        println!("{}", eva_core::psa::cli::HELP);
        return Ok(());
    }
    if args.iter().take_while(|a| *a != "--").any(|a| {
        a == "-inputs" || a == "-suffix" || a.to_str().is_some_and(|v| v.starts_with("-suffix="))
    }) {
        return Err("PSA takes one input and one output bundle".into());
    }
    let a = super::batch::parse(args)?;
    let config = eva_core::psa::cli::parse(&a.options)?;
    let (input, output) = &a.jobs[0];
    if output.exists() {
        return Err("Output already exists".into());
    }
    let input = std::fs::canonicalize(input).map_err(|e| e.to_string())?;
    let parent = std::fs::canonicalize(
        output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new(".")),
    )
    .map_err(|e| e.to_string())?;
    let output = parent.join(output.file_name().ok_or("Output requires a name")?);
    if output.starts_with(&input) {
        return Err("Output must be outside input MFF".into());
    }
    let r = eva_mff::Recording::open(&input).map_err(|e| e.to_string())?;
    let xml = input.join("eva.xml");
    let previous = if xml.exists() {
        if std::fs::metadata(&xml).map_err(|e| e.to_string())?.len() > 32 * 1024 * 1024 {
            return Err("Source eva.xml too large".into());
        }
        Some(std::fs::read_to_string(xml).map_err(|e| e.to_string())?)
    } else {
        None
    };
    let result = eva_core::psa::run(
        &r,
        &config,
        &eva_core::filter::Settings::default(),
        None,
        &AtomicU64::new(1),
        1,
        &|_| {},
    )?;
    eva_core::psa::export::write(
        &result,
        &output,
        &input.to_string_lossy(),
        previous.as_deref(),
    )?;
    eprintln!(
        "PSA: {} / {} accepted; {} averages. Wrote {}",
        result.epochs.len(),
        result.decisions.len(),
        result.averages.len(),
        output.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn cli_exports_epochs_and_provenance_without_changing_input() {
        let d = crate::tests::fixture();
        let output = tempfile::tempdir().unwrap();
        let path = output.path().join("test.psa");
        let original = std::fs::read(d.path().join("signal1.bin")).unwrap();
        let mut args: Vec<std::ffi::OsString> =
            ["-events", "TREV", "-threshold", "false", "-average", "true"]
                .iter()
                .map(Into::into)
                .collect();
        args.extend([d.path().as_os_str().to_owned(), path.as_os_str().to_owned()]);
        super::run(args.clone()).unwrap();
        assert!(super::run(args).is_err());
        let v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(v["averages"][0]["count"], 1);
        assert_eq!(v["sample_rate"], 256);
        assert_eq!(
            original,
            std::fs::read(d.path().join("signal1.bin")).unwrap()
        );
        assert!(std::fs::read_to_string(path.join("source-eva.xml"))
            .unwrap()
            .contains("markBad"));
    }
}
