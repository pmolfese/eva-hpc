use eva_core::{filter, ocular};
use std::{ffi::OsString, sync::atomic::AtomicU64};
pub fn run(args: Vec<OsString>) -> Result<(), String> {
    if args.is_empty()
        || args
            .iter()
            .take_while(|a| *a != "--")
            .any(|a| a == "-help" || a == "-h")
    {
        println!("{}", ocular::cli::HELP);
        return Ok(());
    }
    if args.iter().take_while(|a| *a != "--").any(|a| {
        a == "-inputs" || a == "-suffix" || a.to_str().is_some_and(|v| v.starts_with("-suffix="))
    }) {
        return Err("artifactdetect takes one input MFF and one output JSON report".into());
    }
    let args = crate::batch::parse(args)?;
    let settings = ocular::cli::parse(&args.options)?;
    let (input, output) = &args.jobs[0];
    let input = std::fs::canonicalize(input).map_err(|e| e.to_string())?;
    let parent = std::fs::canonicalize(
        output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new(".")),
    )
    .map_err(|e| e.to_string())?;
    let output = parent.join(output.file_name().ok_or("Output requires a filename")?);
    if output.exists() || output.starts_with(&input) {
        return Err("Output must be new and outside input MFF".into());
    }
    let r = eva_mff::Recording::open(&input).map_err(|e| e.to_string())?;
    let mut report = ocular::run(
        &r,
        &settings,
        &filter::Settings::default(),
        None,
        &AtomicU64::new(1),
        1,
        &|_| {},
    )?;
    report.source = input.to_string_lossy().into();
    ocular::write_report(&report, &output)?;
    for warning in &report.warnings {
        eprintln!("Warning: {warning}");
    }
    eprintln!(
        "Detected {} blinks and {} movements. Wrote {}",
        report
            .candidates
            .iter()
            .filter(|c| c.kind == ocular::Kind::Blink)
            .count(),
        report
            .candidates
            .iter()
            .filter(|c| c.kind == ocular::Kind::Movement)
            .count(),
        output.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn command_report_no_overwrite_and_single_dash() {
        let input = crate::tests::fixture();
        let out = tempfile::tempdir().unwrap();
        let path = out.path().join("eyes.json");
        let original = std::fs::read(input.path().join("signal1.bin")).unwrap();
        let args = vec![
            input.path().as_os_str().to_owned(),
            path.as_os_str().to_owned(),
        ];
        super::run(args.clone()).unwrap();
        assert!(super::run(args).is_err());
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(report["format"], "eva-ocular-threshold");
        assert_eq!(
            std::fs::read(input.path().join("signal1.bin")).unwrap(),
            original
        );
        assert!(super::run(vec![
            "--blink".into(),
            "false".into(),
            input.path().into(),
            out.path().join("bad.json").into()
        ])
        .is_err());
    }
}
