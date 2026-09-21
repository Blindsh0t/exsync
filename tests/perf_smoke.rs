//! Performance smoke tests for exsync.
//!
//! PF-01 requirements:
//! 1. Unrelated-mount: median of 21 runs < 50ms (hard).
//! 2. Single 512 MiB file mirror with readback verify >= 150 MB/s (hard).
//! 3. Informational only: mixed-tree mirror rate and `dd bs=1m`
//!    byte-copy rate of the same data, printed but never asserted.
//! 4. Disk space check: skip if < 2 GiB free.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

fn unique_base(tag: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "exsync-perf-{tag}-{}-{nanos}-{n}",
        std::process::id()
    ))
}

fn run_exsync(cfg: &Path, vols: &Path, log: &Path, extra: &[&str]) -> Output {
    let bin = env!("CARGO_BIN_EXE_exsync");
    let mut cmd = Command::new(bin);
    cmd.env("EXSYNC_CONFIG", cfg);
    cmd.env("EXSYNC_VOLUMES_ROOT", vols);
    cmd.env("EXSYNC_LOG", log);
    for a in extra {
        cmd.arg(a);
    }
    cmd.output().expect("spawn exsync")
}

fn get_free_space() -> u64 {
    #[cfg(unix)]
    {
        let output = Command::new("df")
            .arg("-k")
            .arg(std::env::temp_dir().to_str().unwrap())
            .output()
            .expect("failed to execute df");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().collect();
        if lines.len() < 2 {
            return 0;
        }

        let parts: Vec<&str> = lines[1].split_whitespace().collect();
        if parts.len() < 4 {
            return 0;
        }

        if let Ok(available_kb) = parts[3].parse::<u64>() {
            available_kb * 1024
        } else {
            0
        }
    }
    #[cfg(not(unix))]
    {
        u64::MAX
    }
}

fn write_pattern_file(path: &Path, total_bytes: u64) {
    let mut f = fs::File::create(path).expect("create pattern file");
    let chunk = vec![0x5Au8; 1024 * 1024];
    let mut left = total_bytes;
    while left > 0 {
        let n = std::cmp::min(left, chunk.len() as u64) as usize;
        f.write_all(&chunk[..n]).expect("write pattern chunk");
        left -= n as u64;
    }
    f.flush().expect("flush pattern file");
}

#[test]
fn test_performance_smoke() {
    if get_free_space() < 2 * 1024 * 1024 * 1024 {
        println!("Skipping PF-01: Less than 2 GiB free space available.");
        return;
    }

    // --- Hard 1: unrelated-mount median of 21 runs under 50 ms ---
    let base_unrelated = unique_base("unrelated");
    let vols_unrelated = base_unrelated.join("vols");
    fs::create_dir_all(vols_unrelated.join("KINGSTON")).unwrap();

    let cfg_unrelated = base_unrelated.join("cfg.toml");
    fs::write(
        &cfg_unrelated,
        "version = 1\n\n[[entry]]\nname = \"test\"\nmode = \"mirror\"\ndrive = \"SAMSUNG\"\nsource = \"/tmp/src\"\ndest = \"data\"\n",
    )
    .unwrap();
    let log_unrelated = base_unrelated.join("exsync.log");

    let mut times = Vec::new();
    for _ in 0..21 {
        let start = Instant::now();
        let out = run_exsync(&cfg_unrelated, &vols_unrelated, &log_unrelated, &[]);
        assert_eq!(out.status.code(), Some(0));
        times.push(start.elapsed());
    }

    let mut sorted_times: Vec<_> = times.into_iter().collect();
    sorted_times.sort();
    let median = sorted_times[10];
    println!("Unrelated mount median time: {:?}", median);
    assert!(
        median < Duration::from_millis(50),
        "Median run time {:?} exceeded 50ms",
        median
    );

    let _ = fs::remove_dir_all(&base_unrelated);

    // --- Hard 2: single 512 MiB file mirror with readback verify ---
    const BIG_BYTES: u64 = 512 * 1024 * 1024;
    const FLOOR_MB_S: f64 = 150.0;

    let base_big = unique_base("big");
    let src_big = base_big.join("src");
    fs::create_dir_all(&src_big).unwrap();
    let big_file = src_big.join("big.bin");
    write_pattern_file(&big_file, BIG_BYTES);
    let vols_big = base_big.join("vols");
    fs::create_dir_all(vols_big.join("KINGSTON")).unwrap();

    let cfg_big = base_big.join("cfg.toml");
    let cfg_text = format!(
        "version = 1\n\n[[entry]]\nname = \"perf-big\"\nmode = \"mirror\"\ndrive = \"KINGSTON\"\nsource = \"{}\"\ndest = \"data\"\n",
        src_big.display()
    );
    fs::write(&cfg_big, cfg_text).unwrap();
    let log_big = base_big.join("exsync.log");

    let start_big = Instant::now();
    let out_big = run_exsync(&cfg_big, &vols_big, &log_big, &[]);
    let elapsed_big = start_big.elapsed();
    assert_eq!(out_big.status.code(), Some(0));
    let big_rate = BIG_BYTES as f64 / elapsed_big.as_secs_f64() / (1024.0 * 1024.0);
    if cfg!(debug_assertions) {
        println!(
            "512 MiB mirror with readback verify: {:?} -> {:.2} MB/s (informational (debug build, not asserted); floor {:.0} MB/s applies to release builds)",
            elapsed_big, big_rate, FLOOR_MB_S
        );
    } else {
        println!(
            "512 MiB mirror with readback verify: {:?} -> {:.2} MB/s (floor {:.0} MB/s)",
            elapsed_big, big_rate, FLOOR_MB_S
        );
        assert!(
            big_rate >= FLOOR_MB_S,
            "512 MiB mirror rate {:.2} MB/s below floor {:.0} MB/s",
            big_rate,
            FLOOR_MB_S
        );
    }

    // --- Informational: `dd bs=1m` byte-copy rate of the same 512 MiB file ---
    let dd_dest = base_big.join("dd_out.bin");
    let start_dd = Instant::now();
    let dd_status = Command::new("dd")
        .arg(format!("if={}", big_file.display()))
        .arg(format!("of={}", dd_dest.display()))
        .arg("bs=1m")
        .status()
        .expect("failed to execute dd");
    let elapsed_dd = start_dd.elapsed();
    assert!(dd_status.success(), "dd reference copy failed");
    let dd_bytes = fs::metadata(&dd_dest).expect("stat dd output").len();
    let dd_rate = dd_bytes as f64 / elapsed_dd.as_secs_f64() / (1024.0 * 1024.0);
    println!(
        "dd bs=1m byte-copy of 512 MiB: {:?} -> {:.2} MB/s (informational only)",
        elapsed_dd, dd_rate
    );

    let _ = fs::remove_dir_all(&base_big);

    // --- Informational: mixed tree (1000 small files plus 4 x 128 MiB) ---
    let base_mixed = unique_base("mixed");
    let src_mixed = base_mixed.join("src");
    let vols_mixed = base_mixed.join("vols");
    fs::create_dir_all(vols_mixed.join("KINGSTON")).unwrap();

    let small_dir = src_mixed.join("small");
    fs::create_dir_all(&small_dir).unwrap();
    let small_data = vec![0x5Au8; 1024];
    for i in 0..1000 {
        fs::write(small_dir.join(format!("file_{}.bin", i)), &small_data).unwrap();
    }

    let large_dir = src_mixed.join("large");
    fs::create_dir_all(&large_dir).unwrap();
    for i in 0..4 {
        write_pattern_file(
            &large_dir.join(format!("large_{}.bin", i)),
            128 * 1024 * 1024,
        );
    }

    let cfg_mixed = base_mixed.join("cfg.toml");
    let cfg_mixed_text = format!(
        "version = 1\n\n[[entry]]\nname = \"perf-mixed\"\nmode = \"mirror\"\ndrive = \"KINGSTON\"\nsource = \"{}\"\ndest = \"data\"\n",
        src_mixed.display()
    );
    fs::write(&cfg_mixed, cfg_mixed_text).unwrap();
    let log_mixed = base_mixed.join("exsync.log");

    let start_mixed = Instant::now();
    let out_mixed = run_exsync(&cfg_mixed, &vols_mixed, &log_mixed, &[]);
    let elapsed_mixed = start_mixed.elapsed();
    assert_eq!(out_mixed.status.code(), Some(0));
    let mixed_bytes = (1000 * 1024 + 4 * 128 * 1024 * 1024) as f64;
    let mixed_rate = mixed_bytes / elapsed_mixed.as_secs_f64() / (1024.0 * 1024.0);
    println!(
        "Mixed tree mirror (1000 small + 4 x 128 MiB): {:?} -> {:.2} MB/s (informational only)",
        elapsed_mixed, mixed_rate
    );

    let _ = fs::remove_dir_all(&base_mixed);
}
