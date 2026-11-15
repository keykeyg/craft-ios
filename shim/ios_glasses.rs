//! craft-ios glasses mode: plug display glasses (or any external screen) into the phone and the
//! app's own window moves to them, with the desktop layout; the phone becomes the controller.
//!
//! The controller is a second egui viewport on the phone's screen. Its trackpad gestures (after
//! Keyan's Latch remote-desktop app) become pointer events for the app on the glasses: one finger
//! moves the cursor, tap clicks, two-finger tap right-clicks, two-finger drag scrolls, pinch zooms.
//! Above the trackpad sits a Touch Bar-style strip of commands and sticky modifier keys. Keys from
//! a hardware keyboard or the on-screen keyboard go to the app too.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use egui::{
    Align2, Color32, Context, CornerRadius, Event, FontId, Id, Key, LayerId, Modifiers, Order, Pos2,
    Rect, Sense, Shape, Stroke, StrokeKind, TouchPhase, Ui, Vec2, ViewportBuilder, ViewportClass,
    ViewportId,
};

const CONTROLLER: &str = "loom-glasses-controller";
const SPEED: f32 = 2.2; // cursor points per finger point
const TAP_MOVE: f32 = 10.0;
const TAP_TIME: Duration = Duration::from_millis(320);
const ROW: f32 = 48.0;

#[derive(Clone, Copy, PartialEq)]
enum Stick {
    Off,
    Once,
    Lock,
}

struct Gesture {
    start: Instant,
    fingers: usize,
    max_fingers: usize,
    moved: f32,
    last_centroid: Option<Pos2>,
    last_spread: Option<f32>,
    drag_held: bool,
}

struct State {
    /// The app's window is on the external display.
    active: bool,
    cursor: Pos2,
    screen: Rect,
    queue: Vec<Event>,
    ctrl: Stick,
    alt: Stick,
    shift: Stick,
    cmd: Stick,
    drag_lock: bool,
    keyboard: bool,
    gesture: Option<Gesture>,
    touches: Vec<(egui::TouchId, Pos2)>,
}

static STATE: Mutex<State> = Mutex::new(State {
    active: false,
    cursor: Pos2::ZERO,
    screen: Rect::NOTHING,
    queue: Vec::new(),
    ctrl: Stick::Off,
    alt: Stick::Off,
    shift: Stick::Off,
    cmd: Stick::Off,
    drag_lock: false,
    keyboard: false,
    gesture: None,
    touches: Vec::new(),
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

// ------------------------------------------------------------------- window placement (root)

/// Put the app's window on the external display while one is connected, back on the phone
/// when it goes. Called for the root viewport every frame; polls once a second for changes.
pub(crate) fn sync_window(
    event_loop: &winit::event_loop::ActiveEventLoop,
    window: &winit::window::Window,
    ctx: &Context,
) {
    let primary = event_loop.primary_monitor();
    // Only a real external screen: the phone's own display is "Primary" (on the Duo too).
    let external = event_loop
        .available_monitors()
        .find(|m| Some(m) != primary.as_ref() && m.name().as_deref() != Some("Primary"));
    let mut st = state();
    match (external, st.active) {
        (Some(m), false) => {
            window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(Some(m))));
            st.active = true;
            st.cursor = Pos2::ZERO;
            ctx.request_repaint();
        }
        (None, true) => {
            window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(primary)));
            st.active = false;
            st.queue.clear();
            ctx.request_repaint();
        }
        _ => {}
    }
    drop(st);
    ctx.request_repaint_after_for(Duration::from_secs(1), ViewportId::ROOT);
}

// ------------------------------------------------------------------------ root frame hooks

/// Before the app's frame: touch sizes on the phone, desktop sizes on the glasses, and the
/// controller's queued input. Only for the app's own (root) viewport.
pub(crate) fn before_frame(ctx: &Context, raw_input: &mut egui::RawInput, root: bool) {
    if !root {
        return;
    }
    let mut st = state();
    crate::ios_touch::apply(ctx, !st.active);
    if st.active {
        raw_input.events.append(&mut st.queue);
    }
}

/// After the app's UI: draw the cursor on the glasses and keep the controller open.
pub(crate) fn after_ui(ctx: &Context) {
    let mut st = state();
    if !st.active {
        return;
    }
    let screen = ctx.content_rect();
    st.screen = screen;
    if st.cursor == Pos2::ZERO || !screen.contains(st.cursor) {
        st.cursor = screen.clamp(if st.cursor == Pos2::ZERO { screen.center() } else { st.cursor });
    }
    let p = st.cursor;
    let pressed = st.gesture.as_ref().is_some_and(|g| g.drag_held) || st.drag_lock;
    drop(st);

    let painter = ctx.layer_painter(LayerId::new(Order::Debug, Id::unique("loom-glasses-cursor")));
    let s = 1.25;
    let fill = if pressed { Color32::from_rgb(10, 132, 255) } else { Color32::WHITE };
    let at = |x: f32, y: f32| p + Vec2::new(x * s, y * s);
    // Arrow: a convex head plus a tail, each outlined so it reads on any background.
    painter.line_segment([at(5.5, 11.0), at(9.0, 19.0)], Stroke::new(5.5 * s, Color32::BLACK));
    painter.line_segment([at(5.5, 11.0), at(9.0, 19.0)], Stroke::new(3.0 * s, fill));
    painter.add(Shape::convex_polygon(vec![at(0.0, 0.0), at(0.0, 17.0), at(12.5, 12.5)], fill, Stroke::new(1.5, Color32::BLACK)));

    // On iOS eframe draws extra viewports inside the main window; the controller needs a window
    // of its own on the phone, so embedding is off just for this one.
    let embed = ctx.embed_viewports();
    ctx.set_embed_viewports(false);
    ctx.show_viewport_deferred(
        ViewportId::from_hash_of(CONTROLLER),
        ViewportBuilder::default().with_title("Loom controller"),
        controller_ui,
    );
    ctx.set_embed_viewports(embed);
}

// ------------------------------------------------------------------------------ controller

fn modifiers(st: &State) -> Modifiers {
    let cmd = st.cmd != Stick::Off;
    Modifiers { alt: st.alt != Stick::Off, ctrl: st.ctrl != Stick::Off, shift: st.shift != Stick::Off, mac_cmd: cmd, command: cmd }
}

/// One-shot modifiers end after the next click or key.
fn release_once(st: &mut State) {
    for m in [&mut st.ctrl, &mut st.alt, &mut st.shift, &mut st.cmd] {
        if *m == Stick::Once {
            *m = Stick::Off;
        }
    }
}

thread_local! {
    static CLIPBOARD: std::cell::RefCell<Option<egui_winit::clipboard::Clipboard>> =
        const { std::cell::RefCell::new(None) };
}

/// The system pasteboard's text, for the paste button.
#[allow(unsafe_code)]
fn paste(ctx: &Context) {
    let text = CLIPBOARD.with(|c| {
        // SAFETY: no display handle is passed, so nothing is borrowed.
        c.borrow_mut().get_or_insert_with(|| unsafe { egui_winit::clipboard::Clipboard::new(None) }).get()
    });
    if let Some(text) = text {
        send(ctx, [Event::Paste(text)]);
    }
}

fn send(ctx: &Context, events: impl IntoIterator<Item = Event>) {
    state().queue.extend(events);
    ctx.request_repaint_of(ViewportId::ROOT);
}

fn click(ctx: &Context, button: egui::PointerButton) {
    let mut st = state();
    let (pos, mods) = (st.cursor, modifiers(&st));
    release_once(&mut st);
    drop(st);
    send(ctx, [
        Event::PointerMoved(pos),
        Event::PointerButton { pos, button, pressed: true, modifiers: mods },
        Event::PointerButton { pos, button, pressed: false, modifiers: mods },
    ]);
}

fn key(ctx: &Context, key: Key, extra: Modifiers) {
    let mut st = state();
    let mods = modifiers(&st) | extra;
    release_once(&mut st);
    drop(st);
    send(ctx, [
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: mods },
        Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: mods },
    ]);
}

fn command(ctx: &Context, k: Key) {
    key(ctx, k, Modifiers::COMMAND | Modifiers::MAC_CMD);
}

/// A Touch Bar entry: an app shortcut ("Cmd+Shift+G", "V", "Space") or `type:WORD`, which
/// types a command and presses Return (CAD's command line).
fn run_entry(ctx: &Context, keys: &str) {
    if let Some(word) = keys.strip_prefix("type:") {
        send(ctx, [Event::Text(word.to_string())]);
        key(ctx, Key::Enter, Modifiers::NONE);
        return;
    }
    let mut mods = Modifiers::NONE;
    let mut parts: Vec<&str> = keys.split('+').collect();
    // "Cmd+=" and "Cmd+-" split cleanly; "Cmd++" would not, so treat a trailing "" as "+".
    let last = match parts.pop() {
        Some("") => "Plus",
        Some(k) => k,
        None => return,
    };
    for m in parts {
        match m {
            "Cmd" | "Mod" => mods = mods | Modifiers::COMMAND | Modifiers::MAC_CMD,
            "Ctrl" => mods = mods | Modifiers::CTRL,
            "Shift" => mods = mods | Modifiers::SHIFT,
            "Alt" | "Opt" => mods = mods | Modifiers::ALT,
            _ => {}
        }
    }
    let k = match last {
        "Left" => Some(Key::ArrowLeft),
        "Right" => Some(Key::ArrowRight),
        "Up" => Some(Key::ArrowUp),
        "Down" => Some(Key::ArrowDown),
        "Return" => Some(Key::Enter),
        "Esc" => Some(Key::Escape),
        "=" => Some(Key::Equals),
        "-" => Some(Key::Minus),
        "[" => Some(Key::OpenBracket),
        "]" => Some(Key::CloseBracket),
        "," => Some(Key::Comma),
        "." => Some(Key::Period),
        "/" => Some(Key::Slash),
        other => Key::from_name(other),
    };
    if let Some(k) = k {
        key(ctx, k, mods);
    }
}

fn capsule(ui: &mut Ui, rect: Rect, label: &str, on: bool) -> egui::Response {
    let resp = ui.allocate_rect(rect, Sense::click());
    let fill = if on {
        Color32::from_rgb(10, 132, 255)
    } else if resp.is_pointer_button_down_on() {
        Color32::from_gray(70)
    } else {
        Color32::from_gray(44)
    };
    ui.painter().rect_filled(rect, CornerRadius::same((rect.height() / 2.0) as u8), fill);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(16.0), Color32::WHITE);
    resp
}

fn controller_ui(ui: &mut Ui, _class: ViewportClass) {
    let ctx = ui.ctx().clone();
    let full = ctx.content_rect();
    ui.painter().rect_filled(full, CornerRadius::ZERO, Color32::from_gray(18));
    let pad = 10.0;
    let mut y = full.top() + pad;
    let width = full.width() - 2.0 * pad;

    // Touch Bar-style command strip.
    let cmds: [(&str, &dyn Fn(&Context)); 8] = [
        ("esc", &|c| key(c, Key::Escape, Modifiers::NONE)),
        ("undo", &|c| command(c, Key::Z)),
        ("redo", &|c| key(c, Key::Z, Modifiers::COMMAND | Modifiers::MAC_CMD | Modifiers::SHIFT)),
        ("cut", &|c| send(c, [Event::Cut])),
        ("copy", &|c| send(c, [Event::Copy])),
        ("paste", &paste),
        ("save", &|c| command(c, Key::S)),
        ("delete", &|c| key(c, Key::Delete, Modifiers::NONE)),
    ];
    let per_row = if width > 600.0 { 8 } else { 4 };
    let w = (width - (per_row as f32 - 1.0) * 8.0) / per_row as f32;
    for (i, (label, f)) in cmds.iter().enumerate() {
        let (r, c) = (i / per_row, i % per_row);
        let rect = Rect::from_min_size(Pos2::new(full.left() + pad + c as f32 * (w + 8.0), y + r as f32 * (ROW + 8.0)), Vec2::new(w, ROW));
        if capsule(ui, rect, label, false).clicked() {
            f(&ctx);
        }
    }
    y += (cmds.len().div_ceil(per_row)) as f32 * (ROW + 8.0);

    // The app's own row (tools and commands), like the Mac Touch Bar.
    let app = crate::ios_bar::APP;
    if !app.is_empty() {
        ui.painter().line_segment(
            [Pos2::new(full.left() + pad, y - 2.0), Pos2::new(full.right() - pad, y - 2.0)],
            Stroke::new(1.0, Color32::from_gray(50)),
        );
        y += 6.0;
        for (i, (label, keys)) in app.iter().enumerate() {
            let (r, c) = (i / per_row, i % per_row);
            let rect = Rect::from_min_size(
                Pos2::new(full.left() + pad + c as f32 * (w + 8.0), y + r as f32 * (ROW + 8.0)),
                Vec2::new(w, ROW),
            );
            if capsule(ui, rect, label, false).clicked() {
                run_entry(&ctx, keys);
            }
        }
        y += app.len().div_ceil(per_row) as f32 * (ROW + 8.0);
    }

    // Sticky modifiers: tap once for the next click or key, tap again to lock, again to release.
    let labels = ["⌃ ctrl", "⌥ opt", "⇧ shift", "⌘ cmd", "drag"];
    let w = (width - 4.0 * 8.0) / 5.0;
    for (i, label) in labels.iter().enumerate() {
        let rect = Rect::from_min_size(Pos2::new(full.left() + pad + i as f32 * (w + 8.0), y), Vec2::new(w, ROW));
        let st = state();
        let on = match i {
            0 => st.ctrl != Stick::Off,
            1 => st.alt != Stick::Off,
            2 => st.shift != Stick::Off,
            3 => st.cmd != Stick::Off,
            _ => st.drag_lock,
        };
        let locked = match i {
            0 => st.ctrl == Stick::Lock,
            1 => st.alt == Stick::Lock,
            2 => st.shift == Stick::Lock,
            3 => st.cmd == Stick::Lock,
            _ => false,
        };
        drop(st);
        let text = if locked { format!("{label} •") } else { (*label).to_string() };
        if capsule(ui, rect, &text, on).clicked() {
            let mut st = state();
            let cycle = |m: &mut Stick| *m = match *m { Stick::Off => Stick::Once, Stick::Once => Stick::Lock, Stick::Lock => Stick::Off };
            match i {
                0 => cycle(&mut st.ctrl),
                1 => cycle(&mut st.alt),
                2 => cycle(&mut st.shift),
                3 => cycle(&mut st.cmd),
                _ => {
                    st.drag_lock = !st.drag_lock;
                    let (pos, mods) = (st.cursor, modifiers(&st));
                    let pressed = st.drag_lock;
                    drop(st);
                    send(&ctx, [Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: mods }]);
                }
            }
        }
    }
    y += ROW + 10.0;

    // Keyboard row.
    let kb_rect = Rect::from_min_size(Pos2::new(full.left() + pad, full.bottom() - pad - ROW), Vec2::new(width, ROW));
    let keyboard_on = state().keyboard;
    if capsule(ui, kb_rect, if keyboard_on { "hide keyboard" } else { "keyboard" }, keyboard_on).clicked() {
        let mut st = state();
        st.keyboard = !st.keyboard;
    }
    let text_id = Id::unique("loom-glasses-text");
    if state().keyboard {
        // An off-screen text field only to raise the on-screen keyboard; what's typed goes on.
        let mut sink = String::new();
        let field = Rect::from_min_size(Pos2::new(full.left() - 400.0, full.top()), Vec2::new(10.0, 10.0));
        let resp = ui.put(field, egui::TextEdit::singleline(&mut sink).id(text_id));
        if !resp.has_focus() {
            resp.request_focus();
        }
    } else {
        ctx.memory_mut(|m| m.surrender_focus(text_id));
    }

    // Trackpad.
    let pad_rect = Rect::from_min_max(Pos2::new(full.left() + pad, y), Pos2::new(full.right() - pad, kb_rect.top() - 10.0));
    ui.painter().rect(pad_rect, CornerRadius::same(18), Color32::from_gray(30), Stroke::new(1.0, Color32::from_gray(60)), StrokeKind::Inside);
    ui.painter().text(pad_rect.center(), Align2::CENTER_CENTER, "trackpad", FontId::proportional(15.0), Color32::from_gray(90));
    let _ = ui.allocate_rect(pad_rect, Sense::click_and_drag());

    let events = ctx.input(|i| i.events.clone());
    for ev in events {
        match ev {
            Event::Touch { id, phase, pos, .. } if pad_rect.contains(pos) || phase != TouchPhase::Start => {
                touch(&ctx, id, phase, pos, pad_rect);
            }
            Event::Text(_) | Event::Paste(_) | Event::Copy | Event::Cut => send(&ctx, [ev]),
            Event::Key { key: k, pressed, repeat, modifiers: m, physical_key } => {
                let mut st = state();
                let mods = modifiers(&st) | m;
                if !pressed {
                    release_once(&mut st);
                }
                drop(st);
                send(&ctx, [Event::Key { key: k, physical_key, pressed, repeat, modifiers: mods }]);
            }
            _ => {}
        }
    }
}

fn touch(ctx: &Context, id: egui::TouchId, phase: TouchPhase, pos: Pos2, area: Rect) {
    let mut st = state();
    match phase {
        TouchPhase::Start => {
            st.touches.retain(|(t, _)| *t != id);
            st.touches.push((id, pos));
            let n = st.touches.len();
            match st.gesture.as_mut() {
                Some(g) => {
                    g.fingers = n;
                    g.max_fingers = g.max_fingers.max(n);
                    g.last_centroid = None;
                    g.last_spread = None;
                }
                None => {
                    st.gesture = Some(Gesture { start: Instant::now(), fingers: n, max_fingers: n, moved: 0.0, last_centroid: None, last_spread: None, drag_held: false });
                }
            }
        }
        TouchPhase::Move => {
            if let Some(t) = st.touches.iter_mut().find(|(t, _)| *t == id) {
                t.1 = pos;
            }
            let n = st.touches.len().max(1) as f32;
            let centroid = st.touches.iter().fold(Vec2::ZERO, |a, (_, p)| a + p.to_vec2()) / n;
            let centroid = centroid.to_pos2();
            let spread = if st.touches.len() >= 2 { Some(st.touches[0].1.distance(st.touches[1].1)) } else { None };
            let fingers = st.touches.len();
            let mods = modifiers(&st);
            let screen = st.screen;
            let mut out = Vec::new();
            let mut cursor = st.cursor;
            if let Some(g) = st.gesture.as_mut() {
                if let Some(last) = g.last_centroid {
                    let d = centroid - last;
                    g.moved += d.length();
                    if fingers == 1 {
                        cursor = if screen.is_positive() { screen.clamp(cursor + d * SPEED) } else { cursor + d * SPEED };
                        out.push(Event::PointerMoved(cursor));
                    } else if fingers == 2 {
                        out.push(Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: d * SPEED,
                            phase: TouchPhase::Move,
                            source: egui::MouseWheelSource::Unknown,
                            modifiers: mods,
                        });
                        if let (Some(a), Some(b)) = (g.last_spread, spread) {
                            if a > 1.0 && (b / a - 1.0).abs() > 0.01 {
                                out.push(Event::Zoom(b / a));
                            }
                        }
                    }
                }
                g.last_centroid = Some(centroid);
                g.last_spread = spread;
            }
            st.cursor = cursor;
            let _ = area;
            drop(st);
            if !out.is_empty() {
                send(ctx, out);
            }
        }
        TouchPhase::End | TouchPhase::Cancel => {
            st.touches.retain(|(t, _)| *t != id);
            if st.touches.is_empty() {
                if let Some(g) = st.gesture.take() {
                    let quick = g.start.elapsed() < TAP_TIME && g.moved < TAP_MOVE && phase == TouchPhase::End;
                    drop(st);
                    if quick {
                        match g.max_fingers {
                            1 => click(ctx, egui::PointerButton::Primary),
                            2 => click(ctx, egui::PointerButton::Secondary),
                            _ => {}
                        }
                    }
                }
            } else {
                let n = st.touches.len();
                if let Some(g) = st.gesture.as_mut() {
                    g.fingers = n;
                    g.last_centroid = None;
                    g.last_spread = None;
                }
            }
        }
    }
}
