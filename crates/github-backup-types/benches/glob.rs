// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Criterion benchmarks for [`github_backup_types::glob::glob_match`].
//!
//! Run with:
//! ```text
//! cargo bench -p github-backup-types
//! ```

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use github_backup_types::glob::glob_match;

fn bench_glob_match(c: &mut Criterion) {
    let cases: &[(&str, &str, &str)] = &[
        // (label, pattern, text)
        ("exact_match", "hello-world", "hello-world"),
        ("star_prefix", "rust-*", "rust-backup"),
        ("star_suffix", "*-backup", "github-backup"),
        ("star_both", "*backup*", "github-backup-tool"),
        ("question_mark", "repo-?", "repo-1"),
        ("multi_star", "a*b*c", "axbycz"),
        ("no_match", "rust-*", "python-tool"),
        // Adversarial case that would catastrophically backtrack in a recursive
        // implementation — must complete in O(m×n) with the DP approach.
        // Trailing '*' absorbs the 'b', so this is a match.
        (
            "adversarial_backtrack_match",
            "*a*a*a*a*a*a*a*a*a*a*",
            "aaaaaaaaaaaaaaaaaaaaab",
        ),
        // Same adversarial input without trailing '*' — no match.
        (
            "adversarial_backtrack_nomatch",
            "*a*a*a*a*a*a*a*a*a*a",
            "aaaaaaaaaaaaaaaaaaaaab",
        ),
    ];

    let mut group = c.benchmark_group("glob_match");

    for (label, pattern, text) in cases {
        group.bench_with_input(
            BenchmarkId::new("pattern", label),
            &(*pattern, *text),
            |b, (pat, txt)| {
                b.iter(|| glob_match(black_box(pat), black_box(txt)));
            },
        );
    }

    group.finish();
}

fn bench_long_inputs(c: &mut Criterion) {
    // Test scaling behaviour with increasingly long repository names.
    let patterns = vec![
        ("short", "rust-backup"),
        ("medium", "rust-backup-tool-v2"),
        ("long", "github-rust-backup-comprehensive-mirror-tool"),
    ];

    let mut group = c.benchmark_group("glob_match_long_inputs");

    for (label, text) in patterns {
        group.bench_with_input(BenchmarkId::new("text_length", label), text, |b, txt| {
            b.iter(|| glob_match(black_box("*backup*"), black_box(txt)));
        });
    }

    group.finish();
}

criterion_group!(benches, bench_glob_match, bench_long_inputs);
criterion_main!(benches);
