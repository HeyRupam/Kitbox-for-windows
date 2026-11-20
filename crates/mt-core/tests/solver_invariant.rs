//! The one promise everything else rests on.
//!
//! Kitbox exists because "compress this to between 20 and 50 KB" should be a
//! statement rather than an attempt. Every screen, the command line, the
//! right-click menu and the recipes all end up in the same solver, so there is
//! exactly one invariant worth testing hardest:
//!
//! > For any file and any target, the result is a `Hit` inside the range, a
//! > `Padded` inside the range, or an `Infeasible` that says so. Never a
//! > success that is outside what was asked for.
//!
//! A unit test checks a handful of cases someone thought of. This checks two
//! hundred pairs the author did not choose, and when it fails it prints the
//! seed so the failing pair can be reproduced exactly.
//!
//! Run with: cargo test -p mt-core --release --features testkit --test solver_invariant

use mt_core::engine::image::{compress_to_target, CompressOptions, OutputFormat};
use mt_core::engine::pdf::{self, PdfCompressOptions};
use mt_core::{format_size, Bias, FileKind, Outcome, SizeTarget};

/// A small deterministic generator, so a failure can be reproduced from the
/// seed printed beside it rather than "it happened once on a Tuesday".
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*: short, well-spread, and nobody has to take a dependency
        // on a crate to run the test suite.
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn between(&mut self, low: u64, high: u64) -> u64 {
        low + self.next() % (high - low + 1)
    }

    fn pick<'a, T>(&mut self, options: &'a [T]) -> &'a T {
        &options[(self.next() % options.len() as u64) as usize]
    }
}

/// What the solver was asked, in words, for a failure message.
fn describe(kind: &str, seed: u64, bytes: usize, target: SizeTarget, bias: Bias) -> String {
    format!(
        "seed {seed}: a {kind} of {} against {} ({bias:?})",
        format_size(bytes as u64),
        target.describe()
    )
}

/// The full sweep takes about three and a half minutes, which is too long to
/// sit in front of after every edit — so the suite runs a short one and this
/// runs the plan's two hundred pairs on demand and in CI.
///
///     cargo test -p mt-core --release --features testkit \
///         --test solver_invariant -- --ignored --nocapture
#[test]
#[ignore = "the full sweep; the quick one runs by default"]
fn two_hundred_pairs_never_land_outside_the_target() {
    sweep(200, 0x5EED_1234_ABCD_0001);
}

/// Enough pairs to catch a solver that has started lying, fast enough to run
/// with everything else.
#[test]
fn a_result_is_never_outside_what_was_asked_for() {
    sweep(24, 0x5EED_1234_ABCD_0001);
}

fn sweep(cases: u32, seed: u64) {
    let mut rng = Rng(seed);
    let mut hits = 0;
    let mut padded = 0;
    let mut refused = 0;

    for case in 0..cases {
        let seed = rng.0;

        // A spread of real-ish inputs: phone photos, scans, small crops.
        let (w, h) = match rng.between(0, 3) {
            0 => (rng.between(200, 600) as u32, rng.between(200, 600) as u32),
            1 => (rng.between(800, 1600) as u32, rng.between(600, 1200) as u32),
            2 => (rng.between(1600, 3000) as u32, rng.between(1200, 2200) as u32),
            _ => (rng.between(240, 320) as u32, rng.between(240, 420) as u32),
        };
        let quality = rng.between(60, 96) as u8;
        let bytes = mt_core::testkit::encode_jpeg(&mt_core::testkit::photo(w, h), quality);

        // Targets from the plausible to the impossible, deliberately: a target
        // nothing can reach is the case most likely to be answered wrongly.
        let target = match rng.between(0, 3) {
            0 => SizeTarget::AtMost(rng.between(20_000, 400_000)),
            1 => {
                let min = rng.between(10_000, 120_000);
                SizeTarget::Range { min, max: min + rng.between(5_000, 200_000) }
            }
            2 => SizeTarget::Approx { bytes: rng.between(30_000, 300_000), tol: 0.1 },
            // Absurdly small, so `Infeasible` has to be reached honestly.
            _ => SizeTarget::AtMost(rng.between(300, 4_000)),
        };
        let bias = *rng.pick(&[Bias::Balanced, Bias::KeepSharp, Bias::ShrinkPixels]);

        let outcome = compress_to_target(
            &bytes,
            target,
            bias,
            CompressOptions {
                format: OutputFormat::Keep,
                strip_metadata: true,
                max_dimension: None,
                exact_dimensions: None,
            },
        )
        .unwrap_or_else(|e| panic!("{}: {e}", describe("photo", seed, bytes.len(), target, bias)));

        let what = describe("photo", seed, bytes.len(), target, bias);
        match &outcome {
            Outcome::Hit(solved) => {
                hits += 1;
                assert!(
                    target.contains(solved.size),
                    "{what}: reported a hit at {} which is outside the target",
                    format_size(solved.size)
                );
                assert_eq!(
                    solved.size,
                    solved.bytes.len() as u64,
                    "{what}: the reported size and the bytes disagree"
                );
                assert!(
                    FileKind::sniff(&solved.bytes).is_image(),
                    "{what}: the result isn't a picture any more"
                );
            }
            Outcome::Padded(solved) => {
                padded += 1;
                assert!(
                    target.contains(solved.size),
                    "{what}: padded to {}, still outside the target",
                    format_size(solved.size)
                );
                // Padding exists to clear a floor. Doing it when there is no
                // floor would just be making files bigger.
                assert!(target.min_bytes() > 0, "{what}: padded against a target with no minimum");
            }
            Outcome::Infeasible { best, best_size, why, .. } => {
                refused += 1;
                assert!(
                    !target.contains(*best_size),
                    "{what}: refused with a result that was inside the target all along"
                );
                assert!(!best.is_empty(), "{what}: refused without offering its best attempt");
                assert!(!why.is_empty(), "{what}: refused without saying why");
            }
        }

        // Whatever happened, what comes back has to be a usable file.
        assert!(!outcome.bytes().is_empty(), "{what}: produced nothing at all (case {case})");
    }

    println!("{cases} pairs: {hits} hit, {padded} padded into range, {refused} honestly refused");
    // Proportions rather than counts, so the two sweeps hold the same line.
    assert!(hits * 2 > cases, "only {hits} of {cases} landed inside — the solver has got worse");
    assert!(
        refused > 0 || cases < 50,
        "no impossible target was refused; the test has stopped testing that"
    );
}

#[test]
fn the_same_invariant_holds_for_documents() {
    let mut rng = Rng(0xD0C5_0000_1111_2222);

    for _ in 0..12 {
        let seed = rng.0;
        let pages = rng.between(1, 4) as usize;
        let pdf = mt_core::testkit::pdf_scan(
            pages,
            rng.between(500, 1200) as u32,
            rng.between(700, 1600) as u32,
            rng.between(70, 95) as u8,
        );
        let target = match rng.between(0, 2) {
            0 => SizeTarget::AtMost(rng.between(40_000, 400_000)),
            1 => {
                let min = rng.between(20_000, 90_000);
                SizeTarget::Range { min, max: min + rng.between(20_000, 200_000) }
            }
            _ => SizeTarget::AtMost(rng.between(1_000, 8_000)),
        };

        let outcome = pdf::compress_to_target(
            &pdf,
            target,
            Bias::Balanced,
            PdfCompressOptions { strip_metadata: true, ..Default::default() },
        )
        .unwrap_or_else(|e| {
            panic!("{}: {e}", describe("scan", seed, pdf.len(), target, Bias::Balanced))
        });

        let what = describe("scan", seed, pdf.len(), target, Bias::Balanced);
        if let Some(solved) = outcome.solved() {
            assert!(
                target.contains(solved.size),
                "{what}: succeeded at {}, outside the target",
                format_size(solved.size)
            );
            // A PDF that comes back as something else would be a disaster the
            // size check alone would not catch.
            assert_eq!(
                FileKind::sniff(&solved.bytes),
                FileKind::Pdf,
                "{what}: the result isn't a PDF any more"
            );
            assert!(
                pdf::page_count(&solved.bytes).unwrap_or(0) as usize == pages,
                "{what}: pages went missing"
            );
        }
    }
}
