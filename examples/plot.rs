//! Regenerates the benchmark charts in `assets/` and prints Markdown tables.
//!
//! ```text
//! cargo bench --bench throughput && cargo bench --bench latency
//! cargo run --release --example plot
//! ```
//!
//! Reads criterion's `target/criterion/**/new/{benchmark,estimates}.json` and
//! `target/latency/latency.json`. Throughput is the median with its 95%
//! confidence interval, converted to million items per second using the
//! element count criterion recorded, so nothing here hardcodes bench settings.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use plotters::prelude::*;
use serde_json::Value;

type Res<T> = Result<T, Box<dyn Error>>;

/// Okabe-Ito colours: distinguishable with every common colour-vision deficiency.
const QUEUES: [(&str, &str, RGBColor); 4] = [
    (
        "lockfree",
        "LockFreeQueue (this crate)",
        RGBColor(0, 114, 178),
    ),
    (
        "crossbeam",
        "crossbeam ArrayQueue + spin",
        RGBColor(230, 159, 0),
    ),
    (
        "blocking",
        "BlockingQueue (this crate)",
        RGBColor(0, 158, 115),
    ),
    (
        "std_sync_channel",
        "std sync_channel",
        RGBColor(204, 121, 167),
    ),
];
const FONT: &str = "sans-serif";

#[derive(Debug, Clone)]
struct Sample {
    queue: String,
    producers: u32,
    consumers: u32,
    capacity: u32,
    /// Million items per second: median, and 95% CI bounds.
    median: f64,
    lo: f64,
    hi: f64,
}

fn find_runs(dir: &Path, out: &mut Vec<PathBuf>) -> Res<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "new") {
                out.push(path);
            } else {
                find_runs(&path, out)?;
            }
        }
    }
    Ok(())
}

fn parse_shape(value: &str) -> Option<(u32, u32, u32)> {
    // "4p4c_cap256"
    let (shape, cap) = value.split_once("_cap")?;
    let (p, c) = shape.trim_end_matches('c').split_once('p')?;
    Some((p.parse().ok()?, c.parse().ok()?, cap.parse().ok()?))
}

fn load_throughput(root: &Path) -> Res<BTreeMap<String, Vec<Sample>>> {
    let mut runs = Vec::new();
    find_runs(root, &mut runs)?;
    let mut groups: BTreeMap<String, Vec<Sample>> = BTreeMap::new();
    for run in runs {
        let meta: Value = serde_json::from_str(&fs::read_to_string(run.join("benchmark.json"))?)?;
        let est: Value = serde_json::from_str(&fs::read_to_string(run.join("estimates.json"))?)?;
        let Some(elements) = meta["throughput"]["Elements"].as_f64() else {
            continue;
        };
        let Some((producers, consumers, capacity)) =
            meta["value_str"].as_str().and_then(parse_shape)
        else {
            continue;
        };
        let median = &est["median"];
        let per_sec = |ns: f64| elements / ns * 1e3; // Melem/s
        let ci = &median["confidence_interval"];
        groups
            .entry(meta["group_id"].as_str().unwrap_or_default().to_owned())
            .or_default()
            .push(Sample {
                queue: meta["function_id"].as_str().unwrap_or_default().to_owned(),
                producers,
                consumers,
                capacity,
                median: per_sec(median["point_estimate"].as_f64().ok_or("no median")?),
                // A longer time means lower throughput, so the bounds swap.
                lo: per_sec(ci["upper_bound"].as_f64().ok_or("no ci")?),
                hi: per_sec(ci["lower_bound"].as_f64().ok_or("no ci")?),
            });
    }
    Ok(groups)
}

fn label(queue: &str) -> &'static str {
    QUEUES.iter().find(|q| q.0 == queue).map_or("?", |q| q.1)
}

fn color(queue: &str) -> RGBColor {
    QUEUES.iter().find(|q| q.0 == queue).map_or(BLACK, |q| q.2)
}

/// Line chart over a power-of-two x axis, one series per queue, with CI bars.
fn line_chart(
    path: &Path,
    title: &str,
    x_desc: &str,
    samples: &[Sample],
    x_of: impl Fn(&Sample) -> u32,
) -> Res<()> {
    let root = SVGBackend::new(path, (760, 440)).into_drawing_area();
    root.fill(&WHITE)?;
    let xs: Vec<f64> = samples.iter().map(|s| f64::from(x_of(s)).log2()).collect();
    let (x_min, x_max) = xs
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), &x| (a.min(x), b.max(x)));
    let y_max = samples.iter().map(|s| s.hi).fold(0.0, f64::max) * 1.1;
    let mut chart = ChartBuilder::on(&root)
        .caption(title, (FONT, 20))
        .margin(16)
        .x_label_area_size(44)
        .y_label_area_size(64)
        .build_cartesian_2d(x_min - 0.3..x_max + 0.3, 0.0..y_max)?;
    chart
        .configure_mesh()
        .light_line_style(WHITE)
        .disable_x_mesh()
        .x_desc(x_desc)
        .y_desc("million items / s (higher is better)")
        .x_labels(((x_max - x_min) as usize + 1).max(2))
        .x_label_formatter(&|v| {
            let rounded = v.round();
            if (v - rounded).abs() < 1e-6 {
                format!("{}", 2f64.powf(rounded) as u32)
            } else {
                String::new()
            }
        })
        .label_style((FONT, 14))
        .draw()?;
    for (queue, name, colour) in QUEUES {
        let mut points: Vec<(f64, &Sample)> = samples
            .iter()
            .filter(|s| s.queue == queue)
            .map(|s| (f64::from(x_of(s)).log2(), s))
            .collect();
        if points.is_empty() {
            continue;
        }
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        chart
            .draw_series(LineSeries::new(
                points.iter().map(|(x, s)| (*x, s.median)),
                colour.stroke_width(2),
            ))?
            .label(name)
            .legend(move |(x, y)| {
                PathElement::new(vec![(x, y), (x + 22, y)], colour.stroke_width(3))
            });
        chart.draw_series(
            points
                .iter()
                .map(|(x, s)| Circle::new((*x, s.median), 4, colour.filled())),
        )?;
        chart.draw_series(points.iter().map(|(x, s)| {
            ErrorBar::new_vertical(*x, s.lo, s.median, s.hi, colour.stroke_width(1), 8)
        }))?;
    }
    chart
        .configure_series_labels()
        .position(SeriesLabelPosition::UpperRight)
        .background_style(WHITE.mix(0.9))
        .border_style(RGBColor(200, 200, 200))
        .label_font((FONT, 13))
        .draw()?;
    root.present()?;
    Ok(())
}

/// Grouped bar chart: one group per workload shape, one bar per queue.
fn bar_chart(path: &Path, title: &str, samples: &[Sample]) -> Res<()> {
    let mut shapes: Vec<(u32, u32)> = samples.iter().map(|s| (s.producers, s.consumers)).collect();
    shapes.sort_unstable_by(|a, b| b.cmp(a));
    shapes.dedup();
    let root = SVGBackend::new(path, (760, 440)).into_drawing_area();
    root.fill(&WHITE)?;
    let y_max = samples.iter().map(|s| s.hi).fold(0.0, f64::max) * 1.15;
    let names: Vec<String> = shapes
        .iter()
        .map(|(p, c)| format!("{p} producers / {c} consumers"))
        .collect();
    let mut chart = ChartBuilder::on(&root)
        .caption(title, (FONT, 20))
        .margin(16)
        .x_label_area_size(44)
        .y_label_area_size(64)
        .build_cartesian_2d(-0.5..shapes.len() as f64 - 0.5, 0.0..y_max)?;
    chart
        .configure_mesh()
        .light_line_style(WHITE)
        .disable_x_mesh()
        .x_labels(shapes.len())
        .x_label_formatter(&|v| {
            let i = v.round();
            if (v - i).abs() < 1e-6 && i >= 0.0 {
                names.get(i as usize).cloned().unwrap_or_default()
            } else {
                String::new()
            }
        })
        .y_desc("million items / s (higher is better)")
        .label_style((FONT, 14))
        .draw()?;
    let width = 0.8 / QUEUES.len() as f64;
    for (qi, (queue, name, colour)) in QUEUES.into_iter().enumerate() {
        let bars: Vec<_> = shapes
            .iter()
            .enumerate()
            .filter_map(|(si, shape)| {
                let s = samples
                    .iter()
                    .find(|s| s.queue == queue && (s.producers, s.consumers) == *shape)?;
                let x0 = si as f64 - 0.4 + qi as f64 * width;
                Some((x0, s))
            })
            .collect();
        if bars.is_empty() {
            continue;
        }
        chart
            .draw_series(bars.iter().map(|(x0, s)| {
                Rectangle::new(
                    [(*x0 + 0.01, 0.0), (*x0 + width - 0.01, s.median)],
                    colour.filled(),
                )
            }))?
            .label(name)
            .legend(move |(x, y)| Rectangle::new([(x, y - 5), (x + 14, y + 5)], colour.filled()));
        chart.draw_series(bars.iter().map(|(x0, s)| {
            ErrorBar::new_vertical(
                x0 + width / 2.0,
                s.lo,
                s.median,
                s.hi,
                BLACK.stroke_width(1),
                6,
            )
        }))?;
    }
    chart
        .configure_series_labels()
        .position(SeriesLabelPosition::UpperRight)
        .background_style(WHITE.mix(0.9))
        .border_style(RGBColor(200, 200, 200))
        .label_font((FONT, 13))
        .draw()?;
    root.present()?;
    Ok(())
}

/// Scatter of median wake latency against CPU burned while idle.
fn latency_chart(path: &Path, rows: &[Value]) -> Res<()> {
    let root = SVGBackend::new(path, (760, 440)).into_drawing_area();
    root.fill(&WHITE)?;
    let get = |r: &Value, k: &str| r[k].as_f64().unwrap_or(f64::NAN);
    let x_max = rows.iter().map(|r| get(r, "p50_us")).fold(0.0, f64::max) * 3.0;
    let x_min = rows
        .iter()
        .map(|r| get(r, "p50_us"))
        .fold(f64::MAX, f64::min)
        / 3.0;
    let y_max = (rows
        .iter()
        .map(|r| get(r, "idle_cpu_pct"))
        .fold(0.0, f64::max)
        * 1.2)
        .max(10.0);
    let mut chart = ChartBuilder::on(&root)
        .caption(
            "Waiting consumer: wake latency vs CPU burned while idle",
            (FONT, 20),
        )
        .margin(16)
        .x_label_area_size(44)
        .y_label_area_size(64)
        .build_cartesian_2d((x_min..x_max).log_scale(), 0.0..y_max)?;
    chart
        .configure_mesh()
        .light_line_style(WHITE)
        .x_desc("median wake latency of a waiting consumer, µs (log)")
        .y_desc("process CPU while waiting, % of one core")
        .label_style((FONT, 14))
        .draw()?;
    for r in rows {
        let queue = r["queue"].as_str().unwrap_or_default().to_owned();
        let (x, y) = (get(r, "p50_us"), get(r, "idle_cpu_pct"));
        let colour = color(&queue);
        chart
            .draw_series(std::iter::once(Circle::new((x, y), 7, colour.filled())))?
            .label(format!("{} ({x:.1} µs, {y:.1}% CPU)", label(&queue)))
            .legend(move |(x, y)| Circle::new((x + 7, y), 6, colour.filled()));
    }
    chart
        .configure_series_labels()
        .position(SeriesLabelPosition::MiddleRight)
        .background_style(WHITE.mix(0.9))
        .border_style(RGBColor(200, 200, 200))
        .label_font((FONT, 13))
        .draw()?;
    root.present()?;
    Ok(())
}

fn table(title: &str, samples: &[Sample]) {
    println!("\n### {title}\n");
    println!("| queue | producers | consumers | capacity | Melem/s (median) | 95% CI |");
    println!("|---|---|---|---|---|---|");
    let mut sorted = samples.to_vec();
    sorted.sort_by_key(|s| {
        let rank = QUEUES.iter().position(|q| q.0 == s.queue).unwrap_or(9);
        (s.producers, s.consumers, s.capacity, rank)
    });
    for s in sorted {
        println!(
            "| {} | {} | {} | {} | {:.1} | {:.1}–{:.1} |",
            s.queue, s.producers, s.consumers, s.capacity, s.median, s.lo, s.hi
        );
    }
}

fn main() -> Res<()> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let target = manifest.join("target");
    let assets = manifest.join("assets");
    fs::create_dir_all(&assets)?;

    let groups = load_throughput(&target.join("criterion"))
        .map_err(|e| format!("run `cargo bench --bench throughput` first: {e}"))?;
    let group = |name: &str| -> Res<&Vec<Sample>> {
        groups
            .get(name)
            .ok_or_else(|| format!("missing benchmark group `{name}`").into())
    };

    line_chart(
        &assets.join("mpmc_scaling.svg"),
        "Throughput with N producers + N consumers (capacity 256)",
        "N (producers = consumers); 8+8 oversubscribes 10 cores",
        group("mpmc")?,
        |s| s.producers,
    )?;
    line_chart(
        &assets.join("capacity_sweep.svg"),
        "Throughput vs capacity (4 producers + 4 consumers)",
        "capacity",
        group("capacity")?,
        |s| s.capacity,
    )?;
    line_chart(
        &assets.join("spsc.svg"),
        "Single producer, single consumer",
        "capacity",
        group("spsc")?,
        |s| s.capacity,
    )?;
    bar_chart(
        &assets.join("asymmetric.svg"),
        "Asymmetric workloads (capacity 256)",
        group("asymmetric")?,
    )?;

    for (name, title) in [
        ("spsc", "SPSC"),
        ("mpmc", "MPMC scaling"),
        ("asymmetric", "Asymmetric"),
        ("capacity", "Capacity sweep"),
    ] {
        table(title, group(name)?);
    }

    let latency_path = target.join("latency/latency.json");
    match fs::read_to_string(&latency_path) {
        Ok(text) => {
            let rows: Vec<Value> = serde_json::from_str(&text)?;
            latency_chart(&assets.join("wake_latency.svg"), &rows)?;
            println!("\n### Wake latency\n");
            println!("| queue | p50 (µs) | p90 (µs) | p99 (µs) | CPU while idle |");
            println!("|---|---|---|---|---|");
            for r in &rows {
                println!(
                    "| {} | {:.1} | {:.1} | {:.1} | {:.1}% |",
                    r["queue"].as_str().unwrap_or_default(),
                    r["p50_us"].as_f64().unwrap_or_default(),
                    r["p90_us"].as_f64().unwrap_or_default(),
                    r["p99_us"].as_f64().unwrap_or_default(),
                    r["idle_cpu_pct"].as_f64().unwrap_or_default(),
                );
            }
        }
        Err(_) => eprintln!("skipping wake-latency chart: run `cargo bench --bench latency`"),
    }
    eprintln!("charts written to {}", assets.display());
    Ok(())
}
