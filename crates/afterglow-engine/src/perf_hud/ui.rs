use bevy::prelude::*;
use bevy::window::{Monitor, PrimaryMonitor};

use super::data::PerfData;

#[derive(Component)]
pub struct HudRoot;

#[derive(Component)]
pub struct FpsText;

#[derive(Component)]
pub struct FrameTimeText;

#[derive(Component)]
pub struct FrameBar;

#[derive(Component)]
pub struct TraceHistBar;

#[derive(Component)]
pub struct TraceSeg;

#[derive(Component)]
pub struct SysLegendItem;

#[derive(Component)]
pub struct BarLerp(pub f32);

const LERP_SPEED: f32 = 0.15;
const BAR_W: f32 = 3.0;
const GAP: f32 = 1.0;
const BARS: usize = 60;
const FT_H: f32 = 30.0;
const TRACE_H: f32 = 30.0;
const FONT_SZ: f32 = 10.0;
const MAX_TRACE: usize = 5;

const COLORS: &[Color] = &[
    Color::srgb(0.2, 0.8, 1.0),
    Color::srgb(1.0, 0.4, 0.3),
    Color::srgb(0.3, 1.0, 0.4),
    Color::srgb(1.0, 0.9, 0.2),
    Color::srgb(0.9, 0.4, 1.0),
    Color::srgb(0.6, 0.6, 0.8),
    Color::srgb(0.8, 0.5, 0.2),
    Color::srgb(0.2, 0.9, 0.9),
];

fn span_color(idx: usize) -> Color {
    COLORS[idx % COLORS.len()]
}

pub fn spawn_hud(mut commands: Commands) {
    commands
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(4.0),
                right: Val::Px(4.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(4.0)),
                row_gap: Val::Px(2.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
            GlobalZIndex(i32::MAX - 16),
            Visibility::Visible,
        ))
        .with_children(|r| {
            r.spawn((FpsText, Text("".into()), TextFont { font_size: FONT_SZ, ..default() }, TextColor(Color::srgb(0.0, 1.0, 0.6))));
            r.spawn((FrameTimeText, Text("".into()), TextFont { font_size: FONT_SZ, ..default() }, TextColor(Color::srgb(1.0, 0.6, 0.2))));

            // FT bar history
            let w = BARS as f32 * (BAR_W + GAP);
            r.spawn((Node { width: Val::Px(w), height: Val::Px(FT_H), flex_direction: FlexDirection::Row, align_items: AlignItems::End, ..default() },)).with_children(|r| {
                for _ in 0..BARS {
                    r.spawn((
                        FrameBar,
                        BarLerp(0.0),
                        Node { width: Val::Px(BAR_W), height: Val::Px(0.0), margin: UiRect::right(Val::Px(GAP)), ..default() },
                        BackgroundColor(Color::BLACK),
                    ));
                }
            });
            r.spawn((Text("frame time".into()), TextFont { font_size: FONT_SZ * 0.75, ..default() }, TextColor(Color::srgb(0.6, 0.6, 0.6))));

            // Trace history bars: 60 stacked compound bars
            r.spawn((Text("system trace history".into()), TextFont { font_size: FONT_SZ * 0.75, ..default() }, TextColor(Color::srgb(0.6, 0.6, 0.6))));
            r.spawn((Node { width: Val::Px(w), height: Val::Px(TRACE_H), flex_direction: FlexDirection::Row, align_items: AlignItems::End, ..default() },)).with_children(|r| {
                for _ in 0..BARS {
                    r.spawn((
                        TraceHistBar,
                        Node { width: Val::Px(BAR_W), height: Val::Px(TRACE_H), flex_direction: FlexDirection::Column, justify_content: JustifyContent::End, margin: UiRect::right(Val::Px(GAP)), ..default() },
                    )).with_children(|r| {
                        for _ in 0..MAX_TRACE {
                            r.spawn((
                                TraceSeg,
                                Node { width: Val::Px(BAR_W), height: Val::Px(0.0), ..default() },
                                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.0)),
                            ));
                        }
                    });
                }
            });

            // Legend: top 5 trace system names
            r.spawn((Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(0.0), ..default() },)).with_children(|r| {
                for _ in 0..MAX_TRACE {
                    r.spawn((
                        SysLegendItem,
                        Text("".into()),
                        TextFont { font_size: FONT_SZ * 0.8, ..default() },
                        TextColor(Color::srgb(0.8, 0.8, 0.8)),
                    ));
                }
            });
        });
}

pub fn update_hud(
    mut data: ResMut<PerfData>,
    trace_accum: Option<Res<super::trace_collector::TraceData>>,
    monitor: Query<&Monitor, With<PrimaryMonitor>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut hud: Query<&mut Visibility, With<HudRoot>>,
    mut text_group: ParamSet<(
        Query<&mut Text, (With<FpsText>, Without<FrameTimeText>)>,
        Query<&mut Text, (With<FrameTimeText>, Without<FpsText>)>,
        Query<(&mut Text, &mut TextColor), With<SysLegendItem>>,
    )>,
    mut frame_bars: Query<(&mut Node, &mut BackgroundColor, &mut BarLerp), (With<FrameBar>, Without<TraceSeg>, Without<TraceHistBar>)>,
    trace_bar_ents: Query<Entity, (With<TraceHistBar>, Without<FrameBar>, Without<TraceSeg>)>,
    children_q: Query<&Children, (With<TraceHistBar>, Without<FrameBar>)>,
    mut trace_segs: Query<(&mut Node, &mut BackgroundColor), (With<TraceSeg>, Without<FrameBar>, Without<TraceHistBar>)>,
) {
    if keys.just_pressed(KeyCode::Backquote)
        && (keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight))
    {
        for mut v in &mut hud {
            *v = if *v == Visibility::Visible { Visibility::Hidden } else { Visibility::Visible };
        }
    }
    let refresh_hz = monitor.iter().next()
        .and_then(|m| m.refresh_rate_millihertz)
        .map(|mhz| mhz as f64 / 1000.0)
        .unwrap_or(60.0);

    let green_threshold_ms = 1000.0 / (refresh_hz * 0.95);
    let red_threshold_ms = 33.33;

    // Capture current trace accum as a snapshot for this frame
    let mut current_trace: Vec<(String, f64)> = Vec::new();
    if let Some(td) = trace_accum {
        if let Ok(acc) = td.accum.lock() {
            for (name, (total, _count)) in acc.iter() {
                current_trace.push((name.clone(), *total));
            }
        }
    }
    current_trace.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    current_trace.truncate(MAX_TRACE);
    data.trace_snapshots.push(current_trace.clone());
    if data.trace_snapshots.len() > BARS {
        data.trace_snapshots.remove(0);
    }

    // FPS text row
    for mut t in &mut text_group.p0() {
        let fpss: Vec<f64> = data.history.iter().map(|s| s.fps).collect();
        let cur = data.history.last().map(|s| s.fps as u64).unwrap_or(0);
        let min = fpss.iter().cloned().fold(f64::MAX, f64::min) as u64;
        let max = fpss.iter().cloned().fold(0.0f64, f64::max) as u64;
        let avg = fpss.iter().sum::<f64>() / fpss.len().max(1) as f64;
        let mut sfps = fpss.clone();
        sfps.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
        let p5 = sfps.get((sfps.len() as f64 * 0.05) as usize).copied().unwrap_or(0.0) as u64;
        let p1 = sfps.get((sfps.len() as f64 * 0.01) as usize).copied().unwrap_or(0.0) as u64;
        t.0 = format!("FPS {}  MIN {}  MAX {}  AVG {:.0}  P5 {}  P1 {}", cur, min, max, avg, p5, p1);
    }

    // Frame time text row
    for mut t in &mut text_group.p1() {
        let last = data.history.last();
        let mut sorted: Vec<f64> = data.history.iter().map(|s| s.frame_time_ms).collect();
        sorted.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
        let avg = sorted.iter().sum::<f64>() / sorted.len().max(1) as f64;
        let p95 = sorted.get((sorted.len() as f64 * 0.95) as usize).copied().unwrap_or(0.0);
        let p99 = sorted.get((sorted.len() as f64 * 0.99) as usize).copied().unwrap_or(0.0);
        t.0 = format!(
            "FT {:.1}ms  AVG {:.1}  P95 {:.1}  P99 {:.1}  @{}Hz",
            last.map(|s| s.frame_time_ms).unwrap_or(0.0),
            avg, p95, p99,
            refresh_hz as u64,
        );
    }

    // Frame time bars (lerp-smoothed)
    for (sample, (mut node, mut bg, mut lerp)) in data.history.iter().rev().zip(frame_bars.iter_mut()) {
        let ft_val = sample.frame_time_ms as f32;
        let ratio = (ft_val / red_threshold_ms as f32).clamp(0.0, 1.0);
        let target = (ratio * FT_H).max(1.0);
        lerp.0 += (target - lerp.0) * LERP_SPEED;
        node.height = Val::Px(lerp.0);
        if ft_val <= green_threshold_ms as f32 {
            bg.0 = Color::hsl(120.0, 0.9, 0.5);
        } else if ft_val >= red_threshold_ms as f32 {
            bg.0 = Color::hsl(0.0, 0.9, 0.5);
        } else {
            let t = (ft_val - green_threshold_ms as f32) / (red_threshold_ms as f32 - green_threshold_ms as f32);
            bg.0 = Color::hsl(120.0 * (1.0 - t), 0.9, 0.5);
        }
    }

    // Trace history bars: 60-frame compound stacked bars
    let all_times: Vec<f64> = data.trace_snapshots.iter().flat_map(|s| s.iter().map(|(_, ms)| *ms)).collect();
    let trace_max = all_times.iter().cloned().fold(0.0f64, f64::max).max(0.001) as f32;

    let bar_entities: Vec<Entity> = trace_bar_ents.iter().collect();
    for (snap, &bar_ent) in data.trace_snapshots.iter().rev().zip(bar_entities.iter()) {
        let mut sorted: Vec<(String, f64)> = snap.clone();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));

        if let Ok(children) = children_q.get(bar_ent) {
            for (slot, child) in children.iter().enumerate() {
                if let Ok((mut node, mut bg)) = trace_segs.get_mut(child) {
                    if let Some((_name, ms)) = sorted.get(slot) {
                        let h = (*ms as f32 / trace_max * TRACE_H).max(0.0);
                        node.height = Val::Px(h);
                        bg.0 = span_color(slot);
                    } else {
                        node.height = Val::Px(0.0);
                    }
                }
            }
        }
    }

    // Legend: top 5 trace names by cumulative time across the window
    let mut cum: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for snap in &data.trace_snapshots {
        for (name, ms) in snap {
            *cum.entry(name.clone()).or_insert(0.0) += ms;
        }
    }
    let mut cum_sorted: Vec<(String, f64)> = cum.into_iter().collect();
    cum_sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    for (i, (mut t, mut tc)) in text_group.p2().iter_mut().enumerate() {
        if let Some((name, ms)) = cum_sorted.get(i) {
            t.0 = format!("{}  {:.2}ms", name, ms);
            tc.0 = span_color(i);
        } else {
            t.0 = String::new();
        }
    }
}
