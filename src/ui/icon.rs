//! The window/taskbar icon, drawn in code so no binary asset is needed:
//! a rounded blue tile with a white activity trace.

const SIZE: usize = 64;
const BACKGROUND: [u8; 3] = [0x1f, 0x6f, 0xeb];
/// Activity trace in tile coordinates (0..64).
const TRACE: [(f32, f32); 7] = [
    (10.0, 38.0),
    (22.0, 38.0),
    (28.0, 22.0),
    (36.0, 46.0),
    (42.0, 30.0),
    (46.0, 38.0),
    (54.0, 38.0),
];
const CORNER: f32 = 12.0;
const STROKE: f32 = 3.2;

fn distance_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    let (x, y) = (a.0 + t * dx - p.0, a.1 + t * dy - p.1);
    (x * x + y * y).sqrt()
}

/// Signed coverage (0..1) of the rounded tile at pixel centre `p`.
fn tile_coverage(p: (f32, f32)) -> f32 {
    let half = SIZE as f32 / 2.0;
    let (qx, qy) = (
        (p.0 - half).abs() - (half - CORNER),
        (p.1 - half).abs() - (half - CORNER),
    );
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - CORNER;
    (0.5 - outside).clamp(0.0, 1.0)
}

pub fn rgba() -> Vec<u8> {
    let mut out = Vec::with_capacity(SIZE * SIZE * 4);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let p = (x as f32 + 0.5, y as f32 + 0.5);
            let d = TRACE
                .windows(2)
                .map(|w| distance_to_segment(p, w[0], w[1]))
                .fold(f32::MAX, f32::min);
            let line = (STROKE / 2.0 + 0.5 - d).clamp(0.0, 1.0);
            let mix = |c: u8| (f32::from(c) * (1.0 - line) + 255.0 * line).round() as u8;
            let alpha = (tile_coverage(p) * 255.0).round() as u8;
            out.extend_from_slice(&[mix(BACKGROUND[0]), mix(BACKGROUND[1]), mix(BACKGROUND[2]), alpha]);
        }
    }
    out
}

pub fn icon_data() -> egui::IconData {
    egui::IconData {
        rgba: rgba(),
        width: SIZE as u32,
        height: SIZE as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_transparent_corners_and_white_trace() {
        let px = rgba();
        assert_eq!(px.len(), SIZE * SIZE * 4);
        let at = |x: usize, y: usize| &px[(y * SIZE + x) * 4..(y * SIZE + x) * 4 + 4];
        assert_eq!(at(0, 0)[3], 0, "corner must be transparent");
        assert_eq!(at(32, 4)[3], 255, "edge centre is opaque");
        assert_eq!(at(15, 37), &[255, 255, 255, 255], "trace is white");
    }
}
