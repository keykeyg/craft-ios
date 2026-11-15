//! craft-ios: the UIKit scene life cycle for winit's iOS backend.
//!
//! Apps built with the iOS 27 SDK must use scenes or they don't launch, and only apps built with
//! that SDK fill iPhone Duo's inner display. Info.plist declares the scenes; this module attaches
//! winit's windows to the app's scene when UIKit connects it (windows made before that wait), and
//! moves a window to the scene of another screen (display glasses) for `set_fullscreen`.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::{msg_send, msg_send_id};
use objc2_foundation::{CGRect, NSNotification, NSNotificationCenter, NSObject, NSString};
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
}

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
        EXTERNAL.with(|e| e.borrow_mut().push(scene));
        return;
    }
    APP.with(|a| *a.borrow_mut() = Some(scene.clone()));
    for window in WAITING.with(|w| std::mem::take(&mut *w.borrow_mut())) {
        attach(&window, &scene);
        window.makeKeyAndVisible();
    }
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
        Some(scene) => attach(window, &scene),
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
