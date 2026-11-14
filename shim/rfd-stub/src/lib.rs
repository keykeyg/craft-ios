//! iOS stand-in for the `rfd` 0.17 API surface used by the ArtCraft apps.
//!
//! Upstream `rfd` has no iOS backend. This crate replaces it through
//! `[patch.crates-io]` in iOS-only build trees (see `shim/apply.py`).
//! Based on surfrpt1's PdfCraft iOS spike (MIT OR Apache-2.0), extended with
//! the blocking `FileDialog`, `MessageDialog` and `AsyncMessageDialog` APIs
//! the other apps call.
//!
//! - Async pickers present eagerly in the (non-async) constructors, on the UI
//!   thread, like real `rfd`; the future only waits for the delegate.
//! - Blocking pickers and message boxes present, then spin the main run loop
//!   until the user answers. Off the main thread they resolve to "nothing".
//! - Other targets: everything resolves to "picked nothing" / the default
//!   answer, so the tree still checks on a desktop host.

use std::future::Future;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- file dialogs

/// Builder for an async file dialog. File-type filters are collected and, on
/// iOS, become the picker's document types.
#[derive(Debug, Clone, Default)]
pub struct AsyncFileDialog {
    filters: Vec<String>,
    file_name: Option<String>,
}

impl AsyncFileDialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_filter(mut self, _name: impl Into<String>, extensions: &[impl ToString]) -> Self {
        self.filters.extend(extensions.iter().map(|e| e.to_string().to_ascii_lowercase()));
        self
    }

    pub fn set_directory<P: AsRef<Path>>(self, _path: P) -> Self {
        self
    }

    pub fn set_file_name(mut self, file_name: impl Into<String>) -> Self {
        self.file_name = Some(file_name.into());
        self
    }

    pub fn set_title(self, _title: impl Into<String>) -> Self {
        self
    }

    pub fn set_can_create_directories(self, _can: bool) -> Self {
        self
    }

    pub fn set_parent<W: ?Sized>(self, _parent: &W) -> Self {
        self
    }

    pub fn pick_file(self) -> impl Future<Output = Option<FileHandle>> {
        let fut = platform::pick(false, self.filters);
        async move { fut.await.into_iter().next().map(FileHandle::from_path) }
    }

    pub fn pick_files(self) -> impl Future<Output = Option<Vec<FileHandle>>> {
        let fut = platform::pick(true, self.filters);
        async move { non_empty(fut.await).map(|v| v.into_iter().map(FileHandle::from_path).collect()) }
    }

    pub fn pick_folder(self) -> impl Future<Output = Option<FileHandle>> {
        let fut = platform::folder(false);
        async move { fut.await.into_iter().next().map(FileHandle::from_path) }
    }

    pub fn pick_folders(self) -> impl Future<Output = Option<Vec<FileHandle>>> {
        let fut = platform::folder(true);
        async move { non_empty(fut.await).map(|v| v.into_iter().map(FileHandle::from_path).collect()) }
    }

    pub fn pick_file_or_folder(self) -> impl Future<Output = Option<FileHandle>> {
        self.pick_file()
    }

    pub fn pick_files_or_folders(self) -> impl Future<Output = Option<Vec<FileHandle>>> {
        self.pick_files()
    }

    /// iOS: writes go to the app's Documents folder (visible in Files under
    /// "On My iPhone"), so saving never needs a second panel.
    pub fn save_file(self) -> impl Future<Output = Option<FileHandle>> {
        let path = save_target(self.file_name, &self.filters);
        async move { path.map(FileHandle::from_path) }
    }
}

/// Builder for a blocking file dialog.
#[derive(Debug, Clone, Default)]
pub struct FileDialog {
    inner: AsyncFileDialog,
}

impl FileDialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_filter(mut self, name: impl Into<String>, extensions: &[impl ToString]) -> Self {
        self.inner = self.inner.add_filter(name, extensions);
        self
    }

    pub fn set_directory<P: AsRef<Path>>(self, _path: P) -> Self {
        self
    }

    pub fn set_file_name(mut self, file_name: impl Into<String>) -> Self {
        self.inner = self.inner.set_file_name(file_name);
        self
    }

    pub fn set_title(self, _title: impl Into<String>) -> Self {
        self
    }

    pub fn set_can_create_directories(self, _can: bool) -> Self {
        self
    }

    pub fn set_parent<W: ?Sized>(self, _parent: &W) -> Self {
        self
    }

    pub fn pick_file(self) -> Option<PathBuf> {
        platform::block_on(platform::pick(false, self.inner.filters)).into_iter().next()
    }

    pub fn pick_files(self) -> Option<Vec<PathBuf>> {
        non_empty(platform::block_on(platform::pick(true, self.inner.filters)))
    }

    pub fn pick_folder(self) -> Option<PathBuf> {
        platform::block_on(platform::folder(false)).into_iter().next()
    }

    pub fn pick_folders(self) -> Option<Vec<PathBuf>> {
        non_empty(platform::block_on(platform::folder(true)))
    }

    pub fn save_file(self) -> Option<PathBuf> {
        save_target(self.inner.file_name, &self.inner.filters)
    }
}

fn non_empty(v: Vec<PathBuf>) -> Option<Vec<PathBuf>> {
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

/// Handle to a picked file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileHandle {
    path: PathBuf,
}

impl FileHandle {
    fn from_path(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn file_name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn read(&self) -> Vec<u8> {
        std::fs::read(&self.path).unwrap_or_default()
    }

    pub async fn write(&self, data: &[u8]) -> std::io::Result<()> {
        std::fs::write(&self.path, data)
    }

    pub fn inner(&self) -> &Path {
        &self.path
    }
}

impl From<PathBuf> for FileHandle {
    fn from(path: PathBuf) -> Self {
        Self { path }
    }
}

impl From<FileHandle> for PathBuf {
    fn from(h: FileHandle) -> Self {
        h.path
    }
}

/// Where a save goes: `<Documents>/<name>`, made unique so a save never
/// silently replaces another file.
fn save_target(name: Option<String>, filters: &[String]) -> Option<PathBuf> {
    let name = name.unwrap_or_else(|| match filters.first() {
        Some(ext) => format!("Untitled.{ext}"),
        None => "Untitled".to_string(),
    });
    let dir = documents_dir()?;
    let unique = uniquify(&dir, &name);
    Some(dir.join(unique))
}

/// Unique file name inside `dir`.
fn uniquify(dir: &Path, name: &str) -> String {
    if !dir.join(name).exists() {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 2..1000 {
        let candidate = format!("{stem} ({n}){ext}");
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    name.to_string()
}

/// App sandbox Documents directory (visible in Files with
/// `UIFileSharingEnabled`). Created on demand.
fn documents_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let docs = PathBuf::from(home).join("Documents");
    std::fs::create_dir_all(&docs).ok()?;
    Some(docs)
}

// ------------------------------------------------------------- message dialogs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MessageLevel {
    #[default]
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MessageButtons {
    #[default]
    Ok,
    OkCancel,
    YesNo,
    YesNoCancel,
    OkCustom(String),
    OkCancelCustom(String, String),
    YesNoCancelCustom(String, String, String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MessageDialogResult {
    Yes,
    No,
    #[default]
    Ok,
    Cancel,
    Custom(String),
}

impl MessageButtons {
    /// (label, result) pairs, in display order; the last one is the cancel-
    /// style answer used when there's no way to ask.
    fn choices(&self) -> Vec<(String, MessageDialogResult)> {
        use MessageDialogResult as R;
        match self {
            MessageButtons::Ok => vec![("OK".into(), R::Ok)],
            MessageButtons::OkCancel => vec![("OK".into(), R::Ok), ("Cancel".into(), R::Cancel)],
            MessageButtons::YesNo => vec![("Yes".into(), R::Yes), ("No".into(), R::No)],
            MessageButtons::YesNoCancel => {
                vec![("Yes".into(), R::Yes), ("No".into(), R::No), ("Cancel".into(), R::Cancel)]
            }
            MessageButtons::OkCustom(ok) => vec![(ok.clone(), R::Custom(ok.clone()))],
            MessageButtons::OkCancelCustom(ok, cancel) => {
                vec![(ok.clone(), R::Custom(ok.clone())), (cancel.clone(), R::Custom(cancel.clone()))]
            }
            MessageButtons::YesNoCancelCustom(y, n, c) => vec![
                (y.clone(), R::Custom(y.clone())),
                (n.clone(), R::Custom(n.clone())),
                (c.clone(), R::Custom(c.clone())),
            ],
        }
    }

    /// Answer when the dialog can't be shown: the least destructive choice.
    fn fallback(&self) -> MessageDialogResult {
        self.choices().pop().map(|c| c.1).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default)]
pub struct MessageDialog {
    level: MessageLevel,
    title: String,
    description: String,
    buttons: MessageButtons,
}

impl MessageDialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_level(mut self, level: MessageLevel) -> Self {
        self.level = level;
        self
    }

    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn set_description(mut self, text: impl Into<String>) -> Self {
        self.description = text.into();
        self
    }

    pub fn set_buttons(mut self, buttons: MessageButtons) -> Self {
        self.buttons = buttons;
        self
    }

    pub fn set_parent<W: ?Sized>(self, _parent: &W) -> Self {
        self
    }

    pub fn show(self) -> MessageDialogResult {
        log_message(&self);
        let fallback = self.buttons.fallback();
        platform::block_on(platform::alert(self.title, self.description, self.buttons.choices()))
            .unwrap_or(fallback)
    }
}

#[derive(Debug, Clone, Default)]
pub struct AsyncMessageDialog {
    inner: MessageDialog,
}

impl AsyncMessageDialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_level(mut self, level: MessageLevel) -> Self {
        self.inner = self.inner.set_level(level);
        self
    }

    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.inner = self.inner.set_title(title);
        self
    }

    pub fn set_description(mut self, text: impl Into<String>) -> Self {
        self.inner = self.inner.set_description(text);
        self
    }

    pub fn set_buttons(mut self, buttons: MessageButtons) -> Self {
        self.inner = self.inner.set_buttons(buttons);
        self
    }

    pub fn set_parent<W: ?Sized>(self, _parent: &W) -> Self {
        self
    }

    pub fn show(self) -> impl Future<Output = MessageDialogResult> {
        let d = self.inner;
        log_message(&d);
        let fallback = d.buttons.fallback();
        let fut = platform::alert(d.title, d.description, d.buttons.choices());
        async move { fut.await.unwrap_or(fallback) }
    }
}

fn log_message(d: &MessageDialog) {
    eprintln!("[rfd-ios] {:?}: {} — {}", d.level, d.title, d.description);
}

// -------------------------------------------------------------------- platform

#[cfg(target_os = "ios")]
mod ios_impl;

#[cfg(target_os = "ios")]
mod platform {
    pub(crate) use super::ios_impl::{alert, block_on, folder, pick};
}

#[cfg(not(target_os = "ios"))]
mod platform {
    use super::MessageDialogResult;
    use std::future::Future;
    use std::path::PathBuf;

    pub(crate) fn pick(_multiple: bool, _filters: Vec<String>) -> impl Future<Output = Vec<PathBuf>> {
        std::future::ready(Vec::new())
    }

    pub(crate) fn folder(_multiple: bool) -> impl Future<Output = Vec<PathBuf>> {
        std::future::ready(Vec::new())
    }

    pub(crate) fn alert(
        _title: String,
        _description: String,
        _choices: Vec<(String, MessageDialogResult)>,
    ) -> impl Future<Output = Option<MessageDialogResult>> {
        std::future::ready(None)
    }

    pub(crate) fn block_on<F: Future>(f: F) -> F::Output {
        let mut f = std::pin::pin!(f);
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        loop {
            if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
            std::thread::yield_now();
        }
    }
}

/// Present the iOS share sheet for an already-written file (call right after
/// saving, on the UI thread). Returns false when no window is available.
#[cfg(target_os = "ios")]
pub fn ios_share_file(path: &Path) -> bool {
    ios_impl::share_file(path)
}
