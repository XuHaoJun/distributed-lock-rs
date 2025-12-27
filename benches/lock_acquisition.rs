//! Benchmarks for lock acquisition latency

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use distributed_lock_file::FileLockProvider;
use distributed_lock_core::prelude::*;
use std::time::Duration;
use tempfile::TempDir;

fn bench_file_lock_acquisition(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let provider = FileLockProvider::builder()
        .build(temp_dir.path())
        .unwrap();
    
    let lock = provider.create_lock("bench-lock");

    let mut group = c.benchmark_group("file_lock");
    group.bench_function("try_acquire", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| async {
                if let Ok(Some(handle)) = lock.try_acquire().await {
                    let _ = handle.release().await;
                }
            });
    });
    
    group.bench_function("acquire_no_wait", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| async {
                if let Ok(handle) = lock.acquire(Some(Duration::from_millis(1))).await {
                    let _ = handle.release().await;
                }
            });
    });
    
    group.finish();
}

criterion_group!(benches, bench_file_lock_acquisition);
criterion_main!(benches);
