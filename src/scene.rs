#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    Ellipse,
    Counter,
    Picker,
}

impl Tool {
    pub const ALL: [Tool; 12] = [
        Tool::Pen,
        Tool::Line,
        Tool::Arrow,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Counter,
        Tool::Marker,
        Tool::Text,
        Tool::Pixelate,
        Tool::Blur,
        Tool::Picker,
        Tool::Select,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "select",
            Tool::Pen => "pen",
            Tool::Line => "line",
            Tool::Arrow => "arrow",
            Tool::Rect => "rectangle",
            Tool::Marker => "marker",
            Tool::Text => "text",
            Tool::Pixelate => "pixelate",
            Tool::Blur => "blur",
            Tool::Ellipse => "ellipse",
            Tool::Counter => "marker-number",
            Tool::Picker => "picker",
        }
    }

    pub fn from_name(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.name() == name)
    }

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
            Tool::Ellipse => p::CIRCLE,
            Tool::Counter => p::NUMBER_CIRCLE_ONE,
            Tool::Picker => p::EYEDROPPER,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Pen => "Pen",
            Tool::Line => "Line",
            Tool::Arrow => "Arrow",
            Tool::Rect => "Rectangle",
            Tool::Marker => "Marker",
            Tool::Text => "Text",
            Tool::Pixelate => "Pixelate",
            Tool::Blur => "Blur",
            Tool::Ellipse => "Ellipse",
            Tool::Counter => "Numbered marker",
            Tool::Picker => "Colour picker",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Tool::Select => "S",
            Tool::Pen => "P",
            Tool::Line => "L",
            Tool::Arrow => "A",
            Tool::Rect => "R",
            Tool::Marker => "M",
            Tool::Text => "T",
            Tool::Pixelate => "X",
            Tool::Blur => "B",
            Tool::Ellipse => "E",
            Tool::Counter => "N",
            Tool::Picker => "I",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
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
    Pixelate {
        cell: f32,
        cells: Vec<(u32, u32, egui::Color32)>,
    },
    Blur {
        points: Vec<egui::Pos2>,
        radius: f32,
    },
    Ellipse {
        rect: egui::Rect,
        color: egui::Color32,
        width: f32,
    },
    Counter {
        center: egui::Pos2,
        number: u32,
        color: egui::Color32,
        radius: f32,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Group {
    Colour,
    Width,
    Nudge,
}

#[derive(Default)]
pub struct Scene {
    shapes: Vec<Shape>,
    undo: Vec<Vec<Shape>>,
    redo: Vec<Vec<Shape>>,
    group: Option<(Group, usize)>,
}

const MAX_HISTORY: usize = 200;

impl Scene {
    pub fn begin_change(&mut self) {
        self.undo.push(self.shapes.clone());
        if self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.group = None;
    }

    pub fn push(&mut self, shape: Shape) {
        self.begin_change();
        self.shapes.push(shape);
    }

    pub fn undo(&mut self) {
        self.group = None;
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.shapes, prev));
        }
    }

    pub fn redo(&mut self) {
        self.group = None;
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

    pub fn edit(&mut self, idx: usize, group: Group, shape: Shape) {
        if idx >= self.shapes.len() {
            return;
        }
        if self.group != Some((group, idx)) {
            self.begin_change();
            self.group = Some((group, idx));
        }
        self.shapes[idx] = shape;
    }

    pub fn break_group(&mut self) {
        self.group = None;
    }

    pub fn remove(&mut self, idx: usize) -> bool {
        if idx >= self.shapes.len() {
            return false;
        }
        self.begin_change();
        self.shapes.remove(idx);
        true
    }

    pub fn duplicate(&mut self, idx: usize, offset: egui::Vec2) -> Option<usize> {
        let mut copy = translated(self.shapes.get(idx)?, offset);
        if let Shape::Counter { number, .. } = &mut copy {
            *number = next_counter(&self.shapes);
        }
        self.push(copy);
        Some(self.shapes.len() - 1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grip {
    Box(usize),
    From,
    To,
}

pub fn bounds(shape: &Shape) -> Option<egui::Rect> {
    match shape {
        Shape::Rect { rect, .. } | Shape::Ellipse { rect, .. } => Some(*rect),
        Shape::Line { from, to, .. } | Shape::Arrow { from, to, .. } => {
            Some(egui::Rect::from_two_pos(*from, *to))
        }
        Shape::Pen { points, .. } => {
            let first = *points.first()?;
            Some(
                points
                    .iter()
                    .fold(egui::Rect::from_min_max(first, first), |r, p| {
                        r.union(egui::Rect::from_min_max(*p, *p))
                    }),
            )
        }
        Shape::Text {
            pos, text, size, ..
        } => Some(text_box(*pos, text, *size)),
        Shape::Counter { center, radius, .. } => Some(egui::Rect::from_center_size(
            *center,
            egui::Vec2::splat(radius * 2.0),
        )),
        Shape::Pixelate { .. } | Shape::Blur { .. } => None,
    }
}

pub fn grips(shape: &Shape) -> Vec<(Grip, egui::Pos2)> {
    match shape {
        Shape::Rect { rect, .. } | Shape::Ellipse { rect, .. } => [
            rect.left_top(),
            rect.center_top(),
            rect.right_top(),
            rect.right_center(),
            rect.right_bottom(),
            rect.center_bottom(),
            rect.left_bottom(),
            rect.left_center(),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, p)| (Grip::Box(i), p))
        .collect(),
        Shape::Line { from, to, .. } | Shape::Arrow { from, to, .. } => {
            vec![(Grip::From, *from), (Grip::To, *to)]
        }
        _ => Vec::new(),
    }
}

pub fn recolored(shape: &Shape, new: egui::Color32) -> Shape {
    let mut out = shape.clone();
    match &mut out {
        Shape::Rect { color, .. }
        | Shape::Ellipse { color, .. }
        | Shape::Line { color, .. }
        | Shape::Arrow { color, .. }
        | Shape::Text { color, .. }
        | Shape::Counter { color, .. } => *color = new,
        Shape::Pen { color, .. } => {
            let alpha = color.a();
            *color = egui::Color32::from_rgba_unmultiplied(new.r(), new.g(), new.b(), alpha);
        }
        Shape::Pixelate { .. } | Shape::Blur { .. } => {}
    }
    out
}

pub fn colour_of(shape: &Shape) -> Option<egui::Color32> {
    match shape {
        Shape::Rect { color, .. }
        | Shape::Ellipse { color, .. }
        | Shape::Line { color, .. }
        | Shape::Arrow { color, .. }
        | Shape::Text { color, .. }
        | Shape::Counter { color, .. }
        | Shape::Pen { color, .. } => {
            let [r, g, b, _] = color.to_srgba_unmultiplied();
            Some(egui::Color32::from_rgb(r, g, b))
        }
        Shape::Pixelate { .. } | Shape::Blur { .. } => None,
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
    let widest = text
        .split('\n')
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);
    let lines = text.split('\n').count().max(1);
    let w = (widest.max(1) as f32) * size * 0.55;
    let line = (size * 1.15).round();
    egui::Rect::from_min_size(pos, egui::vec2(w, line * lines as f32 + size * 0.15))
}

pub fn next_counter(shapes: &[Shape]) -> u32 {
    shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Counter { number, .. } => Some(*number),
            _ => None,
        })
        .max()
        .unwrap_or(0)
        + 1
}

pub fn text_on(fill: egui::Color32) -> egui::Color32 {
    let luma = 0.299 * fill.r() as f32 + 0.587 * fill.g() as f32 + 0.114 * fill.b() as f32;
    if luma > 150.0 {
        egui::Color32::BLACK
    } else {
        egui::Color32::WHITE
    }
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
            Shape::Pixelate { .. } => false,
            Shape::Blur { .. } => false,
            Shape::Ellipse { rect, width, .. } => {
                let (c, r) = (rect.center(), rect.size() / 2.0);
                if r.x <= f32::EPSILON || r.y <= f32::EPSILON {
                    false
                } else {
                    let q = p - c;
                    let k = ((q.x / r.x).powi(2) + (q.y / r.y).powi(2)).sqrt();
                    let on_curve = if k > 0.0 { q / k } else { egui::vec2(r.x, 0.0) };
                    (q - on_curve).length() <= width.max(6.0) + 4.0
                }
            }
            Shape::Counter { center, radius, .. } => (p - *center).length() <= radius + 4.0,
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
        Shape::Pixelate { cell, cells } => Shape::Pixelate {
            cell: *cell,
            cells: cells.clone(),
        },
        Shape::Blur { points, radius } => Shape::Blur {
            points: points.iter().map(|p| *p + d).collect(),
            radius: *radius,
        },
        Shape::Ellipse { rect, color, width } => Shape::Ellipse {
            rect: rect.translate(d),
            color: *color,
            width: *width,
        },
        Shape::Counter {
            center,
            number,
            color,
            radius,
        } => Shape::Counter {
            center: *center + d,
            number: *number,
            color: *color,
            radius: *radius,
        },
    }
}

pub fn paint(
    painter: &egui::Painter,
    shape: &Shape,
    to_screen: &impl Fn(egui::Pos2) -> egui::Pos2,
    scale: f32,
    blur_tex: egui::TextureId,
    frame_w: f32,
    frame_h: f32,
) {
    match shape {
        Shape::Ellipse { rect, color, width } => {
            let r = egui::Rect::from_two_pos(to_screen(rect.min), to_screen(rect.max));
            let (radius, stroke) = (r.size() / 2.0, width * scale);
            if radius.min_elem() < stroke {
                let c = r.center();
                let reach = radius.max_elem() - radius.min_elem();
                let along = if radius.x >= radius.y {
                    egui::vec2(reach, 0.0)
                } else {
                    egui::vec2(0.0, reach)
                };
                let thick = stroke + radius.min_elem() * 2.0;
                painter.line_segment([c - along, c + along], egui::Stroke::new(thick, *color));
                painter.circle_filled(c - along, thick / 2.0, *color);
                painter.circle_filled(c + along, thick / 2.0, *color);
            } else {
                painter.add(egui::Shape::ellipse_stroke(
                    r.center(),
                    radius,
                    egui::Stroke::new(stroke, *color),
                ));
            }
        }
        Shape::Counter {
            center,
            number,
            color,
            radius,
        } => {
            let c = to_screen(*center);
            let r = radius * scale;
            for (grow, alpha) in [(1.22, 18), (1.14, 30), (1.07, 45)] {
                painter.circle_filled(
                    c + egui::vec2(0.0, r * 0.1),
                    r * grow,
                    egui::Color32::from_black_alpha(alpha),
                );
            }
            painter.circle_filled(c, r, *color);
            painter.circle_stroke(
                c,
                r,
                egui::Stroke::new((r * 0.13).max(1.5), egui::Color32::WHITE),
            );
            painter.text(
                c - egui::vec2(0.0, r * 0.05),
                egui::Align2::CENTER_CENTER,
                number.to_string(),
                egui::FontId::proportional(r * 1.1),
                text_on(*color),
            );
        }
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
        Shape::Pixelate { cell, cells } => {
            for &(gx, gy, color) in cells {
                let min = egui::pos2(gx as f32 * cell, gy as f32 * cell);
                let r = egui::Rect::from_min_max(
                    to_screen(min),
                    to_screen(egui::pos2(min.x + cell, min.y + cell)),
                );
                painter.rect_filled(r, 0, color);
            }
        }
        Shape::Blur { points, radius } => {
            if points.is_empty() {
                return;
            }
            let r = *radius;
            let white = egui::Color32::WHITE;
            let uvof = |fp: egui::Pos2| egui::pos2(fp.x / frame_w, fp.y / frame_h);
            let vert = |fp: egui::Pos2| egui::epaint::Vertex {
                pos: to_screen(fp),
                uv: uvof(fp),
                color: white,
            };
            let mut mesh = egui::Mesh::with_texture(blur_tex);
            let segs = 20u32;
            for &c in points {
                let base = mesh.vertices.len() as u32;
                mesh.vertices.push(vert(c));
                for i in 0..segs {
                    let a = std::f32::consts::TAU * i as f32 / segs as f32;
                    mesh.vertices
                        .push(vert(egui::pos2(c.x + r * a.cos(), c.y + r * a.sin())));
                }
                for i in 0..segs {
                    mesh.indices.extend_from_slice(&[
                        base,
                        base + 1 + i,
                        base + 1 + (i + 1) % segs,
                    ]);
                }
            }
            for w in points.windows(2) {
                let (a, b) = (w[0], w[1]);
                let dir = b - a;
                let len = dir.length();
                if len < 0.01 {
                    continue;
                }
                let n = egui::vec2(-dir.y, dir.x) / len * r;
                let base = mesh.vertices.len() as u32;
                for &fp in &[a + n, b + n, b - n, a - n] {
                    mesh.vertices.push(vert(fp));
                }
                mesh.indices.extend_from_slice(&[
                    base,
                    base + 1,
                    base + 2,
                    base,
                    base + 2,
                    base + 3,
                ]);
            }
            painter.add(egui::Shape::mesh(mesh));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Color32, pos2};

    #[test]
    fn the_text_hit_box_fits_the_drawn_lines() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            for size in [16.0f32, 20.0, 24.0, 48.0] {
                for n in [1usize, 2, 5] {
                    let text = vec!["Hg"; n].join("\n");
                    let drawn = ui
                        .painter()
                        .layout_no_wrap(
                            text.clone(),
                            egui::FontId::proportional(size),
                            Color32::WHITE,
                        )
                        .size()
                        .y;
                    let hit = text_box(pos2(0.0, 0.0), &text, size).height();
                    assert!(
                        hit >= drawn && hit <= drawn + size * 0.2,
                        "size {size}, {n} lines: hit {hit}, drawn {drawn}"
                    );
                }
            }
        });
    }

    #[test]
    fn several_lines_of_text_are_hit_on_every_line() {
        let shape = Shape::Text {
            pos: egui::pos2(100.0, 100.0),
            text: "ab\nlonger line".into(),
            color: egui::Color32::RED,
            size: 20.0,
        };
        assert!(shape_hit(std::slice::from_ref(&shape), egui::pos2(105.0, 110.0)).is_some());
        assert!(
            shape_hit(std::slice::from_ref(&shape), egui::pos2(200.0, 135.0)).is_some(),
            "the long second line"
        );
        assert!(
            shape_hit(std::slice::from_ref(&shape), egui::pos2(105.0, 170.0)).is_none(),
            "below the last line"
        );
        assert!(
            shape_hit(std::slice::from_ref(&shape), egui::pos2(240.0, 110.0)).is_none(),
            "past the longest line"
        );
    }

    fn counter(number: u32) -> Shape {
        Shape::Counter {
            center: pos2(0.0, 0.0),
            number,
            color: Color32::RED,
            radius: 12.0,
        }
    }

    fn rect(color: Color32) -> Shape {
        Shape::Rect {
            rect: egui::Rect::from_min_max(pos2(10.0, 20.0), pos2(110.0, 70.0)),
            color,
            width: 3.0,
        }
    }

    fn marker_pen() -> Shape {
        Shape::Pen {
            points: vec![pos2(0.0, 0.0), pos2(40.0, 10.0)],
            color: Color32::from_rgba_unmultiplied(255, 0, 0, 110),
            width: 16.0,
        }
    }

    #[test]
    fn bounds_cover_what_can_be_selected() {
        let r = egui::Rect::from_min_max(pos2(10.0, 20.0), pos2(110.0, 70.0));
        assert_eq!(bounds(&rect(Color32::RED)), Some(r));
        let line = Shape::Line {
            from: pos2(50.0, 5.0),
            to: pos2(10.0, 40.0),
            color: Color32::RED,
            width: 3.0,
        };
        assert_eq!(
            bounds(&line),
            Some(egui::Rect::from_min_max(pos2(10.0, 5.0), pos2(50.0, 40.0)))
        );
        let c = Shape::Counter {
            center: pos2(100.0, 100.0),
            number: 1,
            color: Color32::RED,
            radius: 12.0,
        };
        assert_eq!(
            bounds(&c),
            Some(egui::Rect::from_center_size(
                pos2(100.0, 100.0),
                egui::vec2(24.0, 24.0)
            ))
        );
        assert_eq!(
            bounds(&Shape::Blur {
                points: vec![pos2(1.0, 1.0)],
                radius: 16.0
            }),
            None
        );
        assert_eq!(
            bounds(&Shape::Pixelate {
                cell: 16.0,
                cells: vec![]
            }),
            None
        );
    }

    #[test]
    fn boxes_have_eight_grips_and_lines_two() {
        let g = grips(&rect(Color32::RED));
        assert_eq!(g.len(), 8);
        assert_eq!(g[0], (Grip::Box(0), pos2(10.0, 20.0)));
        assert_eq!(g[4], (Grip::Box(4), pos2(110.0, 70.0)));
        assert_eq!(g[7], (Grip::Box(7), pos2(10.0, 45.0)));
        let arrow = Shape::Arrow {
            from: pos2(1.0, 2.0),
            to: pos2(30.0, 40.0),
            color: Color32::RED,
            width: 3.0,
        };
        assert_eq!(
            grips(&arrow),
            vec![(Grip::From, pos2(1.0, 2.0)), (Grip::To, pos2(30.0, 40.0))]
        );
        assert!(grips(&marker_pen()).is_empty());
        assert!(grips(&counter(1)).is_empty());
    }

    #[test]
    fn recolouring_keeps_geometry_and_marker_transparency() {
        match recolored(&rect(Color32::RED), Color32::BLUE) {
            Shape::Rect { color, width, rect } => {
                assert_eq!(color, Color32::BLUE);
                assert_eq!(width, 3.0);
                assert_eq!(rect.min, pos2(10.0, 20.0));
            }
            _ => panic!("not a rect"),
        }
        match recolored(&marker_pen(), Color32::from_rgb(0, 0, 255)) {
            Shape::Pen { color, width, .. } => {
                assert_eq!(color, Color32::from_rgba_unmultiplied(0, 0, 255, 110));
                assert_eq!(width, 16.0);
            }
            _ => panic!("not a pen"),
        }
        let blur = Shape::Blur {
            points: vec![pos2(1.0, 1.0)],
            radius: 16.0,
        };
        assert_eq!(recolored(&blur, Color32::BLUE), blur);
    }

    #[test]
    fn the_colour_of_a_marker_reads_back_opaque() {
        assert_eq!(colour_of(&marker_pen()), Some(Color32::from_rgb(255, 0, 0)));
        assert_eq!(colour_of(&rect(Color32::GREEN)), Some(Color32::GREEN));
        assert_eq!(
            colour_of(&Shape::Pixelate {
                cell: 16.0,
                cells: vec![]
            }),
            None
        );
    }

    #[test]
    fn a_run_of_the_same_edit_is_one_undo_step() {
        let mut scene = Scene::default();
        scene.push(rect(Color32::RED));
        scene.edit(0, Group::Width, rect(Color32::GREEN));
        scene.edit(0, Group::Width, rect(Color32::BLUE));
        assert_eq!(scene.shapes()[0], rect(Color32::BLUE));
        scene.undo();
        assert_eq!(scene.shapes()[0], rect(Color32::RED));
        scene.undo();
        assert!(scene.shapes().is_empty());
    }

    #[test]
    fn a_different_edit_starts_a_new_undo_step() {
        let mut scene = Scene::default();
        scene.push(rect(Color32::RED));
        scene.push(rect(Color32::WHITE));
        scene.edit(0, Group::Colour, rect(Color32::GREEN));
        scene.edit(0, Group::Width, rect(Color32::BLUE));
        scene.edit(1, Group::Width, rect(Color32::BLACK));
        scene.undo();
        assert_eq!(scene.shapes()[1], rect(Color32::WHITE));
        assert_eq!(scene.shapes()[0], rect(Color32::BLUE));
        scene.undo();
        assert_eq!(scene.shapes()[0], rect(Color32::GREEN));
        scene.undo();
        assert_eq!(scene.shapes()[0], rect(Color32::RED));
    }

    #[test]
    fn undo_or_a_new_shape_ends_a_run_of_edits() {
        let mut scene = Scene::default();
        scene.push(rect(Color32::RED));
        scene.edit(0, Group::Nudge, rect(Color32::GREEN));
        scene.undo();
        scene.edit(0, Group::Nudge, rect(Color32::BLUE));
        scene.undo();
        assert_eq!(scene.shapes()[0], rect(Color32::RED));

        scene.edit(0, Group::Nudge, rect(Color32::GREEN));
        scene.push(counter(1));
        scene.edit(0, Group::Nudge, rect(Color32::BLUE));
        scene.undo();
        assert_eq!(scene.shapes()[0], rect(Color32::GREEN));
        assert_eq!(scene.shapes().len(), 2);

        scene.edit(0, Group::Nudge, rect(Color32::WHITE));
        scene.break_group();
        scene.edit(0, Group::Nudge, rect(Color32::BLACK));
        scene.undo();
        assert_eq!(scene.shapes()[0], rect(Color32::WHITE));
    }

    #[test]
    fn removing_a_shape_can_be_undone() {
        let mut scene = Scene::default();
        scene.push(rect(Color32::RED));
        scene.push(counter(1));
        assert!(scene.remove(0));
        assert_eq!(scene.shapes(), &[counter(1)]);
        scene.undo();
        assert_eq!(scene.shapes(), &[rect(Color32::RED), counter(1)]);
        assert!(!scene.remove(2));
        scene.undo();
        assert_eq!(
            scene.shapes(),
            &[rect(Color32::RED)],
            "a miss adds no undo step"
        );
    }

    #[test]
    fn duplicating_offsets_the_copy_on_top() {
        let mut scene = Scene::default();
        scene.push(rect(Color32::RED));
        scene.push(counter(1));
        assert_eq!(scene.duplicate(0, egui::vec2(16.0, 16.0)), Some(2));
        assert_eq!(
            scene.shapes()[2],
            translated(&rect(Color32::RED), egui::vec2(16.0, 16.0))
        );
        assert_eq!(scene.duplicate(1, egui::vec2(16.0, 16.0)), Some(3));
        match &scene.shapes()[3] {
            Shape::Counter { number, center, .. } => {
                assert_eq!(*number, 2);
                assert_eq!(*center, pos2(16.0, 16.0));
            }
            _ => panic!("not a counter"),
        }
        assert_eq!(scene.duplicate(9, egui::vec2(16.0, 16.0)), None);
        scene.undo();
        scene.undo();
        assert_eq!(scene.shapes().len(), 2);
    }

    #[test]
    fn markers_count_up_from_one() {
        assert_eq!(next_counter(&[]), 1);
        assert_eq!(next_counter(&[counter(1), counter(2)]), 3);
    }

    #[test]
    fn marker_numbers_follow_the_highest_left() {
        let line = Shape::Line {
            from: pos2(0.0, 0.0),
            to: pos2(5.0, 5.0),
            color: Color32::RED,
            width: 3.0,
        };
        assert_eq!(next_counter(&[counter(3), line.clone(), counter(1)]), 4);
        assert_eq!(next_counter(&[line]), 1);
    }

    #[test]
    fn marker_digits_contrast_with_their_fill() {
        assert_eq!(text_on(Color32::from_rgb(255, 230, 0)), Color32::BLACK);
        assert_eq!(text_on(Color32::WHITE), Color32::BLACK);
        assert_eq!(text_on(Color32::from_rgb(230, 30, 30)), Color32::WHITE);
        assert_eq!(text_on(Color32::from_rgb(20, 60, 200)), Color32::WHITE);
        assert_eq!(text_on(Color32::BLACK), Color32::WHITE);
    }

    #[test]
    fn an_ellipse_is_hit_on_its_outline_only() {
        let e = Shape::Ellipse {
            rect: egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 100.0)),
            color: Color32::RED,
            width: 3.0,
        };
        let shapes = [e];
        assert_eq!(shape_hit(&shapes, pos2(200.0, 50.0)), Some(0), "right edge");
        assert_eq!(shape_hit(&shapes, pos2(100.0, 1.0)), Some(0), "top edge");
        assert_eq!(shape_hit(&shapes, pos2(100.0, 50.0)), None, "middle");
        assert_eq!(
            shape_hit(&shapes, pos2(5.0, 5.0)),
            None,
            "corner outside the curve"
        );
    }

    fn painted_reach(shape: &Shape) -> f32 {
        let ctx = egui::Context::default();
        let output = ctx.run_ui(Default::default(), |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::background());
            paint(
                &painter,
                shape,
                &|p| p,
                1.0,
                egui::TextureId::default(),
                1000.0,
                1000.0,
            );
        });
        let center = pos2(500.0, 500.0);
        ctx.tessellate(output.shapes, 1.0)
            .iter()
            .flat_map(|p| match &p.primitive {
                egui::epaint::Primitive::Mesh(m) => m.vertices.clone(),
                _ => vec![],
            })
            .map(|v| (v.pos.x - center.x).abs().max((v.pos.y - center.y).abs()))
            .fold(0.0, f32::max)
    }

    #[test]
    fn a_flat_ellipse_stays_where_it_was_drawn() {
        for (rx, ry, width) in [
            (5.0, 0.001, 20.0),
            (40.0, 0.0005, 6.0),
            (0.0005, 30.0, 6.0),
            (2.0, 0.01, 3.0),
        ] {
            let e = Shape::Ellipse {
                rect: egui::Rect::from_center_size(
                    pos2(500.0, 500.0),
                    egui::vec2(rx * 2.0, ry * 2.0),
                ),
                color: Color32::RED,
                width,
            };
            let reach = painted_reach(&e);
            assert!(
                reach <= f32::max(rx, ry) + width + 2.0,
                "r=({rx},{ry}) w={width} reach={reach}"
            );
        }
    }

    #[test]
    fn a_thin_ellipse_is_no_longer_than_its_outline_would_be() {
        let e = Shape::Ellipse {
            rect: egui::Rect::from_center_size(pos2(500.0, 500.0), egui::vec2(8.0, 6.0)),
            color: Color32::RED,
            width: 20.0,
        };
        let reach = painted_reach(&e);
        assert!(reach <= 4.0 + 10.0 + 1.0, "{reach}");
    }

    #[test]
    fn a_normal_ellipse_still_reaches_its_outline() {
        let e = Shape::Ellipse {
            rect: egui::Rect::from_center_size(pos2(500.0, 500.0), egui::vec2(200.0, 100.0)),
            color: Color32::RED,
            width: 4.0,
        };
        let reach = painted_reach(&e);
        assert!((100.0..=106.0).contains(&reach), "{reach}");
    }

    #[test]
    fn a_marker_is_hit_anywhere_on_its_disc() {
        let shapes = [counter(1)];
        assert_eq!(shape_hit(&shapes, pos2(3.0, 4.0)), Some(0));
        assert_eq!(shape_hit(&shapes, pos2(40.0, 0.0)), None);
    }

    #[test]
    fn new_shapes_move_with_a_drag() {
        let d = egui::vec2(10.0, -5.0);
        match translated(&counter(2), d) {
            Shape::Counter { center, number, .. } => {
                assert_eq!(center, pos2(10.0, -5.0));
                assert_eq!(number, 2);
            }
            _ => panic!("still a marker"),
        }
        let e = Shape::Ellipse {
            rect: egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(20.0, 10.0)),
            color: Color32::RED,
            width: 3.0,
        };
        match translated(&e, d) {
            Shape::Ellipse { rect, .. } => assert_eq!(rect.min, pos2(10.0, -5.0)),
            _ => panic!("still an ellipse"),
        }
    }
}
