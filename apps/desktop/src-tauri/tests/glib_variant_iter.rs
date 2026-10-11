// SPDX-License-Identifier: MPL-2.0
//! Real GLib FFI regressions for RUSTSEC-2024-0429. Run this target with
//! --release as well: a debug result does not establish optimized correctness.
#![cfg(target_os = "linux")]

use glib::{prelude::*, Variant};

fn sample() -> Variant {
    eprintln!("GLIB_VARIANT_OPTIMIZED={}", !cfg!(debug_assertions));
    std::hint::black_box(vec!["", "喵呜", "alpha", "tail"]).to_variant()
}

#[test]
fn next_reads_real_glib_strings_and_fuses_at_the_end() {
    let value = sample();
    let mut iter = value.array_iter_str().unwrap();
    for expected in ["", "喵呜", "alpha", "tail"] {
        assert_eq!(iter.next(), Some(expected));
    }
    assert_eq!(iter.len(), 0);
    assert_eq!(iter.next(), None);
    assert_eq!(iter.next_back(), None);
}

#[test]
fn next_back_keeps_front_and_back_boundaries_distinct() {
    let value = sample();
    let mut iter = value.array_iter_str().unwrap();
    assert_eq!(iter.next_back(), Some("tail"));
    assert_eq!(iter.next(), Some(""));
    assert_eq!(iter.next_back(), Some("alpha"));
    assert_eq!(iter.next_back(), Some("喵呜"));
    assert_eq!(iter.next_back(), None);
    assert_eq!(iter.next(), None);
}

#[test]
fn nth_reads_skipped_positions_and_rejects_out_of_range() {
    let value = sample();
    let mut iter = value.array_iter_str().unwrap();
    assert_eq!(iter.nth(1), Some("喵呜"));
    assert_eq!(iter.len(), 2);
    assert_eq!(iter.nth(0), Some("alpha"));
    assert_eq!(iter.nth(1), None);
    assert_eq!(iter.next_back(), None);
}

#[test]
fn nth_back_reads_skipped_positions_and_rejects_out_of_range() {
    let value = sample();
    let mut iter = value.array_iter_str().unwrap();
    assert_eq!(iter.nth_back(1), Some("alpha"));
    assert_eq!(iter.len(), 2);
    assert_eq!(iter.nth_back(0), Some("喵呜"));
    assert_eq!(iter.nth_back(1), None);
    assert_eq!(iter.next(), None);
}

#[test]
fn last_reads_the_remaining_tail_and_handles_empty_arrays() {
    let value = sample();
    assert_eq!(value.array_iter_str().unwrap().last(), Some("tail"));
    let mut iter = value.array_iter_str().unwrap();
    assert_eq!(iter.next(), Some(""));
    assert_eq!(iter.next_back(), Some("tail"));
    assert_eq!(iter.last(), Some("alpha"));
    let empty = Vec::<String>::new().to_variant();
    assert_eq!(empty.array_iter_str().unwrap().last(), None);
}
