//! craft-ios: the UIKit scene life cycle for winit's iOS backend.
//!
//! Apps built with the iOS 27 SDK must use scenes or they don't launch, and only apps built with
//! that SDK fill iPhone Duo's inner display. Info.plist declares the scenes; this module attaches
//! winit's windows to the app's scene when UIKit connects it (windows made before that wait), and
//! moves a window to the scene of another screen (display glasses) for `set_fullscreen`.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2::{msg_send, msg_send_id, sel};
use objc2_foundation::{CGFloat, CGRect, NSNotification, NSNotificationCenter, NSObject, NSString};
use objc2_ui_kit::{UIScreen, UIWindow};

use super::notification_center::create_observer;

thread_local! {
    /// The app's own window scene, once UIKit has connected it.
    static APP: RefCell<Option<Retained<NSObject>>> = const { RefCell::new(None) };
    /// Scenes of external displays (glasses), while connected.
    static EXTERNAL: RefCell<Vec<Retained<NSObject>>> = const { RefCell::new(Vec::new()) };
    /// Windows made before the app's scene connected.
    static WAITING: RefCell<Vec<Retained<UIWindow>>> = const { RefCell::new(Vec::new()) };
    static OBSERVERS: RefCell<Vec<Retained<NSObject>>> = const { RefCell::new(Vec::new()) };
    static HINGE_WATCHED: Cell<bool> = const { Cell::new(false) };
}

/// iPhone Duo's hinge (UIHingeInteraction, iOS 27.1): angle in radians, while reported.
static HINGE_KNOWN: AtomicBool = AtomicBool::new(false);
static HINGE_ANGLE: AtomicU64 = AtomicU64::new(0);

fn is_window_scene(scene: &NSObject) -> bool {
    let class = objc2::class!(UIWindowScene);
    unsafe { msg_send![scene, isKindOfClass: class] }
}

fn is_external(scene: &NSObject) -> bool {
    let session: Option<Retained<NSObject>> = unsafe { msg_send_id![scene, session] };
    let Some(session) = session else { return false };
    let role: Option<Retained<NSString>> = unsafe { msg_send_id![&*session, role] };
    role.is_some_and(|r| r.to_string().contains("ExternalDisplay"))
}

fn screen_of(scene: &NSObject) -> Retained<UIScreen> {
    unsafe { msg_send_id![scene, screen] }
}

fn bounds_of(scene: &NSObject) -> CGRect {
    let space: Retained<NSObject> = unsafe { msg_send_id![scene, coordinateSpace] };
    unsafe { msg_send![&*space, bounds] }
}

fn attach(window: &UIWindow, scene: &NSObject) {
    unsafe {
        let _: () = msg_send![window, setWindowScene: scene];
    }
    window.setFrame(bounds_of(scene));
}

/// The notification's object (a scene), as an NSObject.
fn scene_of(note: &NSNotification) -> Option<Retained<NSObject>> {
    #[allow(unused_unsafe)]
    let object = unsafe { note.object() }?;
    // SAFETY: UIScene notifications carry the scene, an NSObject subclass.
    Some(unsafe { Retained::cast::<NSObject>(object) })
}

fn on_connect(note: &NSNotification) {
    let Some(scene) = scene_of(note) else { return };
    if !is_window_scene(&scene) {
        return;
    }
    if is_external(&scene) {
        // Display glasses and monitors draw at 1x or 2x. A phone panel (iPhone Duo's outer
        // display, which can appear as a second display while the phone is open) draws at 3x
        // and is never treated as glasses.
        if screen_of(&scene).nativeScale() < 2.5 {
            EXTERNAL.with(|e| e.borrow_mut().push(scene));
        }
        return;
    }
    APP.with(|a| *a.borrow_mut() = Some(scene.clone()));
    for window in WAITING.with(|w| std::mem::take(&mut *w.borrow_mut())) {
        attach(&window, &scene);
        watch_hinge(&window);
        window.makeKeyAndVisible();
    }
}

fn responds(obj: *mut AnyObject, sel: Sel) -> bool {
    !obj.is_null() && unsafe { msg_send![obj, respondsToSelector: sel] }
}

/// Follow the hinge (iPhone Duo) through a UIHingeInteraction on the app's first window. Does
/// nothing where UIKit has no hinge API; every call is checked before it's made.
fn watch_hinge(window: &UIWindow) {
    if HINGE_WATCHED.with(|w| w.replace(true)) {
        return;
    }
    let Some(class) = AnyClass::get("UIHingeInteraction") else { return };
    let can_init: bool = unsafe { msg_send![class, instancesRespondToSelector: sel!(initWithUpdateHandler:)] };
    if !can_init {
        return;
    }
    let handler = RcBlock::new(|_interaction: *mut AnyObject, update: *mut AnyObject| {
        if !responds(update, sel!(hinge)) {
            return;
        }
        let hinge: *mut AnyObject = unsafe { msg_send![update, hinge] };
        if !responds(hinge, sel!(angle)) {
            // No hinge in this window's hierarchy (closed, or on another display).
            HINGE_KNOWN.store(false, Ordering::Relaxed);
            return;
        }
        let angle: CGFloat = unsafe { msg_send![hinge, angle] };
        HINGE_ANGLE.store((angle as f64).to_bits(), Ordering::Relaxed);
        HINGE_KNOWN.store(true, Ordering::Relaxed);
    });
    unsafe {
        let interaction: *mut AnyObject = msg_send![class, alloc];
        let interaction: *mut AnyObject = msg_send![interaction, initWithUpdateHandler: &*handler];
        if !interaction.is_null() {
            // Kept for the life of the app (the +1 from alloc/init is never released).
            let _: () = msg_send![window, addInteraction: interaction];
        }
    }
}

/// The hinge angle in radians, while the device reports one (iPhone Duo, iOS 27.1+).
pub(crate) fn hinge_angle() -> Option<f64> {
    HINGE_KNOWN.load(Ordering::Relaxed).then(|| f64::from_bits(HINGE_ANGLE.load(Ordering::Relaxed)))
}

/// Size in points of the app's own scene, once connected.
pub(crate) fn app_size() -> Option<(f64, f64)> {
    APP.with(|a| a.borrow().as_ref().map(|s| bounds_of(s)))
        .map(|b| (b.size.width as f64, b.size.height as f64))
}

fn on_disconnect(note: &NSNotification) {
    let Some(scene) = scene_of(note) else { return };
    EXTERNAL.with(|e| e.borrow_mut().retain(|s| !std::ptr::eq(&**s, &*scene)));
    APP.with(|a| {
        let mut a = a.borrow_mut();
        if a.as_ref().is_some_and(|s| std::ptr::eq(&**s, &*scene)) {
            *a = None;
        }
    });
}

/// Watch scenes connect and go. Call before `UIApplicationMain`.
pub(crate) fn install(center: &NSNotificationCenter) {
    let connect = create_observer(center, &NSString::from_str("UISceneWillConnectNotification"), on_connect);
    let disconnect =
        create_observer(center, &NSString::from_str("UISceneDidDisconnectNotification"), on_disconnect);
    OBSERVERS.with(|o| o.borrow_mut().extend([connect, disconnect]));
}

/// A new window joins the app's scene, or waits for it.
pub(crate) fn adopt(window: &Retained<UIWindow>) {
    match APP.with(|a| a.borrow().clone()) {
        Some(scene) => {
            attach(window, &scene);
            watch_hinge(window);
        },
        None => WAITING.with(|w| w.borrow_mut().push(window.clone())),
    }
}

/// Move a window to an external display's scene when `screen` is one; to any other screen means
/// back to the app's own scene (on iPhone Duo the app's screen need not be `mainScreen`). False
/// only before the app's scene has connected; the caller then falls back to `UIWindow.screen`.
pub(crate) fn move_to(window: &UIWindow, screen: &UIScreen) -> bool {
    let external = EXTERNAL.with(|e| e.borrow().iter().find(|s| &*screen_of(s) == screen).cloned());
    match external.or_else(|| APP.with(|a| a.borrow().clone())) {
        Some(scene) => {
            attach(window, &scene);
            window.makeKeyAndVisible();
            true
        },
        None => false,
    }
}

/// Whether `screen` is an external display (glasses) connected as its own scene.
pub(crate) fn is_external_screen(screen: &UIScreen) -> bool {
    EXTERNAL.with(|e| e.borrow().iter().any(|s| &*screen_of(s) == screen))
}

/// Screens of connected external-display scenes (`UIScreen.screens` may not list them).
pub(crate) fn external_screens() -> Vec<Retained<UIScreen>> {
    EXTERNAL.with(|e| e.borrow().iter().map(|s| screen_of(s)).collect())
}
