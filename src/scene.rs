#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Pen,
    Line,
    Arrow,
    Rect,
    Marker,
    Text,
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
    }
}
