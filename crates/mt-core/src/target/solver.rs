use std::time::Instant;

use super::{Bias, Encodable, Outcome, Prober, SizeTarget, Solved};
use crate::error::{Error, Result};
use crate::size::format_size;
use crate::target::pad;

#[derive(Debug, Clone, Copy)]
pub struct SolveOptions {
    /// Pad a too-small result up into range when the minimum can't be reached
    /// by encoder settings alone. Lossless and reversible, but it makes the file
    /// bigger than it needs to be, so it is reported as [`Outcome::Padded`].
    pub allow_padding: bool,
    /// Hard ceiling on real encodes, as a backstop on pathological ladders.
    pub max_probes: u8,
    /// Compute a similarity score for the winning encode.
    pub score: bool,
}

impl Default for SolveOptions {
    fn default() -> Self {
        SolveOptions { allow_padding: true, max_probes: 24, score: true }
    }
}

pub fn solve<E: Encodable>(enc: &E, target: SizeTarget, bias: Bias) -> Result<Outcome> {
    solve_with(enc, target, bias, SolveOptions::default())
}

pub fn solve_with<E: Encodable>(
    enc: &E,
    target: SizeTarget,
    bias: Bias,
    opts: SolveOptions,
) -> Result<Outcome> {
    let started = Instant::now();
    let ladder = enc.ladder(bias);
    if ladder.is_empty() {
        return Err(Error::other("encoder offered no settings to try"));
    }

    let max = target.max_bytes();
    let min = target.min_bytes();
    if min > max {
        return Err(Error::invalid("size range is backwards"));
    }

    // 0. The file may already be what was asked for. Nothing beats doing
    //    nothing: no quality spent, no time spent, and the bytes someone
    //    already has are the bytes they keep.
    if let Some(original) = enc.untouched() {
        let size = original.len() as u64;
        if target.contains(size) {
            return Ok(Outcome::Hit(Solved {
                bytes: original.to_vec(),
                size,
                knob: "already inside the target, left as it was".into(),
                score: Some(1.0),
                probes: 0,
                elapsed_ms: started.elapsed().as_millis() as u64,
                note: None,
            }));
        }
    }

    let top = ladder.len() - 1;
    let mut p = Prober::new(enc, &ladder);

    // 1. Best setting first. If the untouched-quality encode already fits, we are
    //    done and nothing was given up.
    let size_top = p.size(top)?;
    if size_top <= max {
        return Ok(finish(enc, &mut p, top, size_top, min, &ladder, started, opts));
    }

    // Resolving the ladder to the exact rung is not worth paying for: rungs are
    // spaced a few percent apart, so stopping at the first result that uses most
    // of the window saves several encodes and costs no visible quality.
    let good_enough = min.max((max as f64 * 0.95) as u64);

    // 2. Smallest setting. If even that overshoots, no setting on this ladder works.
    let size_bottom = p.size(0)?;
    if size_bottom > max {
        let (idx, best_len) = p.smallest().unwrap_or((0, size_bottom as usize));
        let best = p.take(idx);
        let mut suggestions = enc.suggestions(target);
        if suggestions.is_empty() {
            suggestions.push("Try a smaller output size or a different format.".into());
        }
        return Ok(Outcome::Infeasible {
            best,
            best_size: best_len as u64,
            why: format!(
                "The smallest setting still produces {}, over the {} limit.",
                format_size(best_len as u64),
                format_size(max)
            ),
            suggestions,
            probes: p.probes,
            elapsed_ms: started.elapsed().as_millis() as u64,
        });
    }

    // 3. Bracket [lo, hi]: lo fits, hi does not. Seed the search by assuming size
    //    grows roughly geometrically along the ladder, which gets us close in one
    //    extra probe instead of three or four bisection steps.
    let mut lo = 0usize;
    let mut hi = top;
    let mut size_lo = size_bottom;
    let mut size_hi = size_top;

    if size_bottom >= good_enough {
        return Ok(finish(enc, &mut p, 0, size_bottom, min, &ladder, started, opts));
    }

    // A couple of rounds of interpolation before falling back to bisection.
    // Ladder rungs are spaced geometrically by construction, so fitting a curve
    // through the bracket usually lands within a rung or two of the answer —
    // far cheaper than halving an 80-rung ladder from scratch.
    for _ in 0..2 {
        if hi - lo <= 3 {
            break;
        }
        let Some(guess) = interpolate(lo, size_lo, hi, size_hi, max) else {
            break;
        };
        let guess = guess.clamp(lo + 1, hi - 1);
        if guess == lo || guess == hi {
            break;
        }
        let s = p.size(guess)?;
        if s <= max {
            if s >= good_enough {
                return Ok(finish(enc, &mut p, guess, s, min, &ladder, started, opts));
            }
            lo = guess;
            size_lo = s;
        } else {
            hi = guess;
            size_hi = s;
        }
    }

    while hi - lo > 1 && p.probes < opts.max_probes {
        let mid = lo + (hi - lo) / 2;
        let s = p.size(mid)?;
        if s <= max {
            if s >= good_enough {
                return Ok(finish(enc, &mut p, mid, s, min, &ladder, started, opts));
            }
            lo = mid;
        } else {
            hi = mid;
        }
    }

    // 4. Encoder size curves are only near-monotone — JPEG quality 71 can land
    //    marginally under quality 70. Peek just above the answer so a local
    //    inversion doesn't cost real quality.
    let mut best_idx = lo;
    let mut best_size = p.size(lo)?;
    for probe in (lo + 1)..=(lo + 2).min(top) {
        if p.probes >= opts.max_probes {
            break;
        }
        let s = p.size(probe)?;
        if s <= max && s > best_size {
            best_idx = probe;
            best_size = s;
        }
    }

    Ok(finish(enc, &mut p, best_idx, best_size, min, &ladder, started, opts))
}

/// Wrap up a chosen ladder index: pad it if it fell under the minimum, score it,
/// and build the report.
fn finish<E: Encodable>(
    enc: &E,
    p: &mut Prober<'_, E>,
    idx: usize,
    size: u64,
    min: u64,
    ladder: &[E::Knob],
    started: Instant,
    opts: SolveOptions,
) -> Outcome {
    let bytes = p.take(idx);
    let knob = enc.describe(&ladder[idx]);
    let score = if opts.score { enc.score(&bytes) } else { None };
    let probes = p.probes;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    if size >= min {
        return Outcome::Hit(Solved {
            size: bytes.len() as u64,
            bytes,
            knob,
            score,
            probes,
            elapsed_ms,
            note: None,
        });
    }

    // Under the required minimum even at this setting. Pad losslessly if we can.
    if opts.allow_padding {
        if let Some(padded) = pad::pad_to(&bytes, min) {
            let size = padded.len() as u64;
            return Outcome::Padded(Solved {
                bytes: padded,
                size,
                knob,
                score,
                probes,
                elapsed_ms,
                note: Some(format!(
                    "Best quality came to {}, under the {} minimum, so the file was padded with \
                     ignored data to reach it. The picture is unchanged.",
                    format_size(size),
                    format_size(min)
                )),
            });
        }
    }

    Outcome::Hit(Solved {
        size: bytes.len() as u64,
        bytes,
        knob,
        score,
        probes,
        elapsed_ms,
        note: Some(format!(
            "This is the largest this file gets at full quality — {} under the {} minimum.",
            format_size(min.saturating_sub(size)),
            format_size(min)
        )),
    })
}

/// Predict the ladder index whose output lands on `target`, assuming log-size is
/// linear in index. Returns `None` when the endpoints give us nothing to fit.
fn interpolate(lo: usize, size_lo: u64, hi: usize, size_hi: u64, target: u64) -> Option<usize> {
    if size_lo == 0 || size_hi <= size_lo || target < size_lo {
        return None;
    }
    let l = (size_lo as f64).ln();
    let h = (size_hi as f64).ln();
    let t = (target as f64).ln();
    let frac = ((t - l) / (h - l)).clamp(0.0, 1.0);
    let span = (hi - lo) as f64;
    Some(lo + (frac * span).round() as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic encoder whose output size is exactly its knob value, so the
    /// solver's behaviour can be checked without touching a real codec.
    struct Fake {
        sizes: Vec<u64>,
    }

    impl Encodable for Fake {
        type Knob = u64;
        fn ladder(&self, _bias: Bias) -> Vec<u64> {
            self.sizes.clone()
        }
        fn encode(&self, knob: &u64) -> Result<Vec<u8>> {
            Ok(vec![0u8; *knob as usize])
        }
        fn describe(&self, knob: &u64) -> String {
            format!("{knob} bytes")
        }
    }

    fn ramp() -> Fake {
        Fake { sizes: (1..=200).map(|i| i * 1_000).collect() }
    }

    #[test]
    fn returns_best_quality_when_it_already_fits() {
        let out = solve(&ramp(), SizeTarget::AtMost(500_000), Bias::Balanced).unwrap();
        let s = out.solved().unwrap();
        assert_eq!(s.size, 200_000);
        assert_eq!(s.probes, 1, "a file that already fits should cost one encode");
    }

    #[test]
    fn lands_on_the_largest_output_inside_the_window() {
        let out = solve(&ramp(), SizeTarget::Range { min: 50_000, max: 100_000 }, Bias::Balanced)
            .unwrap();
        let s = out.solved().unwrap();
        assert!(s.size <= 100_000, "must never exceed the ceiling, got {}", s.size);
        assert!(
            s.size >= 95_000,
            "should use most of the window rather than scraping the bottom, got {}",
            s.size
        );
        assert!(s.probes <= 7, "took {} probes on a 200-rung ladder", s.probes);
    }

    #[test]
    fn reports_infeasible_rather_than_overshooting() {
        let f = Fake { sizes: vec![900_000, 1_000_000, 2_000_000] };
        let out = solve(&f, SizeTarget::AtMost(100_000), Bias::Balanced).unwrap();
        assert!(matches!(out, Outcome::Infeasible { .. }));
        assert!(!out.is_success());
    }

    #[test]
    fn pads_when_even_the_best_setting_is_too_small() {
        // A real JPEG is needed for padding to work, so use the bytes of one.
        let jpeg = crate::testkit::tiny_jpeg();
        struct OneShot(Vec<u8>);
        impl Encodable for OneShot {
            type Knob = ();
            fn ladder(&self, _b: Bias) -> Vec<()> {
                vec![()]
            }
            fn encode(&self, _k: &()) -> Result<Vec<u8>> {
                Ok(self.0.clone())
            }
            fn describe(&self, _k: &()) -> String {
                "as-is".into()
            }
        }
        let target = SizeTarget::Range { min: 50_000, max: 100_000 };
        let out = solve(&OneShot(jpeg), target, Bias::Balanced).unwrap();
        assert!(matches!(out, Outcome::Padded(_)), "expected padding, got {out:?}");
        assert!(target.contains(out.size()), "padded to {} bytes", out.size());
    }

    #[test]
    fn never_returns_an_out_of_range_success() {
        // The invariant the app rests on, over a spread of ladders and targets.
        for step in [1_u64, 7, 33, 250] {
            let f = Fake { sizes: (1..=120).map(|i| i * step * 100).collect() };
            for max in [5_000_u64, 50_000, 300_000, 5_000_000] {
                let target = SizeTarget::AtMost(max);
                let out = solve(&f, target, Bias::Balanced).unwrap();
                if out.is_success() {
                    assert!(
                        out.size() <= max,
                        "step {step} max {max}: returned {} bytes",
                        out.size()
                    );
                }
            }
        }
    }
}
