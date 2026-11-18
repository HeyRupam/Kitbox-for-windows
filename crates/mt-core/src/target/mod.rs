//! The size-targeting solver.
//!
//! Every tool that produces a file can be asked for a size instead of a quality
//! setting. The caller describes a ladder of encoder settings ordered from
//! smallest output to largest; the solver finds the setting that produces the
//! largest file still inside the requested window, by measuring real encodes
//! rather than predicting them.
//!
//! The invariant the whole app rests on: a returned [`Outcome::Hit`] or
//! [`Outcome::Padded`] is always inside the requested range. When no setting can
//! get there, the solver says so and suggests what to change — it never returns
//! an out-of-range result dressed up as success.

pub mod pad;
mod solver;

pub use solver::{solve, solve_with};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::error::Result;

/// What the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SizeTarget {
    /// "under 100 KB"
    AtMost(u64),
    /// "between 50 and 100 KB" — the shape most forms use.
    Range { min: u64, max: u64 },
    /// "about 1 MB", within a tolerance expressed as a fraction (0.1 = ±10%).
    Approx { bytes: u64, tol: f32 },
}

impl SizeTarget {
    /// The largest acceptable output.
    pub fn max_bytes(&self) -> u64 {
        match *self {
            SizeTarget::AtMost(n) => n,
            SizeTarget::Range { max, .. } => max,
            SizeTarget::Approx { bytes, tol } => {
                bytes.saturating_add((bytes as f64 * tol as f64) as u64)
            }
        }
    }

    /// The smallest acceptable output. Zero when the user only set a ceiling.
    pub fn min_bytes(&self) -> u64 {
        match *self {
            SizeTarget::AtMost(_) => 0,
            SizeTarget::Range { min, .. } => min,
            SizeTarget::Approx { bytes, tol } => {
                bytes.saturating_sub((bytes as f64 * tol as f64) as u64)
            }
        }
    }

    pub fn contains(&self, size: u64) -> bool {
        size >= self.min_bytes() && size <= self.max_bytes()
    }

    pub fn describe(&self) -> String {
        use crate::size::format_size;
        match *self {
            SizeTarget::AtMost(n) => format!("under {}", format_size(n)),
            SizeTarget::Range { min, max } => {
                format!("{} to {}", format_size(min), format_size(max))
            }
            SizeTarget::Approx { bytes, .. } => format!("about {}", format_size(bytes)),
        }
    }
}

/// Which lever the solver spends first when it has to give something up.
///
/// Surfaced in the UI as "Keep sharp" / "Shrink pixels".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bias {
    /// Hold full resolution as long as possible, drop encoder quality first.
    /// Right for scans and documents, where pixel count carries the legibility.
    KeepSharp,
    /// Hold encoder quality, reduce dimensions first. Right for photos, where
    /// a smaller sharp image beats a full-size blocky one.
    ShrinkPixels,
    #[default]
    Balanced,
}

/// Something that can be encoded repeatedly at different settings.
///
/// Implementors expose a *ladder*: settings ordered from smallest expected
/// output to largest. The ordering only needs to be approximately monotone —
/// the solver re-checks its neighbourhood before committing.
pub trait Encodable: Sync {
    type Knob: Clone + Send + Sync + std::fmt::Debug;

    /// Settings ordered smallest output first, largest last.
    fn ladder(&self, bias: Bias) -> Vec<Self::Knob>;

    /// Encode at one setting and return the real bytes. The solver only ever
    /// decides based on the length of what this returns.
    fn encode(&self, knob: &Self::Knob) -> Result<Vec<u8>>;

    /// Human description of a setting, shown in the result card.
    fn describe(&self, knob: &Self::Knob) -> String;

    /// Optional similarity score of an encoded result against the source,
    /// 0.0–1.0. Used for the "98.1% visually identical" readout.
    fn score(&self, _encoded: &[u8]) -> Option<f32> {
        None
    }

    /// What to suggest when nothing on the ladder is small enough.
    fn suggestions(&self, _target: SizeTarget) -> Vec<String> {
        Vec::new()
    }

    /// The file exactly as it arrived, when handing it straight back would be
    /// a complete answer.
    ///
    /// Re-encoding something that already meets the target is the one way a
    /// compressor can insult you: a 37 KB photograph asked to come in under
    /// 100 KB has nothing to do, and the best setting on the ladder may well
    /// be *larger* than the file already is. An encoder returns `None` here
    /// when the request means more than a size — a different format, fixed
    /// dimensions — because then the original is not an answer.
    fn untouched(&self) -> Option<&[u8]> {
        None
    }
}

/// A successful solve.
#[derive(Debug, Clone)]
pub struct Solved {
    pub bytes: Vec<u8>,
    pub size: u64,
    /// Human description of the settings that won, e.g. "quality 68, full size".
    pub knob: String,
    /// Similarity to the source, 0.0–1.0, when the engine can measure it.
    pub score: Option<f32>,
    /// How many real encodes it took.
    pub probes: u8,
    pub elapsed_ms: u64,
    /// Set when something about the result deserves a word in the UI.
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Outcome {
    /// Landed inside the requested window.
    Hit(Solved),
    /// Landed inside the window only after lossless padding, because even the
    /// best setting came out under the required minimum.
    Padded(Solved),
    /// Nothing on the ladder fits. `best` is the smallest result achieved.
    Infeasible {
        best: Vec<u8>,
        best_size: u64,
        why: String,
        suggestions: Vec<String>,
        probes: u8,
        elapsed_ms: u64,
    },
}

impl Outcome {
    pub fn solved(&self) -> Option<&Solved> {
        match self {
            Outcome::Hit(s) | Outcome::Padded(s) => Some(s),
            Outcome::Infeasible { .. } => None,
        }
    }

    pub fn bytes(&self) -> &[u8] {
        match self {
            Outcome::Hit(s) | Outcome::Padded(s) => &s.bytes,
            Outcome::Infeasible { best, .. } => best,
        }
    }

    pub fn size(&self) -> u64 {
        match self {
            Outcome::Hit(s) | Outcome::Padded(s) => s.size,
            Outcome::Infeasible { best_size, .. } => *best_size,
        }
    }

    pub fn is_success(&self) -> bool {
        !matches!(self, Outcome::Infeasible { .. })
    }
}

/// Caches encodes by ladder index so a re-probe costs nothing. The solver
/// revisits indices often enough that this meaningfully cuts the work.
pub(crate) struct Prober<'a, E: Encodable> {
    enc: &'a E,
    ladder: &'a [E::Knob],
    cache: BTreeMap<usize, Vec<u8>>,
    pub probes: u8,
}

impl<'a, E: Encodable> Prober<'a, E> {
    pub fn new(enc: &'a E, ladder: &'a [E::Knob]) -> Self {
        Prober { enc, ladder, cache: BTreeMap::new(), probes: 0 }
    }

    pub fn size(&mut self, i: usize) -> Result<u64> {
        if !self.cache.contains_key(&i) {
            let bytes = self.enc.encode(&self.ladder[i])?;
            self.probes = self.probes.saturating_add(1);
            self.cache.insert(i, bytes);
        }
        Ok(self.cache[&i].len() as u64)
    }

    pub fn take(&mut self, i: usize) -> Vec<u8> {
        self.cache.remove(&i).unwrap_or_default()
    }

    /// The smallest result seen so far, for the infeasible report.
    pub fn smallest(&self) -> Option<(usize, usize)> {
        self.cache.iter().map(|(i, b)| (*i, b.len())).min_by_key(|(_, len)| *len)
    }
}
