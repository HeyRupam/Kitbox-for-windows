//! Where a mark sits, shared by the PDF and image watermark tools.

use serde::{Deserialize, Serialize};

/// Where the mark sits on the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    TopLeft,
    TopCentre,
    TopRight,
    MiddleLeft,
    #[default]
    Centre,
    MiddleRight,
    BottomLeft,
    BottomCentre,
    BottomRight,
    /// Repeated in a grid across the whole page.
    Tiled,
}

impl Placement {
    /// The nine fixed spots, in reading order. `Tiled` is deliberately absent:
    /// it is a repetition rather than a position.
    pub const GRID: [Placement; 9] = [
        Placement::TopLeft,
        Placement::TopCentre,
        Placement::TopRight,
        Placement::MiddleLeft,
        Placement::Centre,
        Placement::MiddleRight,
        Placement::BottomLeft,
        Placement::BottomCentre,
        Placement::BottomRight,
    ];

    /// Centre point for this placement inside `(width, height)`, given a mark
    /// of `(mark_w, mark_h)` and a margin, with y growing **upwards**.
    pub fn anchor(
        self,
        width: f32,
        height: f32,
        mark_w: f32,
        mark_h: f32,
        margin: f32,
    ) -> (f32, f32) {
        let left = margin + mark_w / 2.0;
        let right = width - margin - mark_w / 2.0;
        let bottom = margin + mark_h / 2.0;
        let top = height - margin - mark_h / 2.0;
        let (mid_x, mid_y) = (width / 2.0, height / 2.0);

        match self {
            Placement::TopLeft => (left, top),
            Placement::TopCentre => (mid_x, top),
            Placement::TopRight => (right, top),
            Placement::MiddleLeft => (left, mid_y),
            Placement::MiddleRight => (right, mid_y),
            Placement::BottomLeft => (left, bottom),
            Placement::BottomCentre => (mid_x, bottom),
            Placement::BottomRight => (right, bottom),
            Placement::Centre | Placement::Tiled => (mid_x, mid_y),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placements_land_where_their_names_say() {
        let (w, h) = (612.0f32, 792.0f32);
        let top = Placement::TopLeft.anchor(w, h, 100.0, 20.0, 36.0);
        let bottom = Placement::BottomRight.anchor(w, h, 100.0, 20.0, 36.0);
        assert!(top.1 > bottom.1, "the top should be higher up");
        assert!(bottom.0 > top.0, "the right should be further across");
        assert_eq!(Placement::Centre.anchor(w, h, 100.0, 20.0, 36.0), (306.0, 396.0));
    }

    #[test]
    fn the_grid_holds_every_fixed_spot() {
        assert_eq!(Placement::GRID.len(), 9);
        assert!(!Placement::GRID.contains(&Placement::Tiled));
    }
}
