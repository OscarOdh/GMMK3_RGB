//! A procedurally drawn tray/window icon, so the build stays a single crate
//! with no asset files to ship alongside the exe.

pub const SIZE: u32 = 32;

const BODY: [u8; 4] = [0x16, 0x18, 0x1C, 0xFF];
const EDGE: [u8; 4] = [0x00, 0xE0, 0x50, 0xFF];
const KEY: [u8; 4] = [0x00, 0x9A, 0x40, 0xFF];

/// 32×32 RGBA, straight (unmultiplied) alpha.
pub fn rgba() -> Vec<u8> {
    let s = SIZE as i32;
    let mut px = vec![0u8; (s * s * 4) as usize];

    let set = |px: &mut Vec<u8>, x: i32, y: i32, c: [u8; 4]| {
        if x < 0 || y < 0 || x >= s || y >= s {
            return;
        }
        let i = ((y * s + x) * 4) as usize;
        px[i..i + 4].copy_from_slice(&c);
    };

    // Rounded body with a bright edge.
    for y in 0..s {
        for x in 0..s {
            if !in_rounded(x, y, 0, 0, s, s, 7) {
                continue;
            }
            let inner = in_rounded(x, y, 2, 2, s - 4, s - 4, 5);
            set(&mut px, x, y, if inner { BODY } else { EDGE });
        }
    }

    // Three rows of keycaps plus a space bar — reads as a keyboard at 16px.
    for row in 0..3 {
        for col in 0..4 {
            let x0 = 6 + col * 5;
            let y0 = 8 + row * 5;
            for y in y0..y0 + 3 {
                for x in x0..x0 + 4 {
                    set(&mut px, x, y, KEY);
                }
            }
        }
    }
    for y in 23..26 {
        for x in 8..24 {
            set(&mut px, x, y, KEY);
        }
    }

    px
}

fn in_rounded(x: i32, y: i32, rx: i32, ry: i32, w: i32, h: i32, r: i32) -> bool {
    if x < rx || y < ry || x >= rx + w || y >= ry + h {
        return false;
    }
    let (lx, ly) = (x - rx, y - ry);
    // Distance from the nearest corner centre, only inside the corner boxes.
    let cx = if lx < r {
        r
    } else if lx >= w - r {
        w - r - 1
    } else {
        return true;
    };
    let cy = if ly < r {
        r
    } else if ly >= h - r {
        h - r - 1
    } else {
        return true;
    };
    let (dx, dy) = ((lx - cx) as f32, (ly - cy) as f32);
    dx * dx + dy * dy <= (r as f32 + 0.5) * (r as f32 + 0.5)
}
