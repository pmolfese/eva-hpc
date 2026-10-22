//! Shared processing arguments and collision-checked batch output planning.
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

pub const HELP: &str = "Batch mode: -inputs <file1.mff> <file2.mff> ... -suffix <string>\nSuffix is appended literally before .mff, e.g. -suffix _filt.\nOutputs are siblings of their inputs; existing outputs are never overwritten.\nUse ./ before input names starting with '-'. All files share processing settings\n(including MRI motion/timing sidecars). Jobs run sequentially and stop on failure;\ncompleted outputs remain. Explicit input/output syntax is still supported.";

pub struct Arguments {
    pub options: BTreeMap<String, String>,
    pub jobs: Vec<(PathBuf, PathBuf)>,
}
pub fn parse(args: Vec<OsString>) -> Result<Arguments, String> {
    let mut options = BTreeMap::new();
    let mut positional = vec![];
    let mut inputs = vec![];
    let mut batch = false;
    let mut collecting = false;
    let mut end = false;
    let mut suffix = None;
    let mut iter = args.into_iter().peekable();
    while let Some(arg) = iter.next() {
        if !end && arg == "--" {
            end = true;
            continue;
        }
        if !end && arg.to_str().is_some_and(|v| v.starts_with('-')) {
            collecting = false;
            if arg == "-inputs" {
                if batch {
                    return Err("Duplicate -inputs".into());
                }
                batch = true;
                collecting = true;
                if iter
                    .peek()
                    .is_none_or(|v| v.to_str().is_some_and(|s| s.starts_with('-')))
                {
                    return Err(
                        "-inputs requires at least one path (use ./ for names starting with '-')"
                            .into(),
                    );
                }
                continue;
            }
            let key = arg
                .to_str()
                .unwrap()
                .strip_prefix('-')
                .filter(|key| !key.starts_with('-'))
                .ok_or("Options must use a single dash")?;
            let (key, value) = if let Some((k, v)) = key.split_once('=') {
                (k.to_owned(), v.to_owned())
            } else {
                (
                    key.to_owned(),
                    iter.next()
                        .ok_or_else(|| format!("Missing value for -{key}"))?
                        .into_string()
                        .map_err(|_| "Option values must be UTF-8")?,
                )
            };
            if key == "suffix" {
                if suffix.replace(value).is_some() {
                    return Err("Duplicate -suffix".into());
                }
            } else if options.insert(key.clone(), value).is_some() {
                return Err(format!("Duplicate -{key}"));
            }
        } else if collecting {
            inputs.push(PathBuf::from(arg));
        } else {
            positional.push(PathBuf::from(arg));
        }
    }
    let jobs = if batch {
        if !positional.is_empty() {
            return Err("Do not mix positional input/output paths with -inputs".into());
        }
        let suffix = suffix.ok_or("-inputs requires -suffix")?;
        if suffix.is_empty() || suffix.contains(['/', '\\', '\0']) {
            return Err("Suffix must be nonempty and cannot contain path separators or NUL".into());
        }
        let mut jobs = vec![];
        let mut outputs = std::collections::HashSet::new();
        for input in inputs {
            if !input
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("mff"))
            {
                return Err(format!(
                    "Batch input must have a .mff extension: {}",
                    input.display()
                ));
            }
            let canonical =
                std::fs::canonicalize(&input).map_err(|e| format!("{}: {e}", input.display()))?;
            if !canonical.is_dir() {
                return Err(format!("Not an MFF directory: {}", input.display()));
            }
            let mut name = input
                .file_stem()
                .ok_or("Input requires a filename")?
                .to_os_string();
            name.push(&suffix);
            name.push(".");
            name.push(input.extension().unwrap());
            let output = input.with_file_name(name);
            let parent = output
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new("."));
            let resolved = std::fs::canonicalize(parent)
                .map_err(|e| e.to_string())?
                .join(output.file_name().unwrap());
            if resolved.symlink_metadata().is_ok() || !outputs.insert(resolved) {
                return Err(format!(
                    "Output exists or is duplicated: {}",
                    output.display()
                ));
            }
            jobs.push((input, output));
        }
        jobs
    } else {
        if suffix.is_some() {
            return Err("-suffix requires -inputs".into());
        }
        if positional.len() != 2 {
            return Err("Expected input and output MFF paths, or -inputs with -suffix".into());
        }
        vec![(positional.remove(0), positional.remove(0))]
    };
    Ok(Arguments { options, jobs })
}

pub fn execute(
    jobs: &[(PathBuf, PathBuf)],
    mut process: impl FnMut(&std::path::Path, &std::path::Path) -> Result<(), String>,
) -> Result<(), String> {
    for (i, (input, output)) in jobs.iter().enumerate() {
        eprintln!(
            "[{}/{}] {} -> {}",
            i + 1,
            jobs.len(),
            input.display(),
            output.display()
        );
        process(input, output).map_err(|e| {
            format!(
                "{}: {e} (stopped; {i} completed output(s) retained)",
                input.display()
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }
    #[test]
    fn batch_filter_exports_each_input_with_provenance_and_preflights_collisions() {
        let parent = tempfile::tempdir().unwrap();
        let mut paths = vec![];
        for name in ["one.mff", "two with spaces.mff"] {
            let fixture = crate::tests::fixture();
            let path = parent.path().join(name);
            std::fs::rename(fixture.path(), &path).unwrap();
            paths.push(path);
        }
        let mut cmd = args(&["filter", "-lowpass", "40", "-inputs"]);
        cmd.extend(paths.iter().map(|p| p.as_os_str().to_owned()));
        cmd.extend(args(&["-suffix", "_filt"]));
        crate::run(cmd.clone()).unwrap();
        for name in ["one_filt.mff", "two with spaces_filt.mff"] {
            let output = parent.path().join(name);
            let r = eva_mff::Recording::open(&output).unwrap();
            assert_eq!(r.metadata().sample_count, 4224);
            assert!(std::fs::read_to_string(output.join("eva.xml"))
                .unwrap()
                .contains("op=\"filter\""));
        }
        std::fs::remove_dir_all(parent.path().join("one_filt.mff")).unwrap();
        assert!(crate::run(cmd).unwrap_err().contains("Output exists"));
        assert!(!parent.path().join("one_filt.mff").exists());
        let mut duplicate = args(&["-suffix=_new", "-inputs"]);
        duplicate.extend([
            paths[0].as_os_str().to_owned(),
            paths[0].as_os_str().to_owned(),
        ]);
        assert!(parse(duplicate).is_err());
    }
    #[test]
    fn double_dash_options_are_rejected() {
        for option in ["--lowpass=40", "--inputs", "--suffix", "---highpass"] {
            assert!(parse(args(&[option, "in.mff", "out.mff"])).is_err());
        }
        assert!(parse(args(&["-lowpass=40", "in.mff", "out.mff"])).is_ok());
    }
    #[test]
    fn invalid_forms_and_legacy_paths() {
        for items in [
            vec!["-inputs"],
            vec!["-inputs", "a.mff"],
            vec!["-suffix", "_x", "a.mff", "b.mff"],
            vec!["a.mff", "-inputs", "b.mff", "-suffix", "_x"],
            vec!["-inputs", "a.mff", "-suffix", ""],
            vec!["-inputs", "a.mff", "-suffix", "/bad"],
            vec!["-inputs", "a.mff", "-inputs", "b.mff", "-suffix", "_x"],
        ] {
            assert!(parse(args(&items)).is_err(), "{items:?}");
        }
        let parsed = parse(args(&["-lowpass=40", "--", "-input.mff", "output.mff"])).unwrap();
        assert_eq!(parsed.jobs[0].0, PathBuf::from("-input.mff"));
        assert_eq!(parsed.options["lowpass"], "40");
    }
    #[test]
    fn processing_failure_stops_remaining_jobs() {
        let jobs: Vec<_> = (0..3)
            .map(|i| {
                (
                    PathBuf::from(format!("{i}.mff")),
                    PathBuf::from(format!("{i}_out.mff")),
                )
            })
            .collect();
        let mut calls = 0;
        let error = execute(&jobs, |_, _| {
            calls += 1;
            if calls == 2 {
                Err("failed".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(calls, 2);
        assert!(error.contains("1 completed"));
    }
}
