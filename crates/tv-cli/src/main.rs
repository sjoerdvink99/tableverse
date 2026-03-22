use clap::{Parser, Subcommand};
use tracing_subscriber::{fmt, EnvFilter};
use tv_core::ViewExpr;
use tv_engine::{CodegenTarget, DownloadFormat};

#[derive(Parser)]
#[command(
    name = "tableverse",
    about = "Tableverse: high-performance tile-based table viewer"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Serve {
        #[arg(help = "Path or URI to data file (parquet, csv, arrow, json)")]
        source: Option<String>,

        #[arg(long, default_value = "8080", help = "Port to listen on")]
        port: u16,

        #[arg(long, help = "Redis URL for tile caching (optional)")]
        redis: Option<String>,

        #[arg(long, help = "Do not open browser automatically")]
        no_open: bool,

        #[arg(long, help = "Disable request logging to stderr")]
        headless: bool,

        #[arg(long, help = "Path to SQLite catalog database for session persistence")]
        persist_catalog: Option<String>,

        #[arg(long, help = "Arrow Flight server port (disabled by default)")]
        flight_port: Option<u16>,
    },
    Inspect {
        #[arg(help = "Path or URI to data file")]
        source: String,

        #[arg(long, help = "Connection profile name")]
        profile: Option<String>,
    },
    Profile {
        #[arg(help = "Path or URI to data file")]
        source: String,

        #[arg(long, help = "Connection profile name")]
        profile: Option<String>,
    },
    Export {
        #[arg(help = "Path or URI to data file")]
        source: String,

        #[arg(long, help = "Output format: parquet, csv, arrow, jsonl")]
        format: String,

        #[arg(short, long, help = "Output file path")]
        output: Option<String>,

        #[arg(long, help = "Connection profile name")]
        profile: Option<String>,

        #[arg(long, help = "Filter as JSON ViewOp array")]
        filter: Option<String>,

        #[arg(long, help = "Comma-separated column names to select")]
        columns: Option<String>,

        #[arg(long, help = "Column name to sort by")]
        sort: Option<String>,

        #[arg(long, help = "Limit number of rows")]
        limit: Option<u64>,
    },
    Codegen {
        #[arg(help = "Path or URI to data file")]
        source: String,

        #[arg(
            long,
            help = "Target: duckdb_sql, ansi_sql, pandas, polars, python_duckdb, shell, shell_csv, dbt"
        )]
        target: String,

        #[arg(long, help = "Connection profile name")]
        profile: Option<String>,

        #[arg(long, help = "Filter as JSON ViewOp array")]
        filter: Option<String>,

        #[arg(long, help = "Comma-separated column names to select")]
        columns: Option<String>,

        #[arg(long, help = "Column name to sort by")]
        sort: Option<String>,
    },
    Apply {
        #[arg(help = "Path to view.json file")]
        view_file: String,

        #[arg(long, help = "Output format: parquet, csv")]
        format: String,

        #[arg(short, long, help = "Output file path")]
        output: String,
    },
    Connect {
        #[arg(help = "Arrow Flight URI: flight://host:port/source_id")]
        uri: String,

        #[arg(long, default_value = "8080", help = "Local server port")]
        port: u16,

        #[arg(long, default_value = "100000", help = "Max rows to fetch from remote")]
        limit: u64,

        #[arg(long, help = "Do not open browser automatically")]
        no_open: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    fmt()
        .compact()
        .with_target(false)
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Serve {
            source,
            port,
            redis,
            no_open,
            headless: _headless,
            persist_catalog: _persist_catalog,
            flight_port,
        } => {
            tracing::info!(port = port, "starting tableverse server");

            let engine = tv_engine::Engine::new()?;
            if let Some(ref src) = source {
                engine.register_source(src, None, None, None).await?;
            }

            let mut actual_port = port;
            loop {
                let addr: std::net::SocketAddr = format!("0.0.0.0:{actual_port}").parse()?;
                match std::net::TcpListener::bind(addr) {
                    Ok(_) => break,
                    Err(_) if actual_port < port + 10 => actual_port += 1,
                    Err(e) => return Err(e.into()),
                }
            }
            if actual_port != port {
                tracing::info!(
                    port = actual_port,
                    "original port taken, using {}",
                    actual_port
                );
            }

            if let Some(fp) = flight_port {
                let flight_engine = std::sync::Arc::new(engine.clone());
                tokio::spawn(async move {
                    if let Err(e) =
                        tv_flight::serve(flight_engine, tv_flight::FlightServerConfig { port: fp })
                            .await
                    {
                        tracing::warn!(err = %e, "Arrow Flight server error");
                    }
                });
            }

            let url = format!("http://localhost:{actual_port}");

            if !no_open {
                tracing::info!(url = %url, "opening browser");
                if let Err(e) = open::that(&url) {
                    tracing::warn!(err = %e, "could not open browser");
                }
            }

            tv_server::serve(
                engine,
                tv_server::ServerConfig {
                    port: actual_port,
                    redis_url: redis,
                },
            )
            .await?;
        }

        Command::Inspect { source, profile } => {
            let engine = tv_engine::Engine::new()?;
            let meta = engine.register_source(&source, None, profile, None).await?;

            println!(
                "\n  {}  {} rows × {} columns\n",
                meta.name,
                format_count(meta.n_rows),
                meta.n_cols
            );

            let has_quick = !meta.quick_stats.is_empty();

            let max_name = meta
                .columns
                .iter()
                .map(|c| c.name.len())
                .max()
                .unwrap_or(6)
                .max(6);
            let max_type = meta
                .columns
                .iter()
                .map(|c| c.data_type.len())
                .max()
                .unwrap_or(4)
                .max(4);

            if has_quick {
                println!(
                    "  {:<width_n$}  {:<width_t$}  {:>7}  {:<18}  {:<18}",
                    "Column",
                    "Type",
                    "Null%",
                    "Min",
                    "Max",
                    width_n = max_name,
                    width_t = max_type
                );
                println!(
                    "  {}  {}  ───────  ──────────────────  ──────────────────",
                    "─".repeat(max_name),
                    "─".repeat(max_type)
                );
                for (col, qs) in meta.columns.iter().zip(meta.quick_stats.iter()) {
                    let null_pct = if qs.null_rate > 0.0 {
                        format!("{:.1}%", qs.null_rate * 100.0)
                    } else {
                        "0.0%".to_string()
                    };
                    let min_s = format_json_val(&qs.min);
                    let max_s = format_json_val(&qs.max);
                    println!(
                        "  {:<width_n$}  {:<width_t$}  {:>7}  {:<18}  {:<18}",
                        col.name,
                        col.data_type,
                        null_pct,
                        min_s,
                        max_s,
                        width_n = max_name,
                        width_t = max_type
                    );
                }
            } else {
                println!(
                    "  {:<width_n$}  {:<width_t$}  Nullable",
                    "Column",
                    "Type",
                    width_n = max_name,
                    width_t = max_type
                );
                println!(
                    "  {}  {}  --------",
                    "─".repeat(max_name),
                    "─".repeat(max_type)
                );
                for col in &meta.columns {
                    println!(
                        "  {:<width_n$}  {:<width_t$}  {}",
                        col.name,
                        col.data_type,
                        if col.nullable { "yes" } else { "no" },
                        width_n = max_name,
                        width_t = max_type
                    );
                }
            }
            println!();
        }

        Command::Profile { source, profile } => {
            let engine = tv_engine::Engine::new()?;
            let meta = engine.register_source(&source, None, profile, None).await?;
            let stats = engine.profile_source(&meta.id).await?;
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }

        Command::Export {
            source,
            format,
            output,
            profile,
            filter,
            columns,
            sort,
            limit,
        } => {
            let engine = tv_engine::Engine::new()?;
            let meta = engine.register_source(&source, None, profile, None).await?;

            let ops = build_ops(filter, columns, sort, limit);
            let expr = ViewExpr {
                source_id: meta.id.clone(),
                ops,
            };

            let dl_format = parse_download_format(&format)?;
            let default_output = format!("output.{}", dl_format.extension());
            let out_path = output.unwrap_or(default_output);

            let (data, _) = engine.download_view(&expr, dl_format).await?;
            std::fs::write(&out_path, data)?;
            println!("exported to {out_path}");
        }

        Command::Codegen {
            source,
            target,
            profile,
            filter,
            columns,
            sort,
        } => {
            let engine = tv_engine::Engine::new()?;
            let meta = engine.register_source(&source, None, profile, None).await?;

            let ops = build_ops(filter, columns, sort, None);
            let expr = ViewExpr {
                source_id: meta.id.clone(),
                ops,
            };

            let codegen_target = parse_codegen_target(&target)?;
            let code = engine.codegen(&expr, codegen_target)?;
            println!("{code}");
        }

        Command::Apply {
            view_file,
            format,
            output,
        } => {
            let json_str = std::fs::read_to_string(&view_file)?;
            let expr: ViewExpr = serde_json::from_str(&json_str)?;

            let engine = tv_engine::Engine::new()?;
            let dl_format = parse_download_format(&format)?;

            let (data, _) = engine.download_view(&expr, dl_format).await?;
            std::fs::write(&output, data)?;
            println!("exported to {output}");
        }

        Command::Connect {
            uri,
            port,
            limit,
            no_open,
        } => {
            let (host, flight_port, source_id) = tv_flight::parse_flight_uri(&uri)?;

            tracing::info!(
                host = %host,
                flight_port = flight_port,
                source_id = %source_id,
                "connecting to remote Flight server"
            );

            let mut client = tv_flight::FlightClient::connect(&host, flight_port)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "failed to connect to {host}:{flight_port}: {e}\n\
                     Ensure the server is running with --flight-port {flight_port}"
                    )
                })?;

            let batches = client
                .fetch_batches(&source_id, limit)
                .await
                .map_err(|e| anyhow::anyhow!("failed to fetch data from '{source_id}': {e}"))?;

            if batches.is_empty() {
                return Err(anyhow::anyhow!(
                    "remote source '{source_id}' returned no data"
                ));
            }

            let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
            let schema = batches[0].schema();
            let tmp_path = std::env::temp_dir().join(format!("tv_flight_{}.arrow", uuid_hex()));

            let write_result = (|| -> anyhow::Result<()> {
                use arrow::ipc::writer::FileWriter;
                let file = std::fs::File::create(&tmp_path)?;
                let mut writer = FileWriter::try_new(file, &schema)?;
                for batch in &batches {
                    writer.write(batch)?;
                }
                writer.finish()?;
                Ok(())
            })();

            if let Err(e) = write_result {
                let _ = std::fs::remove_file(&tmp_path);
                return Err(e);
            }

            tracing::info!(
                rows = total_rows,
                path = %tmp_path.display(),
                "wrote remote data to local Arrow file"
            );

            let engine = tv_engine::Engine::new()?;
            let local_name = format!("{}@{}:{}", source_id, host, flight_port);
            let meta = engine
                .register_source(&tmp_path.to_string_lossy(), Some(local_name), None, None)
                .await
                .inspect_err(|_| {
                    let _ = std::fs::remove_file(&tmp_path);
                })?;

            let mut actual_port = port;
            loop {
                let addr: std::net::SocketAddr = format!("0.0.0.0:{actual_port}").parse()?;
                match std::net::TcpListener::bind(addr) {
                    Ok(_) => break,
                    Err(_) if actual_port < port + 10 => actual_port += 1,
                    Err(e) => return Err(e.into()),
                }
            }

            let url = format!("http://localhost:{actual_port}/view/{}", meta.id);

            if no_open {
                println!("{url}");
            } else {
                tracing::info!(url = %url, "opening browser");
                if let Err(e) = open::that(&url) {
                    tracing::warn!(err = %e, "could not open browser");
                    println!("{url}");
                }
            }

            tv_server::serve(
                engine,
                tv_server::ServerConfig {
                    port: actual_port,
                    redis_url: None,
                },
            )
            .await?;
        }
    }

    Ok(())
}

fn build_ops(
    filter: Option<String>,
    columns: Option<String>,
    sort: Option<String>,
    limit: Option<u64>,
) -> Vec<tv_core::ViewOp> {
    let mut ops = vec![];

    if let Some(filter_json) = filter {
        if let Ok(predicate) = serde_json::from_str::<tv_core::Predicate>(&filter_json) {
            ops.push(tv_core::ViewOp::Filter { predicate });
        }
    }

    if let Some(cols_str) = columns {
        let cols: Vec<String> = cols_str.split(',').map(|s| s.trim().to_string()).collect();
        if !cols.is_empty() {
            ops.push(tv_core::ViewOp::Select { columns: cols });
        }
    }

    if let Some(sort_col) = sort {
        ops.push(tv_core::ViewOp::Sort {
            keys: vec![tv_core::SortKey {
                column: sort_col,
                descending: false,
                nulls_last: true,
            }],
        });
    }

    if let Some(n) = limit {
        ops.push(tv_core::ViewOp::Limit { n });
    }

    ops
}

fn format_json_val(v: &Option<serde_json::Value>) -> String {
    match v {
        None => "—".to_string(),
        Some(serde_json::Value::Null) => "—".to_string(),
        Some(serde_json::Value::Number(n)) => {
            if let Some(f) = n.as_f64() {
                if f.fract() == 0.0 && f.abs() < 1e15 {
                    return (f as i64).to_string();
                }
                if f.abs() >= 1e6 || (f.abs() < 1e-3 && f != 0.0) {
                    return format!("{:.3e}", f);
                }
                format!("{:.4}", f)
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_string()
            } else {
                n.to_string()
            }
        }
        Some(serde_json::Value::String(s)) => {
            if s.len() > 16 {
                format!("{}…", &s[..15])
            } else {
                s.clone()
            }
        }
        Some(v) => v.to_string(),
    }
}

fn format_count(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

fn parse_download_format(s: &str) -> anyhow::Result<DownloadFormat> {
    match s {
        "parquet" => Ok(DownloadFormat::Parquet),
        "csv" => Ok(DownloadFormat::Csv),
        "arrow" => Ok(DownloadFormat::Arrow),
        "jsonl" => Ok(DownloadFormat::Jsonl),
        other => Err(anyhow::anyhow!("unknown format: {other}")),
    }
}

fn uuid_hex() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{:08x}{:08x}", t, std::process::id())
}

fn parse_codegen_target(s: &str) -> anyhow::Result<CodegenTarget> {
    match s {
        "duckdb_sql" => Ok(CodegenTarget::DuckdbSql),
        "ansi_sql" => Ok(CodegenTarget::AnsiSql),
        "pandas" => Ok(CodegenTarget::PythonPandas),
        "polars" => Ok(CodegenTarget::PythonPolars),
        "python_duckdb" => Ok(CodegenTarget::PythonDuckdb),
        "shell" => Ok(CodegenTarget::Shell),
        "shell_csv" => Ok(CodegenTarget::ShellCsv),
        "dbt" => Ok(CodegenTarget::Dbt),
        other => Err(anyhow::anyhow!("unknown target: {other}")),
    }
}
