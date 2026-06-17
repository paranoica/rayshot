#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Pen,
    Line,
    Arrow,
    Rect,
    Marker,
    Text,
    Pixelate,
    Blur,
}

impl Tool {
    pub fn icon(self) -> &'static str {
        use egui_phosphor::regular as p;
        match self {
            Tool::Select => p::CURSOR,
            Tool::Pen => p::PENCIL_SIMPLE,
            Tool::Line => p::LINE_SEGMENT,
            Tool::Arrow => p::ARROW_UP_RIGHT,
            Tool::Rect => p::RECTANGLE,
            Tool::Marker => p::HIGHLIGHTER,
            Tool::Text => p::TEXT_T,
            Tool::Pixelate => p::SQUARES_FOUR,
            Tool::Blur => p::DROP,
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            Tool::Select => "Select / move / resize (S)",
            Tool::Pen => "Pen (P)",
            Tool::Line => "Line (L)",
            Tool::Arrow => "Arrow (A)",
            Tool::Rect => "Rectangle (R)",
            Tool::Marker => "Marker (M)",
            Tool::Text => "Text (T)",
            Tool::Pixelate => "Pixelate (X)",
            Tool::Blur => "Blur (B)",
        }
    }
}

#[derive(Clone)]
pub enum Shape {
    Rect {
        rect: egui::Rect,
        color: egui::Color32,
        width: f32,
    },
    Arrow {
        from: egui::Pos2,
        to: egui::Pos2,
        color: egui::Color32,
        width: f32,
    },
    Line {
        from: egui::Pos2,
        to: egui::Pos2,
        color: egui::Color32,
        width: f32,
    },
    Pen {
        points: Vec<egui::Pos2>,
        color: egui::Color32,
        width: f32,
    },
    Text {
        pos: egui::Pos2,
        text: String,
        color: egui::Color32,
        size: f32,
    },
    Blur {
        cell: f32,
        cells: Vec<(u32, u32, egui::Color32)>,
    },
}

pub const PIXEL_CELL: f32 = 16.0;
pub const PIXEL_BRUSH: f32 = 16.0;
pub const PIXEL_SAMPLE: f32 = 8.0;
pub const BLUR_CELL: f32 = 3.0;
pub const BLUR_BRUSH: f32 = 16.0;
pub const BLUR_SAMPLE: f32 = 2.0;
pub const BLUR_RADIUS: usize = 8;
pub const BLUR_PASSES: usize = 3;

pub fn blur_frame(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let w = w as usize;
    let h = h as usize;
    let mut src = rgba.to_vec();
    let mut tmp = vec![0u8; src.len()];
    for _ in 0..BLUR_PASSES {
        box_blur_h(&src, &mut tmp, w, h, BLUR_RADIUS);
        box_blur_v(&tmp, &mut src, w, h, BLUR_RADIUS);
    }
    src
}

fn box_blur_h(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize) {
    for y in 0..h {
        let base = y * w * 4;
        for c in 0..4 {
            let mut sum: u32 = 0;
            for x in 0..=r.min(w - 1) {
                sum += src[base + x * 4 + c] as u32;
            }
            let mut lo: isize = -(r as isize);
            let mut hi: isize = r as isize;
            for x in 0..w {
                let count = (hi.min(w as isize - 1) - lo.max(0) + 1) as u32;
                dst[base + x * 4 + c] = (sum / count) as u8;
                let add = hi + 1;
                if add < w as isize {
                    sum += src[base + add as usize * 4 + c] as u32;
                }
                if lo >= 0 {
                    sum -= src[base + lo as usize * 4 + c] as u32;
                }
                lo += 1;
                hi += 1;
            }
        }
    }
}

fn box_blur_v(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize) {
    let stride = w * 4;
    for x in 0..w {
        let col = x * 4;
        for c in 0..4 {
            let mut sum: u32 = 0;
            for y in 0..=r.min(h - 1) {
                sum += src[col + y * stride + c] as u32;
            }
            let mut lo: isize = -(r as isize);
            let mut hi: isize = r as isize;
            for y in 0..h {
                let count = (hi.min(h as isize - 1) - lo.max(0) + 1) as u32;
                dst[col + y * stride + c] = (sum / count) as u8;
                let add = hi + 1;
                if add < h as isize {
                    sum += src[col + add as usize * stride + c] as u32;
                }
                if lo >= 0 {
                    sum -= src[col + lo as usize * stride + c] as u32;
                }
                lo += 1;
                hi += 1;
            }
        }
    }
}

fn box_color(rgba: &[u8], fw: u32, fh: u32, cx: f32, cy: f32, half: f32) -> egui::Color32 {
    let x0 = (cx - half).max(0.0) as u32;
    let y0 = (cy - half).max(0.0) as u32;
    let x1 = ((cx + half) as u32).min(fw);
    let y1 = ((cy + half) as u32).min(fh);
    if x1 <= x0 || y1 <= y0 {
        return egui::Color32::BLACK;
    }
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for py in y0..y1 {
        let base = (py * fw + x0) as usize * 4;
        for px in 0..(x1 - x0) as usize {
            let i = base + px * 4;
            r += rgba[i] as u64;
            g += rgba[i + 1] as u64;
            b += rgba[i + 2] as u64;
            n += 1;
        }
    }
    let n = n.max(1);
    egui::Color32::from_rgb((r / n) as u8, (g / n) as u8, (b / n) as u8)
}

pub fn add_brush_cells(
    cells: &mut Vec<(u32, u32, egui::Color32)>,
    rgba: &[u8],
    fw: u32,
    fh: u32,
    fp: egui::Pos2,
    cell: f32,
    brush: f32,
    sample: f32,
) {
    let max_gx = (fw as f32 / cell).ceil() as u32;
    let max_gy = (fh as f32 / cell).ceil() as u32;
    let gx0 = ((fp.x - brush).max(0.0) / cell) as u32;
    let gy0 = ((fp.y - brush).max(0.0) / cell) as u32;
    let gx1 = (((fp.x + brush) / cell) as u32).min(max_gx.saturating_sub(1));
    let gy1 = (((fp.y + brush) / cell) as u32).min(max_gy.saturating_sub(1));
    let mut seen: std::collections::HashSet<(u32, u32)> =
        cells.iter().map(|&(x, y, _)| (x, y)).collect();
    for gy in gy0..=gy1 {
        for gx in gx0..=gx1 {
            let cxc = (gx as f32 + 0.5) * cell;
            let cyc = (gy as f32 + 0.5) * cell;
            if (egui::pos2(cxc, cyc) - fp).length() > brush + cell * 0.5 {
                continue;
            }
            if seen.insert((gx, gy)) {
                cells.push((gx, gy, box_color(rgba, fw, fh, cxc, cyc, sample)));
            }
        }
    }
}

#[derive(Default)]
pub struct Scene {
    shapes: Vec<Shape>,
    undo: Vec<Vec<Shape>>,
    redo: Vec<Vec<Shape>>,
}

const MAX_HISTORY: usize = 200;

impl Scene {
    pub fn begin_change(&mut self) {
        self.undo.push(self.shapes.clone());
        if self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn push(&mut self, shape: Shape) {
        self.begin_change();
        self.shapes.push(shape);
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.shapes, prev));
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.shapes, next));
        }
    }

    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }

    pub fn set_shape(&mut self, idx: usize, shape: Shape) {
        if let Some(s) = self.shapes.get_mut(idx) {
            *s = shape;
        }
    }
}

fn dist_to_segment(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 <= f32::EPSILON {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

fn text_box(pos: egui::Pos2, text: &str, size: f32) -> egui::Rect {
    let w = (text.chars().count().max(1) as f32) * size * 0.55;
    egui::Rect::from_min_size(pos, egui::vec2(w, size * 1.3))
}

pub fn shape_hit(shapes: &[Shape], p: egui::Pos2) -> Option<usize> {
    for (i, s) in shapes.iter().enumerate().rev() {
        let hit = match s {
            Shape::Rect { rect, width, .. } => {
                let tol = width.max(6.0) + 4.0;
                rect.expand(tol).contains(p) && !rect.shrink(tol).contains(p)
            }
            Shape::Line {
                from, to, width, ..
            }
            | Shape::Arrow {
                from, to, width, ..
            } => dist_to_segment(p, *from, *to) <= width.max(6.0) + 4.0,
            Shape::Pen { points, width, .. } => points
                .windows(2)
                .any(|w| dist_to_segment(p, w[0], w[1]) <= width.max(6.0) + 4.0),
            Shape::Text {
                pos, text, size, ..
            } => text_box(*pos, text, *size).contains(p),
            Shape::Blur { .. } => false,
        };
        if hit {
            return Some(i);
        }
    }
    None
}

pub fn translated(shape: &Shape, d: egui::Vec2) -> Shape {
    match shape {
        Shape::Rect { rect, color, width } => Shape::Rect {
            rect: rect.translate(d),
            color: *color,
            width: *width,
        },
        Shape::Line {
            from,
            to,
            color,
            width,
        } => Shape::Line {
            from: *from + d,
            to: *to + d,
            color: *color,
            width: *width,
        },
        Shape::Arrow {
            from,
            to,
            color,
            width,
        } => Shape::Arrow {
            from: *from + d,
            to: *to + d,
            color: *color,
            width: *width,
        },
        Shape::Pen {
            points,
            color,
            width,
        } => Shape::Pen {
            points: points.iter().map(|p| *p + d).collect(),
            color: *color,
            width: *width,
        },
        Shape::Text {
            pos,
            text,
            color,
            size,
        } => Shape::Text {
            pos: *pos + d,
            text: text.clone(),
            color: *color,
            size: *size,
        },
        Shape::Blur { cell, cells } => Shape::Blur {
            cell: *cell,
            cells: cells.clone(),
        },
    }
}

pub fn paint(
    painter: &egui::Painter,
    shape: &Shape,
    to_screen: &impl Fn(egui::Pos2) -> egui::Pos2,
    scale: f32,
) {
    match shape {
        Shape::Rect { rect, color, width } => {
            let r = egui::Rect::from_two_pos(to_screen(rect.min), to_screen(rect.max));
            painter.rect_stroke(
                r,
                0,
                egui::Stroke::new(width * scale, *color),
                egui::StrokeKind::Inside,
            );
        }
        Shape::Line {
            from,
            to,
            color,
            width,
        } => {
            painter.line_segment(
                [to_screen(*from), to_screen(*to)],
                egui::Stroke::new(width * scale, *color),
            );
        }
        Shape::Arrow {
            from,
            to,
            color,
            width,
        } => {
            let s = to_screen(*from);
            let e = to_screen(*to);
            let stroke = egui::Stroke::new(width * scale, *color);
            let dir = e - s;
            let len = dir.length();
            if len > 1.0 {
                let d = dir / len;
                let head = (width * scale * 4.5).max(12.0);
                let normal = egui::vec2(-d.y, d.x);
                let base = e - d * head;
                painter.line_segment([s, base], stroke);
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        e,
                        base + normal * (head * 0.5),
                        base - normal * (head * 0.5),
                    ],
                    *color,
                    egui::Stroke::NONE,
                ));
            } else {
                painter.line_segment([s, e], stroke);
            }
        }
        Shape::Pen {
            points,
            color,
            width,
        } => {
            let pts: Vec<egui::Pos2> = points.iter().map(|p| to_screen(*p)).collect();
            if pts.len() >= 2 {
                painter.add(egui::Shape::line(
                    pts,
                    egui::Stroke::new(width * scale, *color),
                ));
            }
        }
        Shape::Text {
            pos,
            text,
            color,
            size,
        } => {
            painter.text(
                to_screen(*pos),
                egui::Align2::LEFT_TOP,
                text,
                egui::FontId::proportional(size * scale),
                *color,
            );
        }
        Shape::Blur { cell, cells } => {
            for &(gx, gy, color) in cells {
                let min = egui::pos2(gx as f32 * cell, gy as f32 * cell);
                let r = egui::Rect::from_min_max(
                    to_screen(min),
                    to_screen(egui::pos2(min.x + cell, min.y + cell)),
                );
                painter.rect_filled(r, 0, color);
            }
        }
    }
}
