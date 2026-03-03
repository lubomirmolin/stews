use ratatui::{
    prelude::*,
    symbols::Marker,
    text::Line as TextLine,
    widgets::{
        Block, Borders, Paragraph,
        canvas::{Canvas, Circle, Line as CanvasLine, Points},
    },
};

const PANEL_BG: Color = Color::Rgb(8, 13, 22);
const PANEL_BORDER: Color = Color::Rgb(58, 71, 112);
const RING_OUTER: Color = Color::Rgb(92, 213, 255);
const RING_INNER: Color = Color::Rgb(121, 132, 255);
const TOKEN_COLOR: Color = Color::Rgb(175, 187, 240);
const CORE_COLOR: Color = Color::Rgb(98, 255, 221);
const SUBTITLE: &str = "json workbench // live neon";

pub fn render(frame: &mut Frame, area: Rect, seconds: f64) {
    if area.width < 16 || area.height < 5 {
        return;
    }

    let block = Block::default()
        .title("  STEWS CORE  ")
        .borders(Borders::ALL)
        .style(Style::default().bg(PANEL_BG))
        .border_style(Style::default().fg(PANEL_BORDER));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 8 || inner.height < 4 {
        return;
    }

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(inner);

    render_canvas(frame, sections[0], seconds);
    render_label(frame, sections[1], seconds);
}

fn render_canvas(frame: &mut Frame, area: Rect, seconds: f64) {
    let canvas = Canvas::default()
        .background_color(PANEL_BG)
        .marker(Marker::Braille)
        .x_bounds([-110.0, 110.0])
        .y_bounds([-70.0, 70.0])
        .paint(|ctx| {
            let outer_points = ring_points(48.0, seconds * 0.85, 120, 0.74);
            ctx.draw(&Points {
                coords: &outer_points,
                color: RING_OUTER,
            });

            let inner_points = ring_points(34.0, -seconds * 1.35, 96, 0.58);
            ctx.draw(&Points {
                coords: &inner_points,
                color: RING_INNER,
            });

            ctx.draw(&Circle {
                x: 0.0,
                y: 0.0,
                radius: 18.0,
                color: CORE_COLOR,
            });

            let spokes = 6;
            for i in 0..spokes {
                let angle = (i as f64 / spokes as f64) * std::f64::consts::TAU + (seconds * 0.55);
                let (sin, cos) = angle.sin_cos();
                ctx.draw(&CanvasLine {
                    x1: cos * 22.0,
                    y1: sin * 15.0,
                    x2: cos * 36.0,
                    y2: sin * 26.0,
                    color: CORE_COLOR,
                });
            }

            let fragments = ["{}", "[]", "\"k\"", ":1", "true", "null"];
            for (idx, token) in fragments.iter().enumerate() {
                let phase =
                    seconds * 1.4 + idx as f64 * (std::f64::consts::TAU / fragments.len() as f64);
                let radius = 26.0 + ((seconds * 2.4 + idx as f64).sin() * 3.5);
                let x = radius * phase.cos();
                let y = (radius * 0.62) * phase.sin();
                ctx.print(
                    x,
                    y,
                    TextLine::styled(*token, Style::default().fg(TOKEN_COLOR)),
                );
            }
        });

    frame.render_widget(canvas, area);
}

fn render_label(frame: &mut Frame, area: Rect, seconds: f64) {
    let pulse = (seconds * 3.2).sin() * 0.5 + 0.5;

    let mut spans = vec![Span::raw(" ")];
    for (idx, ch) in ['S', 'T', 'E', 'W', 'S'].iter().enumerate() {
        let t = (pulse + idx as f64 * 0.11).fract();
        let letter_color = lerp_color(Color::Rgb(122, 138, 210), Color::Rgb(110, 248, 255), t);
        spans.push(Span::styled(
            ch.to_string(),
            Style::default()
                .fg(letter_color)
                .bg(PANEL_BG)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    }

    let text = vec![
        Line::from(spans),
        Line::from(Span::styled(
            format!(" {}", SUBTITLE),
            Style::default().fg(Color::Rgb(108, 119, 154)).bg(PANEL_BG),
        )),
    ];

    frame.render_widget(
        Paragraph::new(text).style(Style::default().bg(PANEL_BG)),
        area,
    );
}

fn ring_points(radius: f64, rotation: f64, count: usize, y_scale: f64) -> Vec<(f64, f64)> {
    let mut points = Vec::with_capacity(count);
    for idx in 0..count {
        let angle = rotation + (idx as f64 / count as f64) * std::f64::consts::TAU;
        let jitter = ((rotation * 2.1) + idx as f64 * 0.23).sin() * 1.5;
        points.push((
            (radius + jitter) * angle.cos(),
            (radius + jitter) * angle.sin() * y_scale,
        ));
    }
    points
}

fn lerp_color(a: Color, b: Color, t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    match (a, b) {
        (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => Color::Rgb(
            (ar as f64 + (br as f64 - ar as f64) * t) as u8,
            (ag as f64 + (bg as f64 - ag as f64) * t) as u8,
            (ab as f64 + (bb as f64 - ab as f64) * t) as u8,
        ),
        _ => b,
    }
}
