use crate::files;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const WM_MEASUREMENT: u32 = 0x8002;

#[derive(Clone)]
pub struct Step {
    pub category: &'static str,
    pub paths: Vec<PathBuf>,
    pub open: bool,
}

pub struct Measurement {
    root: PathBuf,
    fixtures: Vec<Value>,
    steps: Vec<Step>,
    index: usize,
    process_started: Instant,
    request_started: Option<Instant>,
    samples: Vec<Value>,
}

impl Measurement {
    pub fn parse(
        args: &[OsString],
        profile: &Path,
        process_started: Instant,
    ) -> io::Result<Option<Self>> {
        if args.first().and_then(|arg| arg.to_str()) != Some("--performance-test") {
            return Ok(None);
        }
        let root = args
            .get(1)
            .ok_or_else(|| io::Error::other("Missing benchmark output directory"))?;
        let root = std::path::absolute(root)?;
        let run_profile = root.join("profile");
        let exe = std::env::current_exe()?;
        let project = exe
            .ancestors()
            .find(|candidate| {
                candidate.join("Folio.cmd").is_file()
                    && candidate.join("src/rust/Cargo.toml").is_file()
            })
            .ok_or_else(|| {
                io::Error::other("Could not find the Folio project for a benchmark run")
            })?;
        if !root.starts_with(project.join("outputs"))
            || files::path_key(profile) != files::path_key(&run_profile)
        {
            return Err(io::Error::other(
                "Benchmark output and isolated profile are required inside the project",
            ));
        }
        files::ensure_no_reparse(&root)?;
        let iterations = args
            .windows(2)
            .find(|pair| pair[0] == "--performance-iterations")
            .and_then(|pair| pair[1].to_str())
            .and_then(|count| count.parse::<usize>().ok())
            .unwrap_or(16)
            .clamp(12, 20);
        let fixtures_root = project.join("work/2026-10-24-reader-performance/fixtures");
        let typical = fixtures_root.join("typical-chinese-prose.md");
        let large = fixtures_root.join("large-gfm.md");
        let fixtures = [&typical, &large].into_iter().map(|path| Ok(json!({
            "Name": name(path), "LengthBytes": path.metadata()?.len(), "SHA256": file_hash(path)?
        }))).collect::<io::Result<Vec<_>>>()?;
        let mut steps = vec![
            Step {
                category: "firstOpen",
                paths: vec![typical.clone()],
                open: true,
            },
            Step {
                category: "fixtureImport",
                paths: vec![large.clone()],
                open: true,
            },
            Step {
                category: "warmup",
                paths: vec![typical.clone()],
                open: false,
            },
        ];
        for index in 0..iterations {
            steps.push(Step {
                category: "measuredSwitch",
                paths: vec![if index % 2 == 0 {
                    large.clone()
                } else {
                    typical.clone()
                }],
                open: false,
            });
        }
        if args.iter().any(|arg| arg == "--rapid-switch-probe") {
            steps.push(Step {
                category: "rapidSwitch",
                paths: vec![large, typical],
                open: false,
            });
        }
        Ok(Some(Self {
            root,
            fixtures,
            steps,
            index: 0,
            process_started,
            request_started: None,
            samples: Vec::new(),
        }))
    }

    pub fn next(&mut self) -> Option<Step> {
        let step = self.steps.get(self.index)?.clone();
        self.request_started = Some(Instant::now());
        Some(step)
    }

    pub fn presented(&mut self, current: &Path, blocks: usize) -> bool {
        let Some(start) = self.request_started else {
            return false;
        };
        let expected = &self.steps[self.index];
        if blocks == 0
            || !expected
                .paths
                .last()
                .is_some_and(|path| files::path_key(path) == files::path_key(current))
        {
            return false;
        }
        let millis = start.elapsed().as_secs_f64() * 1000.;
        let from_entry = self.process_started.elapsed().as_secs_f64() * 1000.;
        self.samples.push(json!({
            "category": expected.category, "fixture": name(current), "drawSubmittedMs": millis,
            "processEntryToDrawMs": from_entry, "documentBlocks": blocks
        }));
        self.index += 1;
        self.request_started = None;
        true
    }

    pub fn finish(&self) -> io::Result<()> {
        let mut aggregates = Vec::new();
        for fixture in self
            .fixtures
            .iter()
            .filter_map(|fixture| fixture["Name"].as_str())
        {
            let mut values: Vec<_> = self
                .samples
                .iter()
                .filter(|sample| {
                    sample["category"] == "measuredSwitch" && sample["fixture"] == fixture
                })
                .filter_map(|sample| sample["drawSubmittedMs"].as_f64())
                .collect();
            values.sort_by(f64::total_cmp);
            if let Some(p50) = percentile(&values, 0.5) {
                aggregates.push(json!({"fixture": fixture, "sampleCount": values.len(), "drawP50Ms": p50, "drawP95Ms": percentile(&values, 0.95)}));
            }
        }
        let report = json!({
            "runId": name(&self.root), "succeeded": self.index == self.steps.len(),
            "error": if self.index == self.steps.len() { "" } else { "Incomplete native frame sequence" },
            "measurement": "Successful Direct2D EndDraw in the live preview window",
            "executableSha256": file_hash(&std::env::current_exe()?)?,
            "fixtures": self.fixtures, "samples": self.samples, "measuredSwitchAggregates": aggregates,
            "rapidSwitchProbe": self.samples.iter().find(|sample| sample["category"] == "rapidSwitch")
        });
        files::atomic_write(
            &self.root.join("performance.json"),
            &serde_json::to_vec_pretty(&report)?,
            None,
        )
    }
}

fn percentile(sorted: &[f64], fraction: f64) -> Option<f64> {
    let rank = sorted.len().checked_sub(1)? as f64 * fraction;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    Some(sorted[lo] + (sorted[hi] - sorted[lo]) * rank.fract())
}

fn name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn file_hash(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect())
}
