#![allow(deprecated, non_snake_case)]
//! iOS backing: `UIDocumentPickerViewController` for files and folders,
//! `UIAlertController` for message boxes, `UIActivityViewController` to share.
//!
//! Presentation happens eagerly (on the UI thread, at call time, like real
//! `rfd`); the returned futures only wait for the delegate / action callback.
//! Pending picks live in a global map keyed by delegate address (plain
//! `usize`, so the map stays `Send`), and the delegate is retained manually
//! with `into_raw` / `from_raw` because `setDelegate` is weak.
//!
//! Adapted from surfrpt1's PdfCraft iOS spike (MIT OR Apache-2.0).

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll, Waker};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, ClassType, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{NSArray, NSDate, NSMutableArray, NSObject, NSRunLoop, NSString, NSURL};
use objc2_ui_kit::{
    UIActivityViewController, UIAlertAction, UIAlertActionStyle, UIAlertController,
    UIAlertControllerStyle, UIApplication, UIDocumentPickerDelegate, UIDocumentPickerMode,
    UIDocumentPickerViewController, UIViewController, UIWindow,
};

use crate::MessageDialogResult;

// ------------------------------------------------------------ shared one-shot

struct Slot<T> {
    done: Option<T>,
    waker: Option<Waker>,
}

/// A one-shot value filled by a UIKit callback and awaited by the app.
pub(crate) struct OneShot<T> {
    slot: Arc<Mutex<Slot<T>>>,
}

impl<T> Clone for OneShot<T> {
    fn clone(&self) -> Self {
        Self { slot: self.slot.clone() }
    }
}

impl<T> OneShot<T> {
    fn new() -> Self {
        Self { slot: Arc::new(Mutex::new(Slot { done: None, waker: None })) }
    }

    fn ready(v: T) -> Self {
        let s = Self::new();
        s.set(v);
        s
    }

    fn set(&self, v: T) {
        let mut s = self.slot.lock().unwrap();
        s.done = Some(v);
        if let Some(w) = s.waker.take() {
            w.wake();
        }
    }
}

impl<T> Future for OneShot<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let mut s = self.slot.lock().unwrap();
        match s.done.take() {
            Some(v) => Poll::Ready(v),
            None => {
                s.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// Drive `f` to completion. On the main thread the run loop keeps turning
/// between polls so UIKit can show the panel and deliver the answer; off it
/// the thread just parks.
pub(crate) fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    if MainThreadMarker::new().is_some() {
        let mut cx = Context::from_waker(Waker::noop());
        let run_loop = NSRunLoop::currentRunLoop();
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
            let until = NSDate::dateWithTimeIntervalSinceNow(0.05);
            run_loop.runUntilDate(&until);
        }
    } else {
        struct Unpark(std::thread::Thread);
        impl std::task::Wake for Unpark {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
            std::thread::park_timeout(std::time::Duration::from_millis(100));
        }
    }
}

// ------------------------------------------------------------- document picker

struct Pick {
    out: OneShot<Vec<PathBuf>>,
    /// Hold security-scoped access on picked (Open-mode) URLs.
    hold_access: bool,
}

static PENDING: OnceLock<Mutex<HashMap<usize, Pick>>> = OnceLock::new();

fn pending() -> &'static Mutex<HashMap<usize, Pick>> {
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CraftIosPickerDelegate"]
    struct PickerDelegate;

    unsafe impl NSObjectProtocol for PickerDelegate {}

    unsafe impl UIDocumentPickerDelegate for PickerDelegate {
        #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
        fn documentPicker_didPickDocumentsAtURLs(
            &self,
            _controller: &UIDocumentPickerViewController,
            urls: &NSArray<NSURL>,
        ) {
            let key = self as *const Self as usize;
            let Some(pick) = pending().lock().unwrap().remove(&key) else { return };
            let mut paths = Vec::new();
            for i in 0..urls.len() {
                let url = urls.objectAtIndex(i);
                if pick.hold_access {
                    // Open-mode URLs live outside the sandbox: hold the access
                    // claim for the session so later reads and writes work.
                    let _ = unsafe { url.startAccessingSecurityScopedResource() };
                }
                if let Some(ns) = url.path() {
                    paths.push(PathBuf::from(ns.to_string()));
                }
            }
            // Balance the manual retain from presentation.
            unsafe { drop(Retained::<PickerDelegate>::from_raw(key as *mut PickerDelegate)) };
            pick.out.set(paths);
        }

        #[unsafe(method(documentPickerWasCancelled:))]
        fn documentPickerWasCancelled(&self, _controller: &UIDocumentPickerViewController) {
            let key = self as *const Self as usize;
            if let Some(pick) = pending().lock().unwrap().remove(&key) {
                unsafe { drop(Retained::<PickerDelegate>::from_raw(key as *mut PickerDelegate)) };
                pick.out.set(Vec::new());
            }
        }
    }
);

/// Extension → UTI for the picker. Unknown types fall back to `public.item`
/// (everything) so no file the app could read is ever hidden.
fn uti_for_ext(ext: &str) -> &'static str {
    match ext {
        "pdf" => "com.adobe.pdf",
        "png" => "public.png",
        "jpg" | "jpeg" => "public.jpeg",
        "gif" => "com.compuserve.gif",
        "tif" | "tiff" => "public.tiff",
        "bmp" => "com.microsoft.bmp",
        "heic" | "heif" => "public.heic",
        "webp" => "org.webmproject.webp",
        "svg" => "public.svg-image",
        "txt" | "text" => "public.plain-text",
        "html" | "htm" => "public.html",
        "csv" => "public.comma-separated-values-text",
        "rtf" => "public.rtf",
        "mp4" | "m4v" => "public.mpeg-4",
        "mov" => "com.apple.quicktime-movie",
        "mp3" => "public.mp3",
        "wav" => "com.microsoft.waveform-audio",
        "m4a" => "com.apple.m4a-audio",
        "aif" | "aiff" => "public.aiff-audio",
        "zip" => "public.zip-archive",
        "json" => "public.json",
        _ => "public.item",
    }
}

fn utis_for_filters(filters: &[String]) -> Vec<String> {
    let mut out: Vec<String> = filters.iter().map(|e| uti_for_ext(e).to_string()).collect();
    // One unknown extension means the picker must show everything.
    if out.is_empty() || out.iter().any(|u| u == "public.item") {
        return vec!["public.item".to_string()];
    }
    out.sort();
    out.dedup();
    out
}

/// The app's root view controller for presenting modal UI. If something is
/// already presented, present on top of it.
fn find_presenter() -> Option<Retained<UIViewController>> {
    let mtm = MainThreadMarker::new()?;
    let app = UIApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    let window: Retained<UIWindow> = match app.keyWindow() {
        Some(w) => w,
        None => {
            #[allow(deprecated)]
            let windows = app.windows();
            let mut found: Option<Retained<UIWindow>> = None;
            for i in 0..windows.len() {
                let w = windows.objectAtIndex(i);
                if w.isKeyWindow() {
                    found = Some(w);
                    break;
                }
                if found.is_none() {
                    found = Some(w);
                }
            }
            found?
        }
    };
    let mut vc = window.rootViewController()?;
    while let Some(next) = vc.presentedViewController() {
        vc = next;
    }
    Some(vc)
}

#[allow(deprecated)]
fn present_picker(doc_types: &[String], mode: UIDocumentPickerMode, multiple: bool, hold_access: bool) -> OneShot<Vec<PathBuf>> {
    let out = OneShot::new();
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("[rfd-ios] picker asked for off the main thread; nothing picked");
        return OneShot::ready(Vec::new());
    };
    let Some(root) = find_presenter() else {
        eprintln!("[rfd-ios] no window to present the picker on");
        return OneShot::ready(Vec::new());
    };
    let types = NSMutableArray::<NSString>::new();
    for t in doc_types {
        types.addObject(&NSString::from_str(t));
    }
    let picker = UIDocumentPickerViewController::initWithDocumentTypes_inMode(
        UIDocumentPickerViewController::alloc(mtm),
        &types,
        mode,
    );
    picker.setAllowsMultipleSelection(multiple);
    let delegate: Retained<PickerDelegate> = unsafe { msg_send![PickerDelegate::class(), new] };
    picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    let raw = Retained::into_raw(delegate) as usize;
    pending().lock().unwrap().insert(raw, Pick { out: out.clone(), hold_access });
    root.presentViewController_animated_completion(&picker, true, None);
    out
}

/// Files: Import mode (single tap picks; the system copies into the sandbox).
pub(crate) fn pick(multiple: bool, filters: Vec<String>) -> OneShot<Vec<PathBuf>> {
    present_picker(&utis_for_filters(&filters), UIDocumentPickerMode::Import, multiple, false)
}

/// Folders: Open mode, access held for the session.
pub(crate) fn folder(multiple: bool) -> OneShot<Vec<PathBuf>> {
    present_picker(&["public.folder".to_string()], UIDocumentPickerMode::Open, multiple, true)
}

// ---------------------------------------------------------------- message box

pub(crate) fn alert(
    title: String,
    description: String,
    choices: Vec<(String, MessageDialogResult)>,
) -> OneShot<Option<MessageDialogResult>> {
    let Some(mtm) = MainThreadMarker::new() else {
        return OneShot::ready(None);
    };
    let Some(root) = find_presenter() else {
        return OneShot::ready(None);
    };
    let out: OneShot<Option<MessageDialogResult>> = OneShot::new();
    let title = (!title.is_empty()).then(|| NSString::from_str(&title));
    let message = (!description.is_empty()).then(|| NSString::from_str(&description));
    let controller = UIAlertController::alertControllerWithTitle_message_preferredStyle(
        title.as_deref(),
        message.as_deref(),
        UIAlertControllerStyle::Alert,
        mtm,
    );
    let last = choices.len().saturating_sub(1);
    for (i, (label, result)) in choices.into_iter().enumerate() {
        let out = out.clone();
        let handler = RcBlock::new(move |_action: NonNull<UIAlertAction>| {
            out.set(Some(result.clone()));
        });
        let style = if i == last && i > 0 { UIAlertActionStyle::Cancel } else { UIAlertActionStyle::Default };
        let action = UIAlertAction::actionWithTitle_style_handler(
            Some(&NSString::from_str(&label)),
            style,
            Some(&handler),
            mtm,
        );
        controller.addAction(&action);
    }
    root.presentViewController_animated_completion(&controller, true, None);
    out
}

// ---------------------------------------------------------------- share sheet

/// Present the iOS share sheet for an already-written file. Must run on the
/// UI thread. On iPad the sheet is anchored to the root view.
pub(crate) fn share_file(path: &std::path::Path) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(root) = find_presenter() else {
        return false;
    };
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let items = NSMutableArray::<objc2::runtime::AnyObject>::new();
    items.addObject(&url);
    let sheet = unsafe {
        UIActivityViewController::initWithActivityItems_applicationActivities(
            UIActivityViewController::alloc(mtm),
            &items,
            None,
        )
    };
    if let Some(popover) = sheet.popoverPresentationController() {
        let Some(view) = root.view() else {
            return false;
        };
        popover.setSourceView(Some(&view));
        popover.setSourceRect(view.bounds());
    }
    root.presentViewController_animated_completion(&sheet, true, None);
    true
}
