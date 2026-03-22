use std::future::Future;
use std::time::Instant;

use clap::Parser;
use serde_json::json;
use tv_core::{Literal, Predicate, ViewExpr, ViewOp};
use tv_engine::Engine;

#[derive(Parser)]
#[command(name = "tv-bench")]
struct Args {
    path: String,

    #[arg(long)]
    scale: String,

    #[arg(long, default_value = "bench/results")]
    out: String,
}

struct Sample {
    median_ms: f64,
    min_ms: f64,
    max_ms: f64,
}

async fn measure<F, Fut>(mut f: F, warmup: usize, iters: usize) -> Sample
where
    F: FnMut() -> Fut,
    Fut: Future,
{
    for _ in 0..warmup {
        f().await;
    }
    let mut times: Vec<f64> = Vec::with_capacity(iters);
    for _ in 0..iters {
        let t = Instant::now();
        f().await;
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = times.len() / 2;
    let median = if times.len() % 2 == 0 {
        (times[mid - 1] + times[mid]) / 2.0
    } else {
        times[mid]
    };
    Sample {
        median_ms: (median * 1000.0).round() / 1000.0,
        min_ms: (times[0] * 1000.0).round() / 1000.0,
        max_ms: (times[times.len() - 1] * 1000.0).round() / 1000.0,
    }
}

fn record(
    results: &mut Vec<serde_json::Value>,
    tool: &str,
    op: &str,
    scale: &str,
    n_rows: u64,
    s: &Sample,
) {
    results.push(json!({
        "tool": tool, "op": op, "scale": scale, "n_rows": n_rows,
        "median_ms": s.median_ms, "min_ms": s.min_ms, "max_ms": s.max_ms,
    }));
    eprintln!(
        "    {:.3}ms (min={:.3} max={:.3})",
        s.median_ms, s.min_ms, s.max_ms
    );
}

fn rss_mb() -> f64 {
    #[cfg(target_os = "macos")]
    {
        use std::mem;
        #[repr(C)]
        struct MachTaskBasicInfo {
            virtual_size: u64,
            resident_size: u64,
            resident_size_max: u64,
            user_time: [i32; 2],
            system_time: [i32; 2],
            policy: i32,
            suspend_count: i32,
        }
        extern "C" {
            fn mach_task_self() -> u32;
            fn task_info(
                target_task: u32,
                flavor: u32,
                task_info_out: *mut MachTaskBasicInfo,
                task_info_outCnt: *mut u32,
            ) -> i32;
        }
        const MACH_TASK_BASIC_INFO: u32 = 20;
        let mut info: MachTaskBasicInfo = unsafe { mem::zeroed() };
        let mut count = (mem::size_of::<MachTaskBasicInfo>() / mem::size_of::<u32>()) as u32;
        let ret = unsafe {
            task_info(
                mach_task_self(),
                MACH_TASK_BASIC_INFO,
                &mut info,
                &mut count,
            )
        };
        if ret == 0 {
            info.resident_size as f64 / 1024.0 / 1024.0
        } else {
            0.0
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/self/status") {
            for line in s.lines() {
                if line.starts_with("VmRSS:") {
                    if let Some(kb) = line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse::<f64>().ok())
                    {
                        return kb / 1024.0;
                    }
                }
            }
        }
        0.0
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    if !std::path::Path::new(&args.path).exists() {
        eprintln!("File not found: {}", args.path);
        std::process::exit(1);
    }

    std::fs::create_dir_all(&args.out)?;

    let engine = Engine::new()?;
    let meta = engine.register_source(&args.path, None, None, None).await?;
    let n_rows = meta.n_rows;
    let source_id = meta.id.clone();
    let filter_threshold = (n_rows as f64 * 0.95) as i64;

    eprintln!("  scale={} n_rows={}", args.scale, n_rows);

    let mut results: Vec<serde_json::Value> = Vec::new();

    eprintln!("  op=open");
    let path = args.path.clone();
    let s = measure(
        || {
            let p = path.clone();
            async move {
                Engine::new()
                    .unwrap()
                    .register_source(&p, None, None, None)
                    .await
                    .unwrap();
            }
        },
        2,
        5,
    )
    .await;
    record(&mut results, "tableverse", "open", &args.scale, n_rows, &s);

    eprintln!("  op=first_tile");
    let expr_plain = ViewExpr {
        source_id: source_id.clone(),
        ops: vec![],
    };
    let n_cols = meta.n_cols;
    let s = measure(
        || {
            let e = engine.clone();
            let expr = expr_plain.clone();
            async move {
                e.query_view_tile(&expr, 0, 0, 256, n_cols).await.unwrap();
            }
        },
        2,
        5,
    )
    .await;
    record(
        &mut results,
        "tableverse",
        "first_tile",
        &args.scale,
        n_rows,
        &s,
    );

    let rss = rss_mb();
    results.push(json!({
        "tool": "tableverse", "op": "mem_load", "scale": args.scale, "n_rows": n_rows,
        "median_ms": 0.0, "min_ms": 0.0, "max_ms": 0.0, "peak_mem_mb": rss,
    }));
    eprintln!("    RSS after first_tile: {:.1}MB", rss);

    eprintln!("  op=col_stats");
    let sid = source_id.clone();
    let s = measure(
        || {
            let e = engine.clone();
            let id = sid.clone();
            async move {
                e.column_stats(&id, 1, 50).await.unwrap();
            }
        },
        1,
        3,
    )
    .await;
    record(
        &mut results,
        "tableverse",
        "col_stats",
        &args.scale,
        n_rows,
        &s,
    );

    let has_id = meta.columns.iter().any(|c| c.name == "id");
    if has_id {
        eprintln!("  op=filter_tile");
        let expr_filter = ViewExpr {
            source_id: source_id.clone(),
            ops: vec![ViewOp::Filter {
                predicate: Predicate::Gt {
                    column: "id".to_string(),
                    value: Literal::Int(filter_threshold),
                },
            }],
        };
        let s = measure(
            || {
                let e = engine.clone();
                let expr = expr_filter.clone();
                async move {
                    e.query_view_tile(&expr, 0, 0, 256, n_cols).await.unwrap();
                }
            },
            2,
            5,
        )
        .await;
        record(
            &mut results,
            "tableverse",
            "filter_tile",
            &args.scale,
            n_rows,
            &s,
        );
    }

    let out_path = format!("{}/tv_{}.json", args.out, args.scale);
    std::fs::write(&out_path, serde_json::to_string_pretty(&results)?)?;
    eprintln!("  wrote {out_path}");

    Ok(())
}
