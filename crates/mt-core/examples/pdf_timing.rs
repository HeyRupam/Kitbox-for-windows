//! Where the time goes when a scan is compressed.
//!
//! The corpus benchmark says a single-page scan to 100 KB takes about eleven
//! seconds and ten encodes. This says which part of one encode is to blame,
//! because the fix for "it decodes too often" is nothing like the fix for "it
//! saves the document too often".
//!
//! cargo run -p mt-core --release --features testkit --example pdf_timing

use std::time::Instant;

use mt_core::engine::pdf::{self, PdfCompressOptions};
use mt_core::{Bias, SizeTarget};

fn main() {
    let scan = mt_core::testkit::pdf_scan(1, 1700, 2200, 90);
    println!("a one-page scan of {} bytes\n", scan.len());

    // The whole job, for a number to compare the parts against.
    let whole = Instant::now();
    let outcome = pdf::compress_to_target(
        &scan,
        SizeTarget::AtMost(100_000),
        Bias::Balanced,
        PdfCompressOptions { strip_metadata: true, ..Default::default() },
    )
    .expect("it should answer");
    let whole = whole.elapsed();

    let solved = outcome.solved().expect("a hit");
    println!(
        "all of it:        {:>7} ms   {} probes, {} bytes out",
        whole.as_millis(),
        solved.probes,
        solved.size
    );
    println!(
        "per probe:        {:>7} ms\n",
        whole.as_millis() / solved.probes.max(1) as u128
    );

    // Now the pieces, each timed on its own.
    let built = Instant::now();
    let compressor = pdf::PdfCompressor::new(&scan, PdfCompressOptions::default())
        .expect("a compressor");
    println!("opening it:       {:>7} ms   (load, inspect, lossless pass)", built.elapsed().as_millis());

    let knob = pdf::PdfKnob { lossless_only: false, dpi: 150, quality: 60, grayscale: false };

    // First encode pays for the decode; the rest should not.
    let first = Instant::now();
    let out = mt_core::Encodable::encode(&compressor, &knob).expect("an encode");
    println!("first encode:     {:>7} ms   {} bytes", first.elapsed().as_millis(), out.len());

    let second = Instant::now();
    let _ = mt_core::Encodable::encode(&compressor, &knob).expect("an encode");
    println!("same again:       {:>7} ms   (decode cached)", second.elapsed().as_millis());

    // A different quality at the same resolution: the resize is the same work.
    let other_quality = pdf::PdfKnob { quality: 40, ..knob };
    let third = Instant::now();
    let _ = mt_core::Encodable::encode(&compressor, &other_quality).expect("an encode");
    println!("another quality:  {:>7} ms   (same resolution)", third.elapsed().as_millis());

    let other_dpi = pdf::PdfKnob { dpi: 100, ..knob };
    let fourth = Instant::now();
    let _ = mt_core::Encodable::encode(&compressor, &other_dpi).expect("an encode");
    println!("another dpi:      {:>7} ms   (a new resize)", fourth.elapsed().as_millis());
}
