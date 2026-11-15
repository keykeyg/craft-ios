//! craft-ios glasses mode and DS mode.
//!
//! Glasses mode: plug display glasses (or any external screen) into the phone and the app's own
//! window moves to them, with the desktop layout; the phone becomes the controller.
//!
//! DS mode (iPhone Duo): fold the inner display like a laptop, or tap "split", and the canvas
//! takes one half while the controller takes the other, like a Nintendo DS. Unfold, or tap
//! "full screen", and the app has the whole display again.
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

type Frame = (f64, f64, f64, f64);

struct State {
    /// The app's window is on the external display.
    active: bool,
    /// DS mode: the inner display split between the canvas and the controller.
    ds: bool,
    /// The "split" / "full screen" buttons; cleared when the hinge moves.
    ds_forced: Option<bool>,
    /// The hinge was partly open at the last check (None: no hinge reported), and since when.
    hinge_partial: Option<bool>,
    hinge_since: Option<Instant>,
    /// The cursor shows in DS mode once the trackpad is used.
    cursor_shown: bool,
    /// The phone's display, and the controller's half of it in DS mode (points).
    full: Option<Frame>,
    controls: Frame,
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
    ds: false,
    ds_forced: None,
    hinge_partial: None,
    hinge_since: None,
    cursor_shown: false,
    full: None,
    controls: (0.0, 0.0, 0.0, 0.0),
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

/// Once, before the app starts. craft-update (the apps' updater) counts this copy as the newest
/// version, so it never offers the original project's releases: updates come from TestFlight.
#[allow(unsafe_code)]
pub(crate) fn startup() {
    // SAFETY: called at the top of run_native, before the app starts any threads.
    unsafe { std::env::set_var("ARTCRAFT_LAUNCHER_PRETEND_VERSION", "9999.0.0") };
}

// ------------------------------------------------------------------------- window placement

fn controller_id() -> ViewportId {
    ViewportId::from_hash_of(CONTROLLER)
}

/// The smallest side, in points, of a display big enough to split (iPhone Duo's inner display;
/// its outer display and other iPhones are smaller).
const SPLIT_MIN: f64 = 560.0;

/// DS mode halves: along the fold, canvas first (top, or left when the display is wide).
fn split((w, h): (f64, f64)) -> (Frame, Frame) {
    if h >= w {
        ((0.0, 0.0, w, h / 2.0), (0.0, h / 2.0, w, h / 2.0))
    } else {
        ((0.0, 0.0, w / 2.0, h), (w / 2.0, 0.0, w / 2.0, h))
    }
}

/// Called for every viewport each frame. The app's window goes to the glasses while they're
/// connected, takes half the inner display in DS mode, and the whole display otherwise; the
/// controller fills the phone (glasses) or the other half (DS mode).
pub(crate) fn sync_window(
    event_loop: &winit::event_loop::ActiveEventLoop,
    window: &winit::window::Window,
    ctx: &Context,
    viewport: ViewportId,
) {
    if viewport == ViewportId::ROOT {
        sync_root(event_loop, window, ctx);
    } else if viewport == controller_id() {
        let st = state();
        let frame = if st.active { st.full } else if st.ds { Some(st.controls) } else { None };
        drop(st);
        if let Some((x, y, w, h)) = frame {
            winit::platform::ios::craft_set_frame(window, x, y, w, h);
        }
    }
}

fn sync_root(event_loop: &winit::event_loop::ActiveEventLoop, window: &winit::window::Window, ctx: &Context) {
    // Polls for glasses and the hinge.
    ctx.request_repaint_after_for(Duration::from_millis(500), ViewportId::ROOT);

    let primary = event_loop.primary_monitor();
    // Only a display connected as an external-display scene (winit names it so): iPhone Duo's
    // inner and outer displays are both the phone, never the glasses.
    let external = event_loop
        .available_monitors()
        .find(|m| m.name().as_deref() == Some("External display"));
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

    let Some(size) = winit::platform::ios::craft_scene_size() else { return };
    st.full = Some((0.0, 0.0, size.0, size.1));
    // Half open (laptop or book posture): between about 20 and 160 degrees either way.
    let partial = winit::platform::ios::craft_hinge_angle().map(|a| a > 0.35 && a < 2.8);
    if partial != st.hinge_partial {
        st.hinge_partial = partial;
        st.hinge_since = Some(Instant::now());
        st.ds_forced = None; // the hinge moved: back to following it
    }
    // A posture held for most of a second, so opening the phone doesn't flash DS mode on.
    let settled = st.hinge_since.is_none_or(|t| t.elapsed() > Duration::from_millis(800));
    let follow = if settled { partial == Some(true) } else { st.ds };
    let ds = !st.active && size.0.min(size.1) >= SPLIT_MIN && st.ds_forced.unwrap_or(follow);
    if ds != st.ds {
        st.ds = ds;
        st.cursor_shown = false;
        st.queue.clear();
        ctx.request_repaint();
    }
    let (canvas, controls) = split(size);
    st.controls = controls;
    let active = st.active;
    drop(st);
    if !active {
        let (x, y, w, h) = if ds { canvas } else { (0.0, 0.0, size.0, size.1) };
        winit::platform::ios::craft_set_frame(window, x, y, w, h);
    }
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
    if st.active || st.ds {
        raw_input.events.append(&mut st.queue);
    }
}

/// After the app's UI: the cursor and the controller in glasses and DS modes, otherwise the
/// "split" button on a display big enough for DS mode.
pub(crate) fn after_ui(ctx: &Context) {
    let mut st = state();
    if !st.active && !st.ds {
        drop(st);
        split_button(ctx);
        return;
    }
    let screen = ctx.content_rect();
    st.screen = screen;
    if st.cursor == Pos2::ZERO || !screen.contains(st.cursor) {
        st.cursor = screen.clamp(if st.cursor == Pos2::ZERO { screen.center() } else { st.cursor });
    }
    let p = st.cursor;
    let pressed = st.gesture.as_ref().is_some_and(|g| g.drag_held) || st.drag_lock;
    let show_cursor = st.active || st.cursor_shown;
    drop(st);

    if show_cursor {
        let painter = ctx.layer_painter(LayerId::new(Order::Debug, Id::unique("loom-glasses-cursor")));
        let s = 1.25;
        let fill = if pressed { Color32::from_rgb(10, 132, 255) } else { Color32::WHITE };
        let at = |x: f32, y: f32| p + Vec2::new(x * s, y * s);
        // Arrow: a convex head plus a tail, each outlined so it reads on any background.
        painter.line_segment([at(5.5, 11.0), at(9.0, 19.0)], Stroke::new(5.5 * s, Color32::BLACK));
        painter.line_segment([at(5.5, 11.0), at(9.0, 19.0)], Stroke::new(3.0 * s, fill));
        painter.add(Shape::convex_polygon(vec![at(0.0, 0.0), at(0.0, 17.0), at(12.5, 12.5)], fill, Stroke::new(1.5, Color32::BLACK)));
    }

    // On iOS eframe draws extra viewports inside the main window; the controller needs a window
    // of its own on the phone, so embedding is off just for this one.
    let embed = ctx.embed_viewports();
    ctx.set_embed_viewports(false);
    ctx.show_viewport_deferred(
        controller_id(),
        ViewportBuilder::default().with_title("Loom controller"),
        controller_ui,
    );
    ctx.set_embed_viewports(embed);
}

/// A small "split" button in the corner of a big display: DS mode without folding.
fn split_button(ctx: &Context) {
    let screen = ctx.content_rect();
    if (screen.width().min(screen.height()) as f64) < SPLIT_MIN {
        return;
    }
    egui::Area::new(Id::unique("loom-split-button"))
        .order(Order::Foreground)
        .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-14.0, -14.0))
        .show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(72.0, 36.0), Sense::click());
            let fill = Color32::from_rgba_unmultiplied(44, 44, 46, if resp.is_pointer_button_down_on() { 250 } else { 200 });
            ui.painter().rect_filled(rect, CornerRadius::same(18), fill);
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, "split", FontId::proportional(15.0), Color32::WHITE);
            if resp.clicked() {
                state().ds_forced = Some(true);
                ctx.request_repaint();
            }
        });
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
    st.cursor_shown = true;
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
    let size = if rect.width() < 100.0 { 14.0 } else { 16.0 };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(size), Color32::WHITE);
    resp
}

/// The common commands, before the app's own row.
const CMDS: [(&str, fn(&Context)); 8] = [
    ("esc", |c| key(c, Key::Escape, Modifiers::NONE)),
    ("undo", |c| command(c, Key::Z)),
    ("redo", |c| key(c, Key::Z, Modifiers::COMMAND | Modifiers::MAC_CMD | Modifiers::SHIFT)),
    ("cut", |c| send(c, [Event::Cut])),
    ("copy", |c| send(c, [Event::Copy])),
    ("paste", paste),
    ("save", |c| command(c, Key::S)),
    ("delete", |c| key(c, Key::Delete, Modifiers::NONE)),
];

#[derive(Clone, Copy)]
enum Act {
    Cmd(usize),
    App(usize),
    Mod(usize),
    Keyboard,
    Full,
}

struct Item {
    label: String,
    act: Act,
    on: bool,
}

/// Groups of capsules, each group starting a new row. Returns the tapped one and the bottom edge.
fn grid(ui: &mut Ui, area: Rect, groups: &[Vec<Item>], per_row: usize, row_h: f32) -> (Option<Act>, f32) {
    let gap = 8.0;
    let w = (area.width() - (per_row as f32 - 1.0) * gap) / per_row as f32;
    let mut y = area.top();
    let mut tapped = None;
    for (g, items) in groups.iter().enumerate() {
        if items.is_empty() {
            continue;
        }
        if g > 0 {
            ui.painter().line_segment(
                [Pos2::new(area.left(), y - gap / 2.0), Pos2::new(area.right(), y - gap / 2.0)],
                Stroke::new(1.0, Color32::from_gray(50)),
            );
        }
        for (i, item) in items.iter().enumerate() {
            let (r, c) = (i / per_row, i % per_row);
            let rect = Rect::from_min_size(Pos2::new(area.left() + c as f32 * (w + gap), y + r as f32 * (row_h + gap)), Vec2::new(w, row_h));
            if capsule(ui, rect, &item.label, item.on).clicked() {
                tapped = Some(item.act);
            }
        }
        y += items.len().div_ceil(per_row) as f32 * (row_h + gap);
    }
    (tapped, y)
}

/// Sticky modifiers: tap once for the next click or key, tap again to lock, again to release.
fn tap_modifier(ctx: &Context, i: usize) {
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
            send(ctx, [Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: mods }]);
        }
    }
}

fn controller_ui(ui: &mut Ui, _class: ViewportClass) {
    let ctx = ui.ctx().clone();
    let full = ctx.content_rect();
    ui.painter().rect_filled(full, CornerRadius::ZERO, Color32::from_gray(18));
    let pad = 10.0;
    let inner = full.shrink(pad);

    // What's on the bar: common commands, the app's own row, then modifiers.
    let (ds, keyboard_on) = {
        let st = state();
        (st.ds, st.keyboard)
    };
    let cmds: Vec<Item> = CMDS.iter().enumerate().map(|(i, (l, _))| Item { label: (*l).into(), act: Act::Cmd(i), on: false }).collect();
    let app: Vec<Item> = crate::ios_bar::APP.iter().enumerate().map(|(i, (l, _))| Item { label: (*l).into(), act: Act::App(i), on: false }).collect();
    let mut mods: Vec<Item> = {
        let st = state();
        let sticky = [st.ctrl, st.alt, st.shift, st.cmd];
        ["⌃ ctrl", "⌥ opt", "⇧ shift", "⌘ cmd"]
            .iter()
            .zip(sticky)
            .enumerate()
            .map(|(i, (l, m))| Item {
                label: if m == Stick::Lock { format!("{l} •") } else { (*l).into() },
                act: Act::Mod(i),
                on: m != Stick::Off,
            })
            .chain([Item { label: "drag".into(), act: Act::Mod(4), on: st.drag_lock }])
            .collect()
    };
    if ds {
        mods.push(Item { label: "full screen".into(), act: Act::Full, on: false });
    }
    let keyboard = Item { label: if keyboard_on { "hide keyboard" } else { "keyboard" }.into(), act: Act::Keyboard, on: keyboard_on };

    // Half a display (DS mode): buttons on the left, trackpad on the right. A whole display:
    // the bar across the top, the trackpad below, the keyboard along the bottom.
    let two_columns = full.height() < 560.0;
    let (tapped, pad_rect) = if two_columns {
        let left = Rect::from_min_max(inner.min, Pos2::new(inner.left() + inner.width() * 0.56, inner.bottom()));
        let per_row = (((left.width() + 8.0) / 88.0).floor() as usize).clamp(3, 8);
        mods.push(keyboard);
        let groups = [cmds, app, mods];
        let rows: usize = groups.iter().map(|g| g.len().div_ceil(per_row)).sum();
        let row_h = ((left.height() - (rows as f32 - 1.0) * 8.0) / rows.max(1) as f32).clamp(30.0, ROW);
        let (tapped, _) = grid(ui, left, &groups, per_row, row_h);
        (tapped, Rect::from_min_max(Pos2::new(left.right() + 10.0, inner.top()), inner.max))
    } else {
        let per_row = if inner.width() > 600.0 { 8 } else { 4 };
        let (mut tapped, bottom) = grid(ui, inner, &[cmds, app, mods], per_row, ROW);
        let kb_rect = Rect::from_min_size(Pos2::new(inner.left(), inner.bottom() - ROW), Vec2::new(inner.width(), ROW));
        if capsule(ui, kb_rect, &keyboard.label, keyboard.on).clicked() {
            tapped = Some(Act::Keyboard);
        }
        (tapped, Rect::from_min_max(Pos2::new(inner.left(), bottom + 2.0), Pos2::new(inner.right(), kb_rect.top() - 10.0)))
    };
    match tapped {
        Some(Act::Cmd(i)) => (CMDS[i].1)(&ctx),
        Some(Act::App(i)) => run_entry(&ctx, crate::ios_bar::APP[i].1),
        Some(Act::Mod(i)) => tap_modifier(&ctx, i),
        Some(Act::Keyboard) => {
            let mut st = state();
            st.keyboard = !st.keyboard;
        }
        Some(Act::Full) => {
            state().ds_forced = Some(false);
            ctx.request_repaint_of(ViewportId::ROOT);
        }
        None => {}
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
            let mut shown = st.cursor_shown;
            if let Some(g) = st.gesture.as_mut() {
                if let Some(last) = g.last_centroid {
                    let d = centroid - last;
                    g.moved += d.length();
                    if fingers == 1 {
                        shown = true;
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
            st.cursor_shown = shown;
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
